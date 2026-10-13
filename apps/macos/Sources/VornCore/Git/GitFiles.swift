import Foundation

/// `GitFileDiff` in types.ts: one changed file and its unified diff.
public struct GitFileDiff: Codable, Hashable, Sendable, Identifiable {
    public enum Status: String, Codable, Sendable {
        case added, modified, deleted, renamed

        /// STATUS_LETTER in DiffSidebar.tsx.
        public var letter: String {
            switch self {
            case .modified: "M"
            case .added: "A"
            case .deleted: "D"
            case .renamed: "R"
            }
        }
    }

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
    public var stat: DiffStat
    public var files: [GitFileDiff]
    public init(stat: DiffStat, files: [GitFileDiff]) {
        self.stat = stat
        self.files = files
    }
}

/// One line of a file's diff as drawn; `index` is its line in the raw diff, which comments are keyed by.
public struct DiffRow: Hashable, Sendable, Identifiable {
    public enum Kind: Sendable { case meta, hunk, add, del, ctx }
    public var kind: Kind
    public var index: Int
    public var text: String
    public var oldLine: Int?
    public var newLine: Int?
    public var id: Int { index }

    /// parseDiffRows: header lines dropped, hunks numbered, blank metadata skipped.
    public static func parse(_ diff: String) -> [DiffRow] {
        var rows: [DiffRow] = []
        var old = 0, new = 0, inHunk = false
        for (index, line) in diff.components(separatedBy: "\n").enumerated() {
            if line.hasPrefix("diff --git") || line.hasPrefix("index ") || line.hasPrefix("--- ") || line.hasPrefix("+++ ") { continue }
            if let m = line.firstMatch(of: #/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)/#) {
                old = Int(m.1) ?? 0
                new = Int(m.2) ?? 0
                inHunk = true
                rows.append(DiffRow(kind: .hunk, index: index, text: line))
                continue
            }
            if !inHunk {
                if !line.trimmingCharacters(in: .whitespaces).isEmpty { rows.append(DiffRow(kind: .meta, index: index, text: line)) }
                continue
            }
            if line.hasPrefix("+") {
                rows.append(DiffRow(kind: .add, index: index, text: line, newLine: new)); new += 1
            } else if line.hasPrefix("-") {
                rows.append(DiffRow(kind: .del, index: index, text: line, oldLine: old)); old += 1
            } else if line.hasPrefix(" ") {
                rows.append(DiffRow(kind: .ctx, index: index, text: line, oldLine: old, newLine: new)); old += 1; new += 1
            }
        }
        return rows
    }
}

/// `DiffComment` in DiffSidebar.tsx: a review note on one line of a file's diff, keyed by its index in the raw diff.
public struct DiffComment: Hashable, Sendable {
    public var filePath: String
    public var lineIndex: Int
    public var lineContent: String
    public var comment: String
    public init(filePath: String, lineIndex: Int, lineContent: String, comment: String) {
        self.filePath = filePath
        self.lineIndex = lineIndex
        self.lineContent = lineContent
        self.comment = comment
    }

    /// formatReviewFeedback: what "Send to Agent" types into the session.
    public static func feedback(_ comments: [DiffComment]) -> String {
        var order: [String] = []
        var grouped: [String: [DiffComment]] = [:]
        for c in comments {
            if grouped[c.filePath] == nil { order.append(c.filePath) }
            grouped[c.filePath, default: []].append(c)
        }
        var out = "Please address the following review comments:\n\n"
        for file in order {
            out += "**\(file):**\n"
            for c in grouped[file] ?? [] {
                let code = String(c.lineContent.dropFirst()).trimmingCharacters(in: .whitespaces)
                out += "- Line `\(code)`: \(c.comment)\n"
            }
            out += "\n"
        }
        return out
    }
}

/// A directory entry (`file:listDir`).
public struct FileEntry: Codable, Hashable, Sendable, Identifiable {
    public var name: String
    public var path: String
    public var isDirectory: Bool
    public var id: String { path }
    public init(name: String, path: String, isDirectory: Bool) {
        self.name = name
        self.path = path
        self.isDirectory = isDirectory
    }
}

/// Size and mtime, for telling whether a file moved under an unsaved draft.
public struct FileStamp: Codable, Hashable, Sendable {
    public var size: Double
    public var mtimeMs: Double
    public init(size: Double, mtimeMs: Double) {
        self.size = size
        self.mtimeMs = mtimeMs
    }

