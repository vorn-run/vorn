import Foundation
import Observation
import SwiftUI
import VornCore
import VornPanes
import VornTerminals
import VornUI

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

/// The grid's status filter (⌥1-5).
public enum StatusFilter: String, CaseIterable, Sendable {
    case all, running, waiting, idle, error

    var label: String { rawValue.prefix(1).uppercased() + rawValue.dropFirst() }

    var dot: Color {
        switch self {
        case .all: Theme.gray400
        case .running: Theme.green500
        case .waiting: Theme.yellow500
        case .idle: Theme.gray500
        case .error: Theme.red500
        }
    }

    func matches(_ status: AgentStatus) -> Bool {
        switch self {
        case .all: true
        case .running: status == .running
        case .waiting: status == .waiting
        case .idle: status == .idle
        case .error: status == .error
        }
    }
}

public enum SortMode: String, CaseIterable, Sendable {
    case manual, created, recent

    var label: String { rawValue.prefix(1).uppercased() + rawValue.dropFirst() }
}

public enum LayoutMode: String, Sendable {
    case grid, tabs
}

/// A floating panel the window shows over everything else.
public enum Overlay: Equatable, Sendable {
    case viewOptions, recent, shortcuts, headless, newMenu, sidebarOptions
    /// The session dock's popover: everything when collapsed, else what did not fit.
    case dock
    /// The project's "More" menu.
    case projectMenu(String)
    /// AddProjectDialog.tsx; editing the named project when set.
    case addProject(editing: String?)
    /// WorktreeCleanupDialog.tsx.
    case removeWorktree(projectPath: String, worktreePath: String)
    /// CommandPalette.tsx (⌘K).
    case commandPalette
    /// OnboardingModal.tsx: the welcome guide, on page `onboardingStep`.
    case onboarding
}

/// What the sidebar's projects section lists (ProjectsSectionToolbar.tsx).
public enum SidebarViewMode: String, CaseIterable, Sendable {
    case worktrees, worktreesSessions = "worktrees-sessions", sessions, sessionsFlat = "sessions-flat"

    var label: String {
        switch self {
        case .worktrees: "Projects > Worktrees"
        case .worktreesSessions: "Projects > Worktrees > Sessions"
        case .sessions: "Projects > Sessions"
        case .sessionsFlat: "Sessions (Flat)"
        }
    }
}

public enum ProjectSort: String, CaseIterable, Sendable {
    case manual, name, recent

    var label: String { self == .recent ? "Recent activity" : rawValue.capitalized }
}

public enum WorktreeSort: String, CaseIterable, Sendable {
    case name, recent

    var label: String { self == .recent ? "Recent activity" : "Name" }
}

public enum WorktreeFilter: String, CaseIterable, Sendable {
    case all, active

    var label: String { self == .active ? "With active sessions" : "All" }
}

/// A right-click menu and where it opened, in the root view's space.
public struct ContextMenuState: Equatable, Sendable {
    public enum Target: Equatable, Sendable {
        case grid
        case card(String)
    }

    public var target: Target
    public var point: CGPoint
    /// A submenu shown open from the start, for renders.
    public var submenu: String?

