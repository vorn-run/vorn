import Foundation
import VornCore

/// `DiffComment` in DiffSidebar.tsx: a review note on one line of a file's diff.
public struct DiffComment: Hashable, Sendable {
    public var filePath: String
    /// The line's index in the raw diff, which comments are keyed by.
    public var lineIndex: Int
    public var lineContent: String
    public var comment: String

    public init(filePath: String, lineIndex: Int, lineContent: String, comment: String) {
        self.filePath = filePath
        self.lineIndex = lineIndex
        self.lineContent = lineContent
        self.comment = comment
    }
}

/// `DiffRow` in DiffSidebar.tsx: one line of a file's diff as it is drawn.
public struct DiffRow: Hashable, Sendable, Identifiable {
    public enum Kind: Sendable { case meta, hunk, add, del, ctx }

    public var kind: Kind
    public var index: Int
    public var text: String
    public var oldLine: Int?
    public var newLine: Int?

    public var id: Int { index }
}

public enum DiffReview {
    private static let hunkPattern = try! NSRegularExpression(pattern: #"^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@"#)

    /// parseDiffRows: header lines dropped, hunks numbered, blank metadata skipped.
    public static func rows(_ diff: String) -> [DiffRow] {
        var rows: [DiffRow] = []
        var oldLine = 0
        var newLine = 0
        var inHunk = false
        for (index, sub) in diff.split(separator: "\n", omittingEmptySubsequences: false).enumerated() {
            let line = String(sub)
            if line.hasPrefix("diff --git") || line.hasPrefix("index ") || line.hasPrefix("--- ") || line.hasPrefix("+++ ") {
                continue
            }
            let ns = line as NSString
            if let m = hunkPattern.firstMatch(in: line, range: NSRange(location: 0, length: ns.length)) {
                oldLine = Int(ns.substring(with: m.range(at: 1))) ?? 0
                newLine = Int(ns.substring(with: m.range(at: 2))) ?? 0
                inHunk = true
                rows.append(DiffRow(kind: .hunk, index: index, text: line))
                continue
            }
            if !inHunk {
                if !line.trimmingCharacters(in: .whitespaces).isEmpty {
                    rows.append(DiffRow(kind: .meta, index: index, text: line))
                }
                continue
            }
            if line.hasPrefix("+") {
                rows.append(DiffRow(kind: .add, index: index, text: line, newLine: newLine))
                newLine += 1
            } else if line.hasPrefix("-") {
                rows.append(DiffRow(kind: .del, index: index, text: line, oldLine: oldLine))
                oldLine += 1
            } else if line.hasPrefix(" ") {
                rows.append(DiffRow(kind: .ctx, index: index, text: line, oldLine: oldLine, newLine: newLine))
                oldLine += 1
                newLine += 1
            }
        }
        return rows
    }

    /// formatReviewFeedback: comments grouped by file, in the order files were first commented on.
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
                let code = String(c.lineContent.dropFirst()).trimmingCharacters(in: .whitespacesAndNewlines)
                out += "- Line `\(code)`: \(c.comment)\n"
            }
            out += "\n"
        }
        return out
    }

    /// buildFeedbackPrompt (prompt-builder.ts): the review wrapped for the agent that did the task.
    public static func prompt(_ feedback: String, task: VornTask, projectName: String) -> String {
        [
            "# Review Feedback for: \(task.title)", "",
            "**Project:** \(projectName)", "**Task ID:** \(task.id)", "",
            feedback, "",
            "Please address the review feedback above and update the task when done.",
        ].joined(separator: "\n")
    }

    /// STATUS_LETTER: the letter says which kind of change, in one tone.
    public static func letter(_ status: GitFileDiff.Status) -> String {
        switch status {
        case .modified: "M"
        case .added: "A"
        case .deleted: "D"
        case .renamed: "R"
        }
    }
}
