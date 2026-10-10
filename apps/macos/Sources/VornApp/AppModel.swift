import Foundation
import Observation
import VornCore
import VornTerminals

public enum MainViewMode: String, CaseIterable, Sendable {
    case sessions, tasks, workflows

    var label: String {
        switch self {
        case .sessions: "Sessions"
        case .tasks: "Tasks"
        case .workflows: "Workflows"
        }
    }

    var shortcut: String {
        switch self {
        case .sessions: "⌘S"
        case .tasks: "⌘T"
        case .workflows: "⌘⇧W"
        }
    }
}

/// The window's UI state, as the renderer's ui-slice.
@MainActor
@Observable
public final class AppModel {
    public var mainViewMode: MainViewMode = .sessions
    public var sidebarOpen = true
    public var sidebarWidth: CGFloat = 256
    public var activeWorkspace = Workspace.personalID
    public var activeProject: String?
    /// A worktree path, or `mainWorktree` for the project's main checkout.
    public var activeWorktreePath: String?
    public var selectedSessionID: String?
    public var expandedSessionID: String?
    public var minimizedIDs: Set<String> = []
    public var projectsOpen = true
    /// Per-project override of the sidebar's default expansion.
    public var expandedProjects: [String: Bool] = [:]
    /// Bumped to pull keyboard focus into a session's terminal.
    public var focusRequests: [String: Int] = [:]
    public var lastError: String?
    /// The composer tip an offscreen render shows; a live window picks one at random.
    public var launcherTip = 1

    public static let mainWorktree = "__main__"

    public init() {}

    /// The sessions the grid shows: active workspace, then project and worktree filters.
    public func visibleSessions(_ store: VornStore) -> [TerminalSession] {
        let workspaceProjects = Set(store.projects.filter { $0.workspace == activeWorkspace }.map(\.name))
        let known = Set(store.projects.map(\.name))
        return store.sessions.filter { s in
            if minimizedIDs.contains(s.id) { return false }
            if let activeProject {
                guard s.projectName == activeProject else { return false }
                if let wt = activeWorktreePath {
                    if wt == Self.mainWorktree { return s.isWorktree != true }
                    return s.worktreePath == wt
                }
                return true
            }
            return workspaceProjects.contains(s.projectName) || !known.contains(s.projectName)
        }
    }

    public func focus(_ id: String) {
        selectedSessionID = id
        focusRequests[id, default: 0] += 1
    }

    public func select(project name: String?) {
        activeProject = name
        activeWorktreePath = nil
    }
}

/// Owns the grid client for the store's current vornd, replacing it when vornd moves.
@MainActor
@Observable
public final class EngineHolder {
    public private(set) var engine: TerminalEngine?

    public init() {}

    public func update(socket: String?, readOnly: Bool) {
        guard engine?.socket != socket || engine?.readOnly != readOnly else { return }
        engine?.shutdown()
        engine = socket.map { TerminalEngine(socket: $0, readOnly: readOnly) }
    }
}
