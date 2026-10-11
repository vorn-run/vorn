import Foundation
import VornCore

/// `GitFileDiff` in types.ts: one file of a worktree's changes.
public struct GitFileDiff: Codable, Hashable, Sendable, Identifiable {
    public enum Status: String, Codable, Sendable { case added, modified, deleted, renamed }

    public var filePath: String
    public var status: Status
    public var insertions: Int
    public var deletions: Int
    public var diff: String

    public var id: String { filePath }

    public init(filePath: String, status: Status, insertions: Int, deletions: Int, diff: String) {
        self.filePath = filePath
        self.status = status
        self.insertions = insertions
        self.deletions = deletions
        self.diff = diff
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        filePath = try c.decode(String.self, forKey: .filePath)
        status = (try? c.decode(Status.self, forKey: .status)) ?? .modified
        insertions = (try? c.decode(Int.self, forKey: .insertions)) ?? 0
        deletions = (try? c.decode(Int.self, forKey: .deletions)) ?? 0
        diff = (try? c.decode(String.self, forKey: .diff)) ?? ""
    }
}

/// `GitDiffResult` in types.ts: what `git:diffFull` answers.
public struct GitDiffResult: Codable, Hashable, Sendable {
    public struct Stat: Codable, Hashable, Sendable {
        public var filesChanged: Int
        public var insertions: Int
        public var deletions: Int

        public init(filesChanged: Int, insertions: Int, deletions: Int) {
            self.filesChanged = filesChanged
            self.insertions = insertions
            self.deletions = deletions
        }
    }

    public var stat: Stat
    public var files: [GitFileDiff]

    public init(stat: Stat, files: [GitFileDiff]) {
        self.stat = stat
        self.files = files
    }
}

/// What `git:commit` and `git:push` answer.
public struct GitOutcome: Codable, Hashable, Sendable {
    public var success: Bool
    public var error: String?

    public init(success: Bool, error: String? = nil) {
        self.success = success
        self.error = error
    }
}

/// What the detail panel asks of vornd beyond the board: images, related runs, and the worktree's changes.
public protocol TaskExtrasService: Sendable {
    /// Copies a local image into the task's folder (`task:imageSave`); answers the stored file name.
    func saveTaskImage(taskId: String, sourcePath: String) async throws -> String
    /// Stores image bytes, as a paste does (`task:imageUpload`).
    func uploadTaskImage(taskId: String, data: Data, filename: String) async throws -> String
    func deleteTaskImage(taskId: String, filename: String) async throws
    func cleanupTaskImages(taskId: String) async throws
    func taskImagePath(taskId: String, filename: String) async throws -> String
    /// Sets fields `task:update` does not take by saving the whole configuration, as the renderer does; null removes one.
    func patchTaskConfig(id: String, _ fields: [String: JSONValue]) async throws
    func listTaskRuns(taskId: String, limit: Int) async throws -> [WorkflowExecution]
    func listWorkflows() async throws -> [WorkflowDefinition]
    func gitDiff(cwd: String) async throws -> GitDiffResult?
    func gitCommit(cwd: String, message: String, includeUnstaged: Bool) async throws -> GitOutcome
    func gitPush(cwd: String) async throws -> GitOutcome
}

extension VornClient: TaskExtrasService {
    public func saveTaskImage(taskId: String, sourcePath: String) async throws -> String {
        try await call("task:imageSave", ["taskId": taskId, "sourcePath": sourcePath], as: String.self)
    }

    public func uploadTaskImage(taskId: String, data: Data, filename: String) async throws -> String {
        let params = ["taskId": taskId, "base64": data.base64EncodedString(), "filename": filename]
        return try await call("task:imageUpload", params, as: String.self)
    }

    public func deleteTaskImage(taskId: String, filename: String) async throws {
        try await call("task:imageDelete", .object(["taskId": .string(taskId), "filename": .string(filename)]))
    }

    public func cleanupTaskImages(taskId: String) async throws {
        try await call("task:imageCleanup", .string(taskId))
    }

    public func taskImagePath(taskId: String, filename: String) async throws -> String {
        try await call("task:imageGetPath", ["taskId": taskId, "filename": filename], as: String.self)
    }

    public func patchTaskConfig(id: String, _ fields: [String: JSONValue]) async throws {
        guard case .object(var config) = try await call("config:load") else {
            throw VornError.badResponse("config:load")
        }
        var tasks = config["tasks"]?.arrayValue ?? []
        guard let i = tasks.firstIndex(where: { $0["id"]?.stringValue == id }), case .object(var task) = tasks[i] else {
            throw VornError.refused("That task no longer exists")
        }
        for (key, value) in fields {
            if case .null = value { task.removeValue(forKey: key) } else { task[key] = value }
        }
        tasks[i] = .object(task)
        config["tasks"] = .array(tasks)
        try await call("config:save", .object(config))
    }

    public func listTaskRuns(taskId: String, limit: Int) async throws -> [WorkflowExecution] {
        let params: JSONValue = .object(["taskId": .string(taskId), "limit": .number(Double(limit))])
        return try await call("workflowRun:listByTask", params, as: [WorkflowExecution].self)
    }

    public func gitDiff(cwd: String) async throws -> GitDiffResult? {
        try await call("git:diffFull", .string(cwd), as: GitDiffResult?.self)
    }

    public func gitCommit(cwd: String, message: String, includeUnstaged: Bool) async throws -> GitOutcome {
        let params: JSONValue = .object([
            "cwd": .string(cwd), "message": .string(message), "includeUnstaged": .bool(includeUnstaged),
        ])
        return try await call("git:commit", params, as: GitOutcome.self)
    }

    public func gitPush(cwd: String) async throws -> GitOutcome {
        try await call("git:push", .string(cwd), as: GitOutcome.self)
    }
}

extension AgentKind {
    /// supportsExactSessionResume (types.ts): every agent but Gemini reopens a conversation by its id.
    public var supportsExactSessionResume: Bool { self != .gemini }
}
