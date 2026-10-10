import Foundation

/// A JSON-RPC error from vornd, or one raised on this side of the socket.
public struct RPCError: Error, LocalizedError, Decodable, Sendable, Equatable {
    public var code: Int
    public var message: String

    public init(code: Int, message: String) {
        self.code = code
        self.message = message
    }

    public var errorDescription: String? { message }

    public static let disconnected = RPCError(code: -32000, message: "Disconnected from vornd")
    public static func readOnly(_ method: String) -> RPCError {
        RPCError(code: -32001, message: "\(method) writes, and this connection is read-only")
    }
}

/// A message vornd sent without being asked.
public struct RPCNotification: Sendable {
    public var method: String
    public var params: JSONValue
}

/// One authenticated JSON-RPC connection to vornd's `/ws`. It does not
/// reconnect: when the socket drops, `notifications` finishes and the owner
/// makes a new one.
public actor VornConnection {
    public nonisolated let endpoint: VornEndpoint
    public nonisolated let readOnly: Bool
    public nonisolated let notifications: AsyncStream<RPCNotification>

    private let sink: AsyncStream<RPCNotification>.Continuation
    private let session: URLSession
    private var task: URLSessionWebSocketTask?
    private var nextID = 1
    private var pending: [Int: CheckedContinuation<JSONValue, Error>] = [:]
    private var closed = false

    /// The methods a read-only connection may call.
    public static let readMethods: Set<String> = [
        "auth:authenticate", "subscribe:set", "config:load", "project:list",
        "terminal:listActive", "git:listWorktrees", "git:isGitRepo",
        "agent:detectInstalled", "sessions:getRecent", "git:listBranches",
    ]

    public init(endpoint: VornEndpoint, readOnly: Bool = false) {
        self.endpoint = endpoint
        self.readOnly = readOnly
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 30
        session = URLSession(configuration: config)
        (notifications, sink) = AsyncStream.makeStream(of: RPCNotification.self)
    }

    /// Opens the socket, presenting the endpoint's token on the upgrade.
    public func connect() async throws {
        var request = URLRequest(url: endpoint.webSocketURL)
        request.setValue("Bearer \(endpoint.token)", forHTTPHeaderField: "Authorization")
        let task = session.webSocketTask(with: request)
        task.maximumMessageSize = 64 << 20
        self.task = task
        task.resume()
        Task { await self.receiveLoop(task) }
        // The credential on the upgrade admits the socket; a ping confirms it was accepted.
        try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Void, Error>) in
            task.sendPing { error in
                if let error { cont.resume(throwing: error) } else { cont.resume() }
            }
        }
    }

    public func close() {
        task?.cancel(with: .goingAway, reason: nil)
        finish()
    }

    /// Calls `method` and returns its raw result.
    @discardableResult
    public func call(_ method: String, _ params: JSONValue? = nil) async throws -> JSONValue {
        if readOnly && !Self.readMethods.contains(method) { throw RPCError.readOnly(method) }
        guard let task, !closed else { throw RPCError.disconnected }
        let id = nextID
        nextID += 1
        var message: [String: JSONValue] = ["jsonrpc": "2.0", "id": .number(Double(id)), "method": .string(method)]
        if let params { message["params"] = params }
        let text = String(decoding: try JSONEncoder().encode(JSONValue.object(message)), as: UTF8.self)
        return try await withCheckedThrowingContinuation { cont in
            pending[id] = cont
            task.send(.string(text)) { error in
                guard let error else { return }
                Task { await self.fail(id, error) }
            }
        }
    }

    /// Calls `method` with encodable params and decodes its result as `T`.
    public func call<T: Decodable, P: Encodable>(_ method: String, params: P, as type: T.Type) async throws -> T {
        try await call(method, JSONValue.from(params)).decode(T.self)
    }

    /// Calls a method that takes no params and decodes its result as `T`.
    public func call<T: Decodable>(_ method: String, as type: T.Type) async throws -> T {
        try await call(method, nil).decode(T.self)
    }

    private func fail(_ id: Int, _ error: Error) {
        pending.removeValue(forKey: id)?.resume(throwing: error)
    }

    private func receiveLoop(_ task: URLSessionWebSocketTask) async {
        while !closed {
            do {
                switch try await task.receive() {
                case .string(let text): handle(Data(text.utf8))
                case .data(let data): handle(data)
                @unknown default: break
                }
            } catch {
                break
            }
        }
        finish()
    }

    private struct Envelope: Decodable {
        var id: Int?
        var method: String?
        var params: JSONValue?
        var result: JSONValue?
        var error: RPCError?
    }

    private func handle(_ data: Data) {
        guard let env = try? JSONDecoder().decode(Envelope.self, from: data) else { return }
        if let id = env.id, let cont = pending.removeValue(forKey: id) {
            if let error = env.error {
                cont.resume(throwing: error)
            } else {
                cont.resume(returning: env.result ?? .null)
            }
        } else if let method = env.method {
            sink.yield(RPCNotification(method: method, params: env.params ?? .null))
        }
    }

    private func finish() {
        guard !closed else { return }
        closed = true
        for cont in pending.values { cont.resume(throwing: RPCError.disconnected) }
        pending.removeAll()
        sink.finish()
        session.invalidateAndCancel()
    }
}
