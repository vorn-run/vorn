import Foundation

/// The fields `task:create` takes.
public struct TaskDraft: Encodable, Sendable, Equatable {
    public var projectName: String
    public var title: String
    public var description: String?
    public var status: TaskStatus?
    public var branch: String?
    public var useWorktree: Bool?
    public var assignedAgent: AgentKind?

    public init(projectName: String, title: String, description: String? = nil, status: TaskStatus? = nil,
                branch: String? = nil, useWorktree: Bool? = nil, assignedAgent: AgentKind? = nil) {
        self.projectName = projectName
        self.title = title
        self.description = description
        self.status = status
        self.branch = branch
        self.useWorktree = useWorktree
        self.assignedAgent = assignedAgent
    }
}

/// The fields `task:update` takes; nil leaves a field as it is.
public struct TaskPatch: Encodable, Sendable, Equatable {
    public var projectName: String?
    public var title: String?
    public var description: String?
    public var status: TaskStatus?
    public var branch: String?
    public var useWorktree: Bool?
    public var assignedAgent: AgentKind?
    /// The server skips nulls, so unassigning sends an empty agent, which reads back as none.
    public var clearsAgent: Bool

    public init(projectName: String? = nil, title: String? = nil, description: String? = nil, status: TaskStatus? = nil,
                branch: String? = nil, useWorktree: Bool? = nil, assignedAgent: AgentKind? = nil, clearsAgent: Bool = false) {
        self.projectName = projectName
        self.title = title
        self.description = description
        self.status = status
        self.branch = branch
        self.useWorktree = useWorktree
        self.assignedAgent = assignedAgent
        self.clearsAgent = clearsAgent
    }

    enum CodingKeys: String, CodingKey {
        case projectName, title, description, status, branch, useWorktree, assignedAgent
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encodeIfPresent(projectName, forKey: .projectName)
        try c.encodeIfPresent(title, forKey: .title)
        try c.encodeIfPresent(description, forKey: .description)
        try c.encodeIfPresent(status, forKey: .status)
        try c.encodeIfPresent(branch, forKey: .branch)
        try c.encodeIfPresent(useWorktree, forKey: .useWorktree)
        if let assignedAgent {
            try c.encode(assignedAgent, forKey: .assignedAgent)
        } else if clearsAgent {
            try c.encode("", forKey: .assignedAgent)
        }
    }
}

/// The task methods a board needs; `VornClient` answers them over the socket.
public protocol TaskService: Sendable {
    func listTasks() async throws -> [VornTask]
    func getTask(id: String) async throws -> VornTask?
    func createTask(_ draft: TaskDraft) async throws -> VornTask
    func updateTask(id: String, _ patch: TaskPatch) async throws -> VornTask?
    func deleteTask(id: String) async throws
    func archiveTask(id: String, archived: Bool) async throws
    func reorderTasks(ids: [String]) async throws
    func listProjects() async throws -> [VornProject]
    func taskViewMode() async throws -> TaskViewMode
    func setTaskViewMode(_ mode: TaskViewMode) async throws
    func installedAgents() async throws -> [AgentKind: Bool]
    /// Yields each time the board may have changed, and once on every reconnect.
    func boardChanges() async -> AsyncStream<Void>
}

private struct OkAnswer: Decodable {
    let ok: Bool
    let task: VornTask?
}

private struct IdParams: Encodable { let id: String }

extension VornClient: TaskService {
    public func listTasks() async throws -> [VornTask] {
        try await call("task:list", ["includeDescription": true], as: [VornTask].self)
    }

    public func getTask(id: String) async throws -> VornTask? {
        try await call("task:get", IdParams(id: id), as: VornTask?.self)
    }

    public func createTask(_ draft: TaskDraft) async throws -> VornTask {
        let answer = try await call("task:create", draft, as: OkAnswer.self)
        guard answer.ok, let task = answer.task else {
            throw VornError.refused("No project named \(draft.projectName)")
        }
        return task
    }

    @discardableResult
    public func updateTask(id: String, _ patch: TaskPatch) async throws -> VornTask? {
        var params = try JSONValue.from(patch)
        if case .object(var o) = params {
            o["id"] = .string(id)
            params = .object(o)
        }
        let answer: OkAnswer = try await call("task:update", params).decode()
        guard answer.ok else { throw VornError.refused("That task no longer exists") }
        return answer.task
    }

    public func deleteTask(id: String) async throws {
        _ = try await call("task:delete", IdParams(id: id), as: OkAnswer.self)
    }

    public func archiveTask(id: String, archived: Bool) async throws {
        struct P: Encodable { let id: String; let archived: Bool }
        let answer = try await call("task:archive", P(id: id, archived: archived), as: OkAnswer.self)
        guard answer.ok else { throw VornError.refused("Only done or cancelled tasks can be archived") }
    }

    public func reorderTasks(ids: [String]) async throws {
        struct P: Encodable { let ids: [String] }
        _ = try await call("task:reorder", P(ids: ids), as: OkAnswer.self)
    }

    public func listProjects() async throws -> [VornProject] {
        try await call("project:list").decode([VornProject].self)
    }

    public func taskViewMode() async throws -> TaskViewMode {
        let config = try await call("config:load")
        return Self.viewMode(in: config)
    }

    /// Today's renderer saves the whole configuration with the one default changed; this does the same.
    public func setTaskViewMode(_ mode: TaskViewMode) async throws {
        guard case .object(var config) = try await call("config:load") else {
            throw VornError.badResponse("config:load")
        }
        var defaults: [String: JSONValue] = [:]
        if case .object(let d) = config["defaults"] ?? .null { defaults = d }
        defaults["taskViewMode"] = .string(mode.rawValue)
        config["defaults"] = .object(defaults)
        try await call("config:save", .object(config))
    }

    public func installedAgents() async throws -> [AgentKind: Bool] {
        let raw = try await call("agent:detectInstalled").decode([String: Bool].self)
        var out: [AgentKind: Bool] = [:]
        for (k, v) in raw { if let a = AgentKind(rawValue: k) { out[a] = v } }
        return out
    }

    public func boardChanges() async -> AsyncStream<Void> {
        let changes = notifications("config:changed")
        let states = stateUpdates()
        return AsyncStream { continuation in
            let a = Task {
                for await _ in changes { continuation.yield(()) }
            }
            let b = Task {
                var wasConnected = false
                for await state in states {
                    if state == .connected {
                        if wasConnected { continuation.yield(()) }
                        wasConnected = true
                    }
                }
            }
            continuation.onTermination = { _ in a.cancel(); b.cancel() }
        }
    }

    static func viewMode(in config: JSONValue) -> TaskViewMode {
        config["defaults"]?["taskViewMode"]?.stringValue.flatMap(TaskViewMode.init(rawValue:)) ?? .list
    }
}