    public init(target: Target, point: CGPoint, submenu: String? = nil) {
        self.target = target
        self.point = point
        self.submenu = submenu
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
    /// The session dock folded into one count button (sessionDockCollapsed).
    public var sessionDockCollapsed = false
    public var projectsOpen = true
    /// Per-project override of the sidebar's default expansion.
    public var expandedProjects: [String: Bool] = [:]
    /// Bumped to pull keyboard focus into a session's terminal.
    public var focusRequests: [String: Int] = [:]
    /// The panes sessions own, popped-out cards and the changes panel.
    public var panes = PanesModel()
    /// The changes panel's commit dialog is open.
    public var commitOpen = false
    /// The window's toasts (Toast.tsx).
    public let toasts = ToastCenter()
    /// An error for the toast stack; set it and it is shown and cleared.
    public var lastError: String? {
        didSet { if let m = lastError { toasts.show(m, .error); lastError = nil } }
    }
    /// SettingsPage.tsx covers the window while open.
    public var settingsOpen = false
    /// The sidebar's update notice was waved away; the gear keeps a dot.
    public var updateBannerDismissed = false

    /// The composer tip an offscreen render shows; a live window picks one at random.
    public var launcherTip = 1
    /// OnboardingModal.tsx: the welcome guide's page.
    public var onboardingStep = 0
    /// Remembers the guide was seen; set where the store is known.
    @ObservationIgnored public var onOnboardingDone: () -> Void = {}
    /// index.ts `toggleWidget`, for the Window menu; set once the widget is installed.
    @ObservationIgnored public var toggleWidget: (() -> Void)?
    public var statusFilter: StatusFilter = .all
    public var sortMode: SortMode = .manual
    /// 0 auto, 1-4 fixed, -1 flexible.
    public var gridColumns = 0
    /// The tab the tabs layout shows.
    public var activeTabID: String?
    public var overlay: Overlay?
    public var contextMenu: ContextMenuState?
    /// The sidebar's toolbar choices, kept across launches as the renderer keeps them.
    public var sidebarViewMode = SidebarViewMode(rawValue: AppModel.saved("viewMode") ?? "") ?? .worktreesSessions {
        didSet { save("viewMode", sidebarViewMode.rawValue) }
    }
    public var projectSort = ProjectSort(rawValue: AppModel.saved("projectSort") ?? "") ?? .manual {
        didSet { save("projectSort", projectSort.rawValue) }
    }
    public var worktreeSort = WorktreeSort(rawValue: AppModel.saved("worktreeSort") ?? "") ?? .name {
        didSet { save("worktreeSort", worktreeSort.rawValue) }
    }
    public var worktreeFilter = WorktreeFilter(rawValue: AppModel.saved("worktreeFilter") ?? "") ?? .all {
        didSet { save("worktreeFilter", worktreeFilter.rawValue) }
    }
    /// Renders and tests leave the user's defaults alone.
    @ObservationIgnored public var persistsSidebar = true

    var hasSidebarOptions: Bool {
        sidebarViewMode != .worktreesSessions || projectSort != .manual || worktreeSort != .name || worktreeFilter != .all
    }

    private static func saved(_ key: String) -> String? {
        UserDefaults.standard.string(forKey: "vorn.sidebar.\(key)")
    }

    private func save(_ key: String, _ value: String) {
        if persistsSidebar { UserDefaults.standard.set(value, forKey: "vorn.sidebar.\(key)") }
    }

    /// F2: the session whose name field is open.
    public var renamingSessionID: String?
    /// The restored banner was dismissed for this launch.
    public var bannerDismissed = false
    /// Frames of the controls panels hang from, in the root view's space.
    public var anchors: [String: CGRect] = [:]
    /// Set while a layout change is being saved, so the switch is immediate.
    public var layoutOverride: LayoutMode?
    /// The recent list a render shows instead of asking vornd.
    public var recentFixture: [RecentSession]?

    public static let mainWorktree = "__main__"

    public init() {}

    public func layoutMode(_ store: VornStore) -> LayoutMode {
        layoutOverride ?? LayoutMode(rawValue: store.defaults?.layoutMode ?? "") ?? .grid
    }

    /// Sessions in the active project or workspace, whatever their status or worktree (`focusableIds`).
    public func scopedSessions(_ store: VornStore) -> [TerminalSession] {
        let workspaceProjects = Set(store.projects.filter { $0.workspace == activeWorkspace }.map(\.name))
        let known = Set(store.projects.map(\.name))
        let claimed = panes.claimedTerminalIDs
        return sorted(store.sessions.filter { s in
            if claimed.contains(s.id) { return false }
            if let activeProject { return s.projectName == activeProject }
            return workspaceProjects.contains(s.projectName) || !known.contains(s.projectName)
        }, store)
    }

    /// The sessions the grid shows (`useVisibleTerminals`): scope, worktree, status, then sort; minimized ones dock.
    public func visibleSessions(_ store: VornStore, includeMinimized: Bool = false) -> [TerminalSession] {
        scopedSessions(store).filter { s in
            if !includeMinimized && minimizedIDs.contains(s.id) { return false }
            if activeProject != nil, let wt = activeWorktreePath {
                if wt == Self.mainWorktree {
                    if s.isWorktree == true { return false }
                } else if s.worktreePath != wt {
                    return false
                }
            }
            return statusFilter.matches(s.status)
        }
    }

    private func sorted(_ list: [TerminalSession], _ store: VornStore) -> [TerminalSession] {
        switch sortMode {
        case .manual: list
        case .created: list.sorted { $0.createdAt > $1.createdAt }
        case .recent:
            list.sorted { (store.lastActivity[$0.id] ?? $0.createdAt) > (store.lastActivity[$1.id] ?? $1.createdAt) }
        }
    }

    /// Whether anything in the view options differs from its default (the dot on the sliders).
    public func hasActiveViewOptions(_ store: VornStore) -> Bool {
        statusFilter != .all || sortMode != .manual || gridColumns != 0 || layoutMode(store) != .grid
    }

    /// ⌘] / ⌘[: expanded, the next session in scope; tabs, the next tab; grid, the next card selected.
    public func cycleFocus(_ store: VornStore, by step: Int) {
        if let expanded = expandedSessionID {
            let ids = scopedSessions(store).map(\.id)
            guard !ids.isEmpty else { return }
            let next = ids.firstIndex(of: expanded).map { ids[wrap($0 + step, ids.count)] } ?? ids[0]
            setFocused(next)
            return
        }
        let ids = visibleSessions(store).map(\.id)
        guard !ids.isEmpty else { return }
        let first = step > 0 ? 0 : ids.count - 1
        if layoutMode(store) == .tabs {
            let current = activeTabID.flatMap { ids.firstIndex(of: $0) }
            showTab(current.map { ids[wrap($0 + step, ids.count)] } ?? ids[first])
        } else {
            let current = selectedSessionID.flatMap { ids.firstIndex(of: $0) }
            focus(current.map { ids[wrap($0 + step, ids.count)] } ?? ids[first])
        }
    }

    private func wrap(_ i: Int, _ n: Int) -> Int { (i % n + n) % n }

    /// ui-slice `setFocusedTerminal`: expands a card (nil collapses) and selects it.
    public func setFocused(_ id: String?) {
        expandedSessionID = id
        selectedSessionID = id
        if let id { focusRequests[id, default: 0] += 1 }
    }

    /// The tabs layout's active tab, with its terminal taking the keyboard.
    public func showTab(_ id: String) {
        activeTabID = id
        focus(id)
    }

    /// SessionItem.tsx click: the session's tab in the tabs layout, else its card expanded.
    public func reveal(_ id: String, _ store: VornStore) {
        if layoutMode(store) == .tabs {
            expandedSessionID = nil
            showTab(id)
        } else {
            setFocused(id)
        }
    }

    /// Whether the sidebar marks a session as the one on screen (SessionItem.tsx `isActive`).
    public func isShowing(_ id: String, _ store: VornStore) -> Bool {
        layoutMode(store) == .tabs ? activeTabID == id : expandedSessionID == id
    }

    /// Esc: the innermost open thing closes first.
    @discardableResult
    public func escape() -> Bool {
        if contextMenu != nil {
            contextMenu = nil
        } else if overlay == .onboarding {
            closeOnboarding()
        } else if overlay != nil {
            overlay = nil
        } else if settingsOpen {
            settingsOpen = false
        } else if panes.maximizedPaneId != nil {
            panes.maximizedPaneId = nil
        } else if panes.diffSessionID != nil {
            panes.diffSessionID = nil
        } else if renamingSessionID != nil {
            renamingSessionID = nil
        } else if let expanded = expandedSessionID {
            expandedSessionID = nil
            selectedSessionID = expanded
        } else if selectedSessionID != nil {
            selectedSessionID = nil
        } else {
            return false
        }
        return true
    }

    /// OnboardingModal.tsx `close`: shuts the guide and remembers it was seen.
    public func closeOnboarding() {
        guard overlay == .onboarding else { return }
        overlay = nil
        onOnboardingDone()
    }

    /// The guide's "Try Command Palette".
    public func openCommandPalette() {
        contextMenu = nil
        overlay = .commandPalette
    }

    /// The welcome guide, from its first page.
    public func openOnboarding() {
        contextMenu = nil
        onboardingStep = 0
        overlay = .onboarding
    }

    /// ⌘1-9: the session at a position: expanded if a card is, the tab in tabs, else selected.
    public func jump(to position: Int, _ store: VornStore) {
        let ids = visibleSessions(store).map(\.id)
        guard position >= 1, position <= ids.count else { return }
        let id = ids[position - 1]
        if expandedSessionID != nil {
            setFocused(id)
        } else if layoutMode(store) == .tabs {
            showTab(id)
        } else {
            focus(id)
        }
    }

    /// ui-slice `reorderTerminals`: `dragged` takes the slot of `target`; nil when nothing moves.
    public static func reordered(_ ids: [String], moving dragged: String, onto target: String) -> [String]? {
        guard let from = ids.firstIndex(of: dragged), let to = ids.firstIndex(of: target), from != to else { return nil }
        var order = ids
        order.insert(order.remove(at: from), at: to)
        return order
    }

    /// ⌘W while a card is expanded: collapses it and keeps it selected.
    @discardableResult
    public func collapse() -> Bool {
        guard let expanded = expandedSessionID else { return false }
        expandedSessionID = nil
        selectedSessionID = expanded
        return true
    }

    /// F2: renames the focused or selected session.
    public func startRename() {
        renamingSessionID = expandedSessionID ?? selectedSessionID
    }

    public func toggle(_ panel: Overlay) {
        contextMenu = nil
        overlay = overlay == panel ? nil : panel
    }

    public func focus(_ id: String) {
        selectedSessionID = id
        focusRequests[id, default: 0] += 1
    }

    /// ProjectItem / "All Projects": filters the grid to a project and collapses any expanded card.
    public func select(project name: String?) {
        activeProject = name
        activeWorktreePath = nil
        setFocused(nil)
    }

    /// WorktreeItem click: the project and one of its worktrees (`mainWorktree` for the checkout),
    /// toggling off when already chosen.
    public func toggle(worktree path: String, in project: String) {
        let same = activeProject == project && activeWorktreePath == path
        activeProject = project
        activeWorktreePath = same ? nil : path
        setFocused(nil)
    }
}

/// Owns the grid client for the store's current vornd, replacing it when vornd moves.
@MainActor
@Observable
public final class EngineHolder {
    public private(set) var engine: TerminalEngine?

    public init() {}

    public func update(socket: String?, readOnly: Bool, fontSize: Double? = nil) {
        let size = fontSize.map { CGFloat($0) } ?? Theme.Terminal.fontSize
        guard engine?.socket != socket || engine?.readOnly != readOnly || engine?.fontSize != size else { return }
        engine?.shutdown()
        engine = socket.map { TerminalEngine(socket: $0, readOnly: readOnly, fontSize: size) }
    }
}
