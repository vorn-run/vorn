import Foundation
import VornCore

/// editor-drafts.ts: unsaved edits kept across quits, keyed by tab or card, with the file's stamp when taken.
public struct EditorDraft: Codable, Equatable, Sendable {
    public var filePath: String
    public var text: String
    public var base: FileStamp?
    public var savedAt: Double
}

public final class EditorDrafts: @unchecked Sendable {
    public static let shared = EditorDrafts()
    private let defaults: UserDefaults
    private let key = "vorn:drafts"

    public init(defaults: UserDefaults = .standard) { self.defaults = defaults }

    private func load() -> [String: EditorDraft] {
        guard let d = defaults.data(forKey: key) else { return [:] }
        return (try? JSONDecoder().decode([String: EditorDraft].self, from: d)) ?? [:]
    }

    private func save(_ drafts: [String: EditorDraft]) {
        defaults.set(try? JSONEncoder().encode(drafts), forKey: key)
    }

    public func read(_ paneID: String, filePath: String) -> EditorDraft? {
        guard let d = load()[paneID], d.filePath == filePath else { return nil }
        return d
    }

    public func write(_ paneID: String, filePath: String, text: String, base: FileStamp?) {
        var all = load()
        all[paneID] = EditorDraft(filePath: filePath, text: text, base: base, savedAt: Date().timeIntervalSince1970 * 1000)
        save(all)
    }

    public func forget(_ paneID: String) {
        var all = load()
        guard all.removeValue(forKey: paneID) != nil else { return }
        save(all)
    }
}
