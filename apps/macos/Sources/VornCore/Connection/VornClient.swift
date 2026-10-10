import Foundation

/// A JSON-RPC connection to vornd over its `/ws` WebSocket.
///
/// It authenticates with the bearer credential on the upgrade, reconnects on
/// its own, and keeps one topic subscription that is the union of every
/// caller's, so each feature can listen without silencing the others.
public actor VornClient {
    public enum State: Sendable, Equatable {
        case idle
        case connecting
        case connected
        case disconnected(String)
    }

    /// Methods a read-only client may call; anything else is refused before it is sent.
    public static let readMethods: Set<String> = [
        "task:list", "task:get", "project:list", "config:load", "agent:detectInstalled",
    ]

    public nonisolated let endpoint: VornEndpoint
    public nonisolated let readOnly: Bool

    private let session: URLSession
    private var socket: URLSessionWebSocketTask?
    private var receiver: Task<Void, Never>?
    private var nextId = 1
    private var pending: [Int: CheckedContinuation<JSONValue, Error>] = [:]
    private var listeners: [UUID: (topic: String, continuation: AsyncStream<JSONValue>.Continuation)] = [:]
    private var stateListeners: [UUID: AsyncStream<State>.Continuation] = [:]
    private var reconnecting: Task<Void, Never>?
    private var closedByUser = false
    public private(set) var state: State = .idle

    public init(endpoint: VornEndpoint, readOnly: Bool = false, session: URLSession = URLSession(configuration: .ephemeral)) {
        self.endpoint = endpoint
        self.readOnly = readOnly
        self.session = session
    }

    // MARK: Lifecycle

    /// Opens the socket and proves it is admitted with one call.
    public func connect() async throws {
        closedByUser = false
        if state == .connected { return }
        try await open()
    }

    public func close() {
        closedByUser = true
        reconnecting?.cancel()
        reconnecting = nil
        teardown(reason: "closed")
        for (_, l) in listeners { l.continuation.finish() }
        listeners.removeAll()
        for (_, c) in stateListeners { c.finish() }
        stateListeners.removeAll()
    }

    private func open() async throws {
        setState(.connecting)
        var request = URLRequest(url: endpoint.socketURL)
        request.setValue("Bearer \(endpoint.token)", forHTTPHeaderField: "Authorization")
        let task = session.webSocketTask(with: request)
        task.maximumMessageSize = 128 << 20
        socket = task
        task.resume()
        receiver = Task { [weak self] in await self?.receiveLoop(task) }
        do {
            try await sendSubscription()
            setState(.connected)
        } catch {
            teardown(reason: error.localizedDescription)
            throw error
        }
    }

    private func teardown(reason: String) {
        receiver?.cancel()
        receiver = nil
        socket?.cancel(with: .normalClosure, reason: nil)
        socket = nil
        let waiting = pending
        pending.removeAll()
        for (_, c) in waiting { c.resume(throwing: VornError.disconnected) }
        setState(.disconnected(reason))
    }

    private func receiveLoop(_ task: URLSessionWebSocketTask) async {
        while !Task.isCancelled {
            let message: URLSessionWebSocketTask.Message
            do {
                message = try await task.receive()
            } catch {
                lost(task, reason: error.localizedDescription)
                return
            }
            let data: Data
            switch message {
            case .string(let s): data = Data(s.utf8)
            case .data(let d): data = d
            @unknown default: continue
            }
            handle(data)
        }
    }

    private func lost(_ task: URLSessionWebSocketTask, reason: String) {
        guard socket === task else { return }
        teardown(reason: reason)
        scheduleReconnect()
    }

    private func scheduleReconnect() {
        guard !closedByUser, reconnecting == nil else { return }
        reconnecting = Task { [weak self] in
            var delay: UInt64 = 250_000_000
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: delay)
                guard let self else { return }
                if await self.tryReopen() { return }
                delay = min(delay * 2, 5_000_000_000)
            }
        }
    }

    private func tryReopen() async -> Bool {
        if closedByUser { reconnecting = nil; return true }
        do {
            try await open()
            reconnecting = nil
            return true
        } catch {
            return false
        }
    }

    // MARK: Frames

    private func handle(_ data: Data) {
        guard let frame = try? JSONDecoder().decode(JSONValue.self, from: data) else { return }
        if case .number(let n) = frame["id"] ?? .null, frame["method"] == nil {
            let id = Int(n)
            guard let continuation = pending.removeValue(forKey: id) else { return }
            if let error = frame["error"], case .object = error {
                let code: Int
                if case .number(let c) = error["code"] ?? .null { code = Int(c) } else { code = 0 }
                continuation.resume(throwing: VornError.rpc(code: code, message: error["message"]?.stringValue ?? "error"))
            } else {
                continuation.resume(returning: frame["result"] ?? .null)
            }
            return
        }
        guard let method = frame["method"]?.stringValue else { return }
        let params = frame["params"] ?? .null
        for (_, l) in listeners where l.topic == method {
            l.continuation.yield(params)
        }
    }

    // MARK: Calls

    /// Calls `method` and returns its raw result.
    @discardableResult
    public func call(_ method: String, _ params: JSONValue? = nil) async throws -> JSONValue {
        if readOnly && !Self.readMethods.contains(method) {
            throw VornError.refused("\(method) is a write, and this connection is read-only")
        }
        return try await send(method, params)
    }

    /// Calls `method` with encodable params and decodes the result.
    public func call<P: Encodable, R: Decodable>(_ method: String, _ params: P, as: R.Type = R.self) async throws -> R {
        let result = try await call(method, try JSONValue.from(params))
        do {
            return try result.decode(R.self)
        } catch {
            throw VornError.badResponse("\(method): \(error)")
        }
    }

    private func send(_ method: String, _ params: JSONValue?) async throws -> JSONValue {
        guard let socket else { throw VornError.notConnected }
        let id = nextId
        nextId += 1
        var frame: [String: JSONValue] = ["jsonrpc": .string("2.0"), "id": .number(Double(id)), "method": .string(method)]
        if let params { frame["params"] = params }
        let text = String(decoding: try JSONEncoder().encode(JSONValue.object(frame)), as: UTF8.self)
        return try await withCheckedThrowingContinuation { continuation in
            pending[id] = continuation
            socket.send(.string(text)) { [weak self] error in
                guard let error else { return }
                Task { await self?.failSend(id, error) }
            }
        }
    }

    private func failSend(_ id: Int, _ error: Error) {
        pending.removeValue(forKey: id)?.resume(throwing: error)
    }

    // MARK: Subscriptions

    /// Every notification named `topic`, for as long as the stream is held.
    public func notifications(_ topic: String) -> AsyncStream<JSONValue> {
        let id = UUID()
        let (stream, continuation) = AsyncStream<JSONValue>.makeStream(bufferingPolicy: .bufferingNewest(16))
        listeners[id] = (topic, continuation)
        continuation.onTermination = { [weak self] _ in
            Task { await self?.removeListener(id) }
        }
        Task { try? await self.sendSubscription() }
        return stream
    }

    /// The connection's state, starting with the current one.
    public func stateUpdates() -> AsyncStream<State> {
        let id = UUID()
        let (stream, continuation) = AsyncStream<State>.makeStream(bufferingPolicy: .bufferingNewest(4))
        continuation.yield(state)
        stateListeners[id] = continuation
        continuation.onTermination = { [weak self] _ in
            Task { await self?.removeStateListener(id) }
        }
        return stream
    }

    private func removeListener(_ id: UUID) {
        guard listeners.removeValue(forKey: id) != nil else { return }
        Task { try? await self.sendSubscription() }
    }

    private func removeStateListener(_ id: UUID) {
        stateListeners.removeValue(forKey: id)
    }

    private func setState(_ new: State) {
        guard new != state else { return }
        state = new
        for (_, c) in stateListeners { c.yield(new) }
    }

    /// An empty list asks vornd for everything, so no listeners names a topic nothing sends.
    private func sendSubscription() async throws {
        guard socket != nil else { return }
        var topics = Array(Set(listeners.values.map(\.topic))).sorted()
        if topics.isEmpty { topics = ["client:none"] }
        _ = try await send("subscribe:set", .object(["topics": .array(topics.map(JSONValue.string))]))
    }
}
