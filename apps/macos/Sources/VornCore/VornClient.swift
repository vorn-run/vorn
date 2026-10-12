import Foundation

/// Something vornd pushed, or the connection dropping.
public enum VornEvent: Sendable {
    case notification(method: String, params: JSONValue)
    case disconnected(String)
}

/// A JSON-RPC connection to vornd over its `/ws` WebSocket, shared by every feature.
///
/// It authenticates with the bearer credential on the upgrade, reconnects on
/// its own (re-reading the endpoint files when it was made from a data
/// directory), and keeps one topic subscription that is the union of every
/// caller's, so each feature can listen without silencing the others.
public actor VornClient {
    public enum State: Sendable, Equatable {
        case idle
        case connecting
        case connected
        case disconnected(String)
    }

    /// Writes-free methods a read-only client may call besides `list*`/`get*` queries.
    public static let readMethods: Set<String> = [
        "auth:authenticate", "subscribe:set", "config:load", "agent:detectInstalled", "git:isGitRepo",
        "sessions:restored", "git:diffStat", "git:worktreeDirty",
        "core:status", "credential:safeStorageAvailable", "worktree:inventory", "git:diffFull", "file:readContent", "file:stamp", "ide:detect",
    ]

    /// Whether a read-only connection may send `method`.
    public static func isReadOnly(_ method: String) -> Bool {
        if readMethods.contains(method) { return true }
        guard let colon = method.firstIndex(of: ":") else { return false }
        let verb = method[method.index(after: colon)...]
        return verb.hasPrefix("list") || verb.hasPrefix("get")
    }

    public nonisolated let dataDir: URL
    public nonisolated let readOnly: Bool
    /// The endpoint in use; re-read from `dataDir` on each connect unless fixed.
    public private(set) var endpoint: VornEndpoint?

    private struct Listener {
        var topics: [String]
        var wantsDisconnects: Bool
        var continuation: AsyncStream<VornEvent>.Continuation
    }

    private let fixedEndpoint: VornEndpoint?
    private let session: URLSession
    private var socket: URLSessionWebSocketTask?
    private var receiver: Task<Void, Never>?
    private var nextId = 1
    private var pending: [Int: CheckedContinuation<JSONValue, Error>] = [:]
    private var listeners: [UUID: Listener] = [:]
    private var stateListeners: [UUID: AsyncStream<State>.Continuation] = [:]
    private var reconnecting: Task<Void, Never>?
    private var closedByUser = false
    public private(set) var state: State = .idle

    /// A client for one fixed endpoint.
    public init(endpoint: VornEndpoint, readOnly: Bool = false, session: URLSession = URLSession(configuration: .ephemeral)) {
        dataDir = endpoint.dataDir
        fixedEndpoint = endpoint
        self.endpoint = endpoint
        self.readOnly = readOnly
        self.session = session
    }

    /// A client for whichever vornd serves `dataDir`, found again on every reconnect.
    public init(dataDir: URL = VornEndpoint.defaultDataDir(), readOnly: Bool = false) {
        self.dataDir = dataDir
        fixedEndpoint = nil
        self.readOnly = readOnly
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 30
        session = URLSession(configuration: config)
    }

    // MARK: Lifecycle

    /// Opens the socket and proves it is admitted with one call.
    public func connect() async throws {
        closedByUser = false
        if state == .connected { return }
        try await open()
    }

    /// Connects, and keeps trying in the background until it does or `close()`.
    public func start() async {
        do {
            try await connect()
        } catch {
            scheduleReconnect()
        }
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
        let endpoint: VornEndpoint
        do {
            endpoint = try fixedEndpoint ?? VornEndpoint.discover(dataDir: dataDir)
        } catch {
            setState(.disconnected(error.localizedDescription))
            throw error
        }
        self.endpoint = endpoint
        var request = URLRequest(url: endpoint.webSocketURL)
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
        if closedByUser || state == .connected { reconnecting = nil; return true }
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
        if let n = frame["id"]?.numberValue, frame["method"] == nil {
            guard let continuation = pending.removeValue(forKey: Int(n)) else { return }
            if let error = frame["error"], case .object = error {
                let code = error["code"]?.numberValue.map { Int($0) } ?? 0
                continuation.resume(throwing: VornError.rpc(code: code, message: error["message"]?.stringValue ?? "error"))
            } else {
                continuation.resume(returning: frame["result"] ?? .null)
            }
            return
        }
        guard let method = frame["method"]?.stringValue else { return }
        let event = VornEvent.notification(method: method, params: frame["params"] ?? .null)
        for (_, l) in listeners where l.topics.contains(where: { Self.topic($0, matches: method) }) {
            l.continuation.yield(event)
        }
    }

    /// A topic names one method, or every method it prefixes when it ends in `*`.
    static func topic(_ topic: String, matches method: String) -> Bool {
        topic.hasSuffix("*") ? method.hasPrefix(topic.dropLast()) : topic == method
    }

    // MARK: Calls

    /// Calls `method` and returns its raw result.
    @discardableResult
    public func call(_ method: String, _ params: JSONValue? = nil) async throws -> JSONValue {
        if readOnly && !Self.isReadOnly(method) {
            throw VornError.refused("\(method) is a write, and this connection is read-only")
        }
        return try await send(method, params)
    }

    /// Calls `method` and decodes the result.
    public func call<R: Decodable>(_ method: String, _ params: JSONValue? = nil, as: R.Type) async throws -> R {
        try decode(method, try await call(method, params))
    }

    /// Calls `method` with encodable params and decodes the result.
    public func call<P: Encodable, R: Decodable>(_ method: String, _ params: P, as: R.Type = R.self) async throws -> R {
        let raw: JSONValue? = try JSONValue.from(params)
        return try decode(method, try await call(method, raw))
    }

    /// Sends `method` as a notification: no id, no reply.
    public func notify(_ method: String, _ params: JSONValue) async throws {
        if readOnly { throw VornError.refused("\(method) is a write, and this connection is read-only") }
        guard let socket else { throw VornError.notConnected }
        let frame: [String: JSONValue] = ["jsonrpc": "2.0", "method": .string(method), "params": params]
        let text = String(decoding: try JSONEncoder().encode(JSONValue.object(frame)), as: UTF8.self)
        try await socket.send(.string(text))
    }

    private func decode<R: Decodable>(_ method: String, _ result: JSONValue) throws -> R {
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
        var frame: [String: JSONValue] = ["jsonrpc": "2.0", "id": .number(Double(id)), "method": .string(method)]
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
        let events = listen([topic], disconnects: false, buffer: 16)
        return AsyncStream { continuation in
            let task = Task {
                for await case .notification(_, let params) in events { continuation.yield(params) }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    /// The notifications `topics` name, and each disconnect, for as long as the stream is held.
    public func events(_ topics: [String]) -> AsyncStream<VornEvent> {
        listen(topics, disconnects: true, buffer: 512)
    }

    private func listen(_ topics: [String], disconnects: Bool, buffer: Int) -> AsyncStream<VornEvent> {
        let id = UUID()
        let (stream, continuation) = AsyncStream<VornEvent>.makeStream(bufferingPolicy: .bufferingNewest(buffer))
        listeners[id] = Listener(topics: topics, wantsDisconnects: disconnects, continuation: continuation)
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
        if case .disconnected(let reason) = new {
            for (_, l) in listeners where l.wantsDisconnects { l.continuation.yield(.disconnected(reason)) }
        }
    }

    /// An empty list asks vornd for everything, so no listeners names a topic nothing sends.
    private func sendSubscription() async throws {
        guard socket != nil else { return }
        var topics = Array(Set(listeners.values.flatMap(\.topics))).sorted()
        if topics.isEmpty { topics = ["client:none"] }
        _ = try await send("subscribe:set", .object([
            "topics": .array(topics.map(JSONValue.string)), "terminalBytes": false,
        ]))
    }
}
