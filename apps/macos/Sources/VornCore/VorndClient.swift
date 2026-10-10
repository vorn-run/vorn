import Foundation

/// Something vornd pushed, or the connection dropping.
public enum VorndEvent: Sendable {
    case notification(method: String, params: JSONValue)
    case disconnected(String)
}

public struct VorndRPCError: Error, Sendable, Equatable, CustomStringConvertible {
    public var code: Int
    public var message: String

    public init(code: Int, message: String) {
        self.code = code
        self.message = message
    }

    public var description: String { message }
}

public enum VorndClientError: Error, Sendable, Equatable, CustomStringConvertible {
    case notConnected
    case disconnected(String)
    case refusedReadOnly(String)
    case authFailed(String)
    case badReply

    public var description: String {
        switch self {
        case .notConnected: return "not connected to vornd"
        case .disconnected(let why): return "vornd disconnected: \(why)"
        case .refusedReadOnly(let m): return "\(m) writes, and this connection is read-only"
        case .authFailed(let why): return "vornd refused the token: \(why)"
        case .badReply: return "vornd sent a reply that could not be read"
        }
    }
}

/// JSON-RPC over vornd's WebSocket: authenticates, matches replies to calls, fans pushes out.
public actor VorndClient {
    public enum Access: Sendable { case readWrite, readOnly }

    public enum State: Sendable, Equatable {
        case idle, connecting, connected
        case disconnected(String)
    }

    public let endpoint: VorndEndpoint
    public let access: Access
    public private(set) var state: State = .idle

    private let session: URLSession
    private var task: URLSessionWebSocketTask?
    private var nextId = 1
    private var pending: [Int: CheckedContinuation<JSONValue, Error>] = [:]
    private var subscribers: [UUID: AsyncStream<VorndEvent>.Continuation] = [:]
    private var receiving: Task<Void, Never>?

    public init(endpoint: VorndEndpoint, access: Access = .readWrite) {
        self.endpoint = endpoint
        self.access = access
        self.session = URLSession(configuration: .ephemeral)
    }

    /// Calls a read-only connection may make: auth, subscriptions, and list/get queries.
    public static func isReadOnly(_ method: String) -> Bool {
        if ["auth:authenticate", "subscribe:set"].contains(method) { return true }
        guard let verb = method.split(separator: ":", maxSplits: 1).last, method.contains(":") else { return false }
        return verb.hasPrefix("list") || verb.hasPrefix("get")
    }

    public func connect() async throws {
        if state == .connected { return }
        state = .connecting
        let task = session.webSocketTask(with: endpoint.url)
        task.maximumMessageSize = 64 << 20
        self.task = task
        task.resume()
        receiving = Task { [weak self] in await self?.receiveLoop(task) }
        do {
            _ = try await send("auth:authenticate", params: .object(["token": .string(endpoint.token)]))
        } catch {
            drop(task, reason: "\(error)")
            throw VorndClientError.authFailed("\(error)")
        }
        state = .connected
    }

    public func close() {
        guard let task else { return }
        task.cancel(with: .normalClosure, reason: nil)
        drop(task, reason: "closed")
    }

    /// Calls a method and returns its raw result.
    @discardableResult
    public func call(_ method: String, _ params: JSONValue? = nil) async throws -> JSONValue {
        if access == .readOnly && !Self.isReadOnly(method) {
            throw VorndClientError.refusedReadOnly(method)
        }
        guard state == .connected else { throw VorndClientError.notConnected }
        return try await send(method, params: params)
    }

    /// Calls a method and decodes its result.
    public func call<T: Decodable & Sendable>(_ method: String, _ params: JSONValue? = nil, as type: T.Type) async throws -> T {
        let raw = try await call(method, params)
        do {
            return try raw.decode(T.self)
        } catch {
            throw VorndClientError.badReply
        }
    }

    /// Narrows what vornd pushes to these topics (exact, or a prefix ending in `*`).
    public func subscribe(topics: [String]) async throws {
        try await call("subscribe:set", .object(["topics": .array(topics.map(JSONValue.string))]))
    }

    /// A stream of pushes and disconnects; ends when the caller stops iterating.
    public func events() -> AsyncStream<VorndEvent> {
        let (stream, continuation) = AsyncStream<VorndEvent>.makeStream(bufferingPolicy: .bufferingNewest(512))
        let id = UUID()
        subscribers[id] = continuation
        continuation.onTermination = { [weak self] _ in
            Task { await self?.unsubscribe(id) }
        }
        return stream
    }

    private func unsubscribe(_ id: UUID) {
        subscribers[id] = nil
    }

    private func send(_ method: String, params: JSONValue?) async throws -> JSONValue {
        guard let task else { throw VorndClientError.notConnected }
        let id = nextId
        nextId += 1
        var message: [String: JSONValue] = ["jsonrpc": .string("2.0"), "id": .number(Double(id)), "method": .string(method)]
        if let params { message["params"] = params }
        let data = try JSONEncoder().encode(JSONValue.object(message))
        return try await withCheckedThrowingContinuation { continuation in
            pending[id] = continuation
            Task {
                do {
                    try await task.send(.string(String(decoding: data, as: UTF8.self)))
                } catch {
                    self.fail(id, error)
                }
            }
        }
    }

    private func fail(_ id: Int, _ error: Error) {
        pending.removeValue(forKey: id)?.resume(throwing: error)
    }

    private func receiveLoop(_ task: URLSessionWebSocketTask) async {
        while true {
            let message: URLSessionWebSocketTask.Message
            do {
                message = try await task.receive()
            } catch {
                drop(task, reason: error.localizedDescription)
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

    private func handle(_ data: Data) {
        guard let msg = try? JSONDecoder().decode([String: JSONValue].self, from: data) else { return }
        if let id = msg["id"]?.numberValue.map(Int.init), let continuation = pending.removeValue(forKey: id) {
            if let error = msg["error"], !error.isNull {
                let code = error["code"]?.numberValue.map(Int.init) ?? 0
                let text = error["message"]?.stringValue ?? "error"
                continuation.resume(throwing: VorndRPCError(code: code, message: text))
            } else {
                continuation.resume(returning: msg["result"] ?? .null)
            }
            return
        }
        if let method = msg["method"]?.stringValue {
            broadcast(.notification(method: method, params: msg["params"] ?? .null))
        }
    }

    private func drop(_ dropped: URLSessionWebSocketTask, reason: String) {
        guard task === dropped else { return }
        task = nil
        receiving?.cancel()
        receiving = nil
        state = .disconnected(reason)
        let waiting = pending
        pending.removeAll()
        for (_, c) in waiting { c.resume(throwing: VorndClientError.disconnected(reason)) }
        broadcast(.disconnected(reason))
    }

    private func broadcast(_ event: VorndEvent) {
        for c in subscribers.values { c.yield(event) }
    }
}