    /// editor-drafts.ts hasMoved.
    public static func hasMoved(_ base: FileStamp?, _ current: FileStamp?) -> Bool {
        guard let base else { return false }
        guard let current else { return true }
        return base.size != current.size || base.mtimeMs != current.mtimeMs
    }
}

/// An editor or app a project can open in (`ide:detect`).
public struct DetectedIDE: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var command: String
    public init(id: String, name: String, command: String) {
        self.id = id
        self.name = name
        self.command = command
    }
}

/// The git, file and editor calls the panes make.
public extension VornClient {
    func diffFull(_ cwd: String) async throws -> GitDiffResult? {
        try await call("git:diffFull", .string(cwd)).decode(GitDiffResult?.self)
    }

    /// `git:commit`; nil on success, else why it failed.
    func commit(cwd: String, message: String, includeUnstaged: Bool) async throws -> String? {
        let r = try await call("git:commit", .object(["cwd": .string(cwd), "message": .string(message), "includeUnstaged": .bool(includeUnstaged)]))
        return r["success"]?.boolValue == true ? nil : (r["error"]?.stringValue ?? "Commit failed")
    }

    func push(cwd: String) async throws -> String? {
        let r = try await call("git:push", .string(cwd))
        return r["success"]?.boolValue == true ? nil : (r["error"]?.stringValue ?? "Push failed")
    }

    /// `git:checkoutBranch`; nil on success, else why it failed.
    func checkout(cwd: String, branch: String) async throws -> String? {
        let r = try await call("git:checkoutBranch", .object(["cwd": .string(cwd), "branch": .string(branch)]))
        return r["ok"]?.boolValue == true ? nil : (r["error"]?.stringValue ?? "Failed to checkout '\(branch)'")
    }

    func listRemoteBranches(_ projectPath: String) async throws -> [String] {
        try await call("git:listRemoteBranches", .string(projectPath), as: [String].self)
    }

    func gitBranch(_ cwd: String) async throws -> String? {
        try await call("git:getBranch", .string(cwd)).stringValue
    }

    func detectIDEs() async throws -> [DetectedIDE] {
        try await call("ide:detect", as: [DetectedIDE].self)
    }

    func openInIDE(_ ideId: String, path: String) async throws {
        try await call("ide:open", .object(["ideId": .string(ideId), "projectPath": .string(path)]))
    }

    func listDir(_ path: String, remoteHostId: String? = nil) async throws -> [FileEntry] {
        var p: [String: JSONValue] = ["dirPath": .string(path)]
        if let remoteHostId { p["remoteHostId"] = .string(remoteHostId) }
        return try await call("file:listDir", .object(p), as: [FileEntry].self)
    }

    /// The file's text, or nil when it is binary or unreadable.
    func readFile(_ path: String, remoteHostId: String? = nil) async throws -> String? {
        var p: [String: JSONValue] = ["filePath": .string(path)]
        if let remoteHostId { p["remoteHostId"] = .string(remoteHostId) }
        return try await call("file:readContent", .object(p)).stringValue
    }

    func fileStamp(_ path: String, remoteHostId: String? = nil) async throws -> FileStamp? {
        var p: [String: JSONValue] = ["filePath": .string(path)]
        if let remoteHostId { p["remoteHostId"] = .string(remoteHostId) }
        return try await call("file:stamp", .object(p)).decode(FileStamp?.self)
    }

    /// `file:writeContent`; nil on success, else why it failed.
    func writeFile(_ path: String, content: String, remoteHostId: String? = nil) async throws -> String? {
        var p: [String: JSONValue] = ["filePath": .string(path), "content": .string(content)]
        if let remoteHostId { p["remoteHostId"] = .string(remoteHostId) }
        let r = try await call("file:writeContent", .object(p))
        return r["success"]?.boolValue == true ? nil : (r["error"]?.stringValue ?? "Failed to save")
    }
}

/// string-utils.ts isTruncatedRead: a capped read must not be saved back over the file.
public func isTruncatedRead(_ text: String) -> Bool {
    text.firstMatch(of: #/\n\n--- truncated(?: \(\d+ bytes total\))? ---$/#) != nil
}
