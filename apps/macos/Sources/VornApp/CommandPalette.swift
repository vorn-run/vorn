import AppKit
import SwiftUI
import VornCore
import VornSettings
import VornUI

// MARK: Commands

/// CommandPalette.tsx: a command's group, in the order the palette lists them.
public enum PaletteCategory: String, CaseIterable, Sendable {
    case actions, terminals, recent, projects, workflows, quicklaunch, filter

    public var label: String {
        switch self {
        case .actions: "Actions"
        case .terminals: "Terminals"
        case .recent: "Recent Sessions"
        case .projects: "Projects"
        case .workflows: "Workflows"
        case .quicklaunch: "Quick Launch"
        case .filter: "Filter & Sort"
        }
    }
}

public enum PaletteIcon: Sendable {
    case none
    case lucide(LucideGlyph)
    case agent(AgentType)
    case file(String)
}

/// What running a command does, so the list can be built and checked without a window.
public enum PaletteAction: Equatable, Sendable {
    case newSession, openSettings, toggleSidebar, shortcuts, addProject, addWorkflow, openTasks, createTask
    case stopServer, toggleLayout, welcomeGuide, newTerminal, copyMCPURL
    case settings(SettingsCategory)
    case focus(String)
    case focusCard(String)
    case resume(RecentSession)
    case allProjects
    case project(String)
    case runWorkflow(String)
    case launch(AgentType, project: String, worktree: Worktree?)
    case launchNewWorktree(AgentType, project: String)
    case terminalIn(project: String, worktree: Worktree?)
    case statusFilter(StatusFilter)
    case sort(SortMode)
}

public struct PaletteCommand: Identifiable, Sendable {
    public var id: String
    public var label: String
    public var sublabel: String?
    public var category: PaletteCategory
    public var icon: PaletteIcon = .none
    public var shortcut: String?
    public var keywords: [String] = []
    public var action: PaletteAction
}

/// A file popped out of a session into its own card (usePromotedCards.ts).
public struct PaletteCard: Sendable {
    public var id: String
    public var sessionId: String
    public var path: String

    public var name: String { (path as NSString).lastPathComponent }
}

/// Everything the palette lists, read from the stores when it opens.
public struct PaletteSources {
    public var sessions: [TerminalSession] = []
    /// Shells held by a session's terminals panel; no other surface shows them.
    public var claimed: Set<String> = []
    public var cards: [PaletteCard] = []
    public var recent: [RecentSession] = []
    public var projects: [Project] = []
    public var workflows: [WorkflowDefinition] = []
    public var installedAgents: [AgentType] = []
    public var gitRepos: [String: Bool] = [:]
    public var worktrees: [String: [Worktree]] = [:]
    public var now = Date()

    public init() {}
}

public enum PaletteCommands {
    static let mcpURL = "http://localhost:56433/mcp"

    /// useCommands: every command, in the order the renderer pushes them.
    public static func build(_ src: PaletteSources) -> [PaletteCommand] {
        var out: [PaletteCommand] = []
        func add(_ id: String, _ label: String, _ category: PaletteCategory, _ icon: PaletteIcon = .none,
                 sublabel: String? = nil, shortcut: String? = nil, keywords: [String] = [], _ action: PaletteAction) {
            out.append(PaletteCommand(id: id, label: label, sublabel: sublabel, category: category, icon: icon,
                                      shortcut: shortcut, keywords: keywords, action: action))
        }

        add("action:new-session", "New Session", .actions, .lucide(.plus), shortcut: "⌘N",
            keywords: ["create", "add", "agent", "terminal", "launch"], .newSession)
        add("action:settings", "Open Settings", .actions, .lucide(.settings), shortcut: "⌘,",
            keywords: ["preferences", "config", "options"], .openSettings)
        add("action:toggle-sidebar", "Toggle Sidebar", .actions, .lucide(.panelLeft), shortcut: "⌘B",
            keywords: ["hide", "show", "panel"], .toggleSidebar)
        add("action:shortcuts-panel", "Keyboard Shortcuts", .actions, .lucide(.keyboard), shortcut: "⌘?",
            keywords: ["shortcuts", "keys", "help", "reference", "bindings", "hotkeys"], .shortcuts)
        add("action:add-project", "Add Project", .actions, .lucide(.folderPlus),
            keywords: ["new project", "create project"], .addProject)
        add("action:add-workflow", "Add Workflow", .actions, .lucide(.workflow),
            keywords: ["new workflow", "create workflow", "schedule"], .addWorkflow)
        add("action:open-tasks", "Open Tasks View", .actions, .lucide(.listTodo),
            keywords: ["task", "kanban", "queue", "todo", "board"], .openTasks)
        add("action:create-task", "Create Task", .actions, .lucide(.plus),
            keywords: ["new task", "add task", "todo"], .createTask)
        add("action:stop-sessions-and-server", "Stop Sessions and Server", .actions, .lucide(.power),
            sublabel: src.sessions.isEmpty ? nil : "\(src.sessions.count) running",
            keywords: ["stop", "quit", "shutdown", "server", "end", "kill"], .stopServer)
        add("action:toggle-layout", "Toggle Layout (Grid/Tabs)", .actions, .lucide(.layoutDashboard),
            keywords: ["grid", "tabs", "layout", "switch", "view"], .toggleLayout)
        add("action:welcome-guide", "Show Welcome Guide", .actions, .lucide(.bookOpen),
            keywords: ["welcome", "guide", "help", "onboarding", "tour"], .welcomeGuide)
        add("action:new-terminal", "New Terminal Session", .actions, .lucide(.terminal), shortcut: "Ctrl+`",
            keywords: ["shell", "terminal", "zsh", "bash"], .newTerminal)
        add("action:copy-mcp-url", "Copy MCP Server URL", .actions, .lucide(.plug),
            keywords: ["mcp", "api", "integration", "claude", "cursor", "server", "url"], .copyMCPURL)
        add("action:manage-worktrees", "Manage Worktrees", .actions, .lucide(.hardDrive),
            keywords: ["worktree", "disk", "space", "cleanup", "clean", "prune", "size", "node_modules"], .settings(.worktrees))
        add("action:manage-ssh", "SSH & Hosts", .actions, .lucide(.server),
            keywords: ["ssh", "remote", "host", "server", "key", "credential", "vault"], .settings(.ssh))

        for s in src.sessions where !src.claimed.contains(s.id) {
            let kind = s.agentType == .shell ? "Shell" : s.agentType.displayName
            add("terminal:\(s.id)", displayName(s), .terminals, .agent(s.agentType),
                keywords: [kind, s.projectName, s.status.rawValue], .focus(s.id))
        }
        for card in src.cards {
            let owner = src.sessions.first { $0.id == card.sessionId }
            add("card:\(card.id)", card.name, .terminals, .file(card.name),
                keywords: [card.path, owner?.projectName ?? "", owner?.branch ?? ""].filter { !$0.isEmpty }, .focusCard(card.id))
        }

        for r in src.recent where r.canResumeExact {
            let project = projectName(for: r.projectPath, in: src.projects)
            add("recent:\(r.sessionId)", r.display.isEmpty ? "Untitled session" : r.display, .recent, .agent(r.agentType),
                sublabel: "\(project) · \(timeAgo(r.timestamp, now: src.now)) · \(activity(r))",
                keywords: ["resume", "recent", "history", project, r.agentType.rawValue], .resume(r))
        }

        add("project:all", "All Projects", .projects, .lucide(.monitor), keywords: ["show all", "clear filter"], .allProjects)
        for p in src.projects {
            add("project:\(p.name)", p.name, .projects, keywords: [p.path], .project(p.name))
        }

        for wf in src.workflows {
            let projects = wf.nodes.filter { $0.type == .launchAgent }
                .compactMap { $0.config["projectName"]?.stringValue }.filter { !$0.isEmpty }
            add("workflow:\(wf.id)", wf.name, .workflows, .lucide(.workflow), keywords: projects, .runWorkflow(wf.id))
        }

        let local = src.projects.filter(isLocal)
        func worktrees(_ p: Project) -> [Worktree] {
            src.gitRepos[p.path] == true ? (src.worktrees[p.path] ?? []).filter { !$0.isMain } : []
        }
        func branchNote(_ wt: Worktree) -> String { wt.branch == wt.name ? "Existing worktree" : "Branch: \(wt.branch)" }
        for agent in src.installedAgents {
            for p in local {
                add("quicklaunch:\(agent.rawValue):\(p.name)", "\(agent.displayName) on \(p.name)", .quicklaunch,
                    .agent(agent), keywords: ["launch", "start", "run"], .launch(agent, project: p.name, worktree: nil))
                guard src.gitRepos[p.path] == true else { continue }
                for wt in worktrees(p) {
                    add("quicklaunch:\(agent.rawValue):\(p.name):wt:\(wt.path)", "\(agent.displayName) on \(p.name) › \(wt.name)",
                        .quicklaunch, .agent(agent), sublabel: branchNote(wt),
                        keywords: ["launch", "start", "run", "worktree", "branch", wt.name, wt.branch],
                        .launch(agent, project: p.name, worktree: wt))
                }
                add("quicklaunch:\(agent.rawValue):\(p.name):worktree", "\(agent.displayName) on \(p.name) (new worktree)",
                    .quicklaunch, .agent(agent), sublabel: "Isolated worktree from current branch",
                    keywords: ["launch", "start", "run", "worktree", "branch", "isolated", "fork", "new"],
                    .launchNewWorktree(agent, project: p.name))
            }
        }
        for p in local {
            add("terminal:project:\(p.name)", "Terminal in \(p.name)", .quicklaunch, .lucide(.terminal),
                keywords: ["terminal", "shell", "launch", p.name], .terminalIn(project: p.name, worktree: nil))
            for wt in worktrees(p) {
                add("terminal:project:\(p.name):wt:\(wt.path)", "Terminal in \(p.name) › \(wt.name)", .quicklaunch,
                    .lucide(.terminal), sublabel: branchNote(wt),
                    keywords: ["terminal", "shell", "launch", "worktree", wt.name, wt.branch],
                    .terminalIn(project: p.name, worktree: wt))
            }
        }

        let filters: [(StatusFilter, String, String)] = [
            (.all, "Show All", "⌘1"), (.running, "Show Running", "⌘2"), (.waiting, "Show Waiting", "⌘3"),
            (.idle, "Show Idle", "⌘4"), (.error, "Show Errors", "⌘5"),
        ]
        for (f, label, key) in filters {
            add("filter:status:\(f.rawValue)", label, .filter, .lucide(.filter), shortcut: key,
                keywords: ["filter", "status"], .statusFilter(f))
        }
        let sorts: [(SortMode, String)] = [(.manual, "Sort: Manual"), (.created, "Sort: Created"), (.recent, "Sort: Recent Activity")]
        for (m, label) in sorts {
            add("filter:sort:\(m.rawValue)", label, .filter, .lucide(.arrowUpDown), keywords: ["sort", "order"], .sort(m))
        }
        return out
    }

    /// terminal-display.ts getDisplayName.
    static func displayName(_ s: TerminalSession) -> String {
        let name = s.displayName?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return name.isEmpty ? s.projectName : name
    }

    /// session-utils.ts formatRecentSessionActivity: "3 prompts", "1 entry", "2 entries".
    static func activity(_ r: RecentSession) -> String {
        let label = r.activityLabel
        guard r.activityCount != 1 else { return "1 \(label)" }
        return "\(r.activityCount) " + (label.hasSuffix("y") ? label.dropLast() + "ies" : label + "s")
    }

    /// getProjectHostIds: no hosts means local.
    static func isLocal(_ p: Project) -> Bool {
        guard let ids = p.hostIds, !ids.isEmpty else { return true }
        return ids.contains("local")
    }

    /// session-utils.ts resolveProjectName: the project at that path or one of its managed worktrees, else the folder's name.
    static func projectName(for path: String, in projects: [Project]) -> String {
        let target = comparable(path)
        let match = projects.first { p in
            let base = comparable(p.path)
            if base == target { return true }
            var parts = base.split(separator: "/").map(String.init)
            guard let name = parts.popLast() else { return false }
            let parent = (base.hasPrefix("/") ? "/" : "") + parts.joined(separator: "/")
            let prefix = (parent.isEmpty ? "." : parent == "/" ? "" : parent) + "/.vorn-worktrees/" + name
            return target.hasPrefix(prefix + "/")
        }
        if let match { return match.name }
        let trimmed = path.replacingOccurrences(of: "\\", with: "/").trimmingSuffix("/")
        return trimmed.split(separator: "/").last.map(String.init) ?? "untitled"
    }

    private static func comparable(_ p: String) -> String {
        let n = p.replacingOccurrences(of: "\\", with: "/").trimmingSuffix("/")
        return n.isEmpty ? "/" : n.lowercased()
    }
}

private extension String {
    func trimmingSuffix(_ c: Character) -> String {
        var s = Substring(self)
        while s.last == c { s = s.dropLast() }
        return String(s)
    }
}

// MARK: Search

/// CommandPalette.tsx's matching, ranking, grouping and arrow-key arithmetic.
public enum PaletteSearch {
    /// scoreMatch: exact 100, prefix 80, a word's prefix 60, anywhere in the label 40, in the keywords 20.
    public static func score(_ query: String, label: String, keywords: [String]) -> Int {
        let q = query.lowercased()
        let l = label.lowercased()
        if l == q { return 100 }
        if l.hasPrefix(q) { return 80 }
        if l.split(whereSeparator: \.isWhitespace).contains(where: { $0.hasPrefix(q) }) { return 60 }
        if l.contains(q) { return 40 }
        if keywords.joined(separator: " ").lowercased().contains(q) { return 20 }
        return 0
    }

    static func trimmed(_ query: String) -> String { query.trimmingCharacters(in: .whitespacesAndNewlines) }

    /// The selectable list: grouped by category without a query (quick launch hidden), ranked by score with one.
    public static func results(_ commands: [PaletteCommand], query: String) -> [PaletteCommand] {
        let q = trimmed(query)
        guard !q.isEmpty else {
            let shown = commands.filter { $0.category != .quicklaunch }
            return PaletteCategory.allCases.flatMap { cat in shown.filter { $0.category == cat } }
        }
        var ranked: [(offset: Int, score: Int)] = []
        for (i, cmd) in commands.enumerated() {
            let s = score(q, label: cmd.label, keywords: cmd.keywords)
            if s > 0 { ranked.append((i, s)) }
        }
        ranked.sort { a, b in a.score != b.score ? a.score > b.score : a.offset < b.offset }
        return ranked.map { commands[$0.offset] }
    }

    public struct Section: Identifiable {
        /// No header while searching: the results are one ranked list.
        public var category: PaletteCategory?
        public var items: [(index: Int, command: PaletteCommand)]
        public var id: String { category?.rawValue ?? "results" }
    }

    /// The list as drawn: a header per non-empty category, or one flat run of results; indices count commands only.
    public static func sections(_ results: [PaletteCommand], query: String) -> [Section] {
        let indexed = results.enumerated().map { (index: $0.offset, command: $0.element) }
        guard trimmed(query).isEmpty else { return indexed.isEmpty ? [] : [Section(category: nil, items: indexed)] }
        return PaletteCategory.allCases.compactMap { cat in
            let items = indexed.filter { $0.command.category == cat }
            return items.isEmpty ? nil : Section(category: cat, items: items)
        }
    }

    /// ArrowDown and ArrowUp, wrapping at both ends.
    public static func step(_ index: Int, by delta: Int, count: Int) -> Int {
        let n = max(count, 1)
        return delta > 0 ? (index + 1) % n : (index - 1 + count) % n
    }

    /// highlightMatch: the label split around the first case-insensitive match of the raw query.
    public static func highlight(_ text: String, query: String) -> (before: String, match: String, after: String)? {
        guard !trimmed(query).isEmpty, let r = text.range(of: query, options: .caseInsensitive) else { return nil }
        return (String(text[..<r.lowerBound]), String(text[r]), String(text[r.upperBound...]))
    }
}

// MARK: Running

/// Runs a palette command against the shell's stores (each `onExecute` in CommandPalette.tsx).
@MainActor
struct PaletteRunner {
    let model: AppModel
    let store: VornStore
    let actions: ShellActions
    let features: FeatureStores

    /// The palette's sources, read from the stores.
    func sources(recent: [RecentSession]) -> PaletteSources {
        var s = PaletteSources()
        s.sessions = store.sessions
        s.claimed = model.panes.claimedTerminalIDs
        s.cards = model.panes.cardOrder.compactMap { id in
            model.panes.editorCards[id].map { PaletteCard(id: id, sessionId: $0.sessionId, path: $0.filePath) }
        }
        s.recent = recent
        s.projects = store.projects
        s.workflows = features.workflows.workflows
        s.installedAgents = store.installedAgents
        s.gitRepos = store.gitRepos
        s.worktrees = store.worktrees
        return s
    }

    /// executeCommand: the palette closes, then the command runs.
    func run(_ command: PaletteCommand) {
        if model.overlay == .commandPalette { model.overlay = nil }
        perform(command.action)
    }

    private var resolvedProject: Project? {
        actions.activeProject ?? store.projects.first { $0.workspace == model.activeWorkspace }
    }

    func perform(_ action: PaletteAction) {
        switch action {
        case .newSession: actions.newSession(resolvedProject, actions.activeWorktree)
        case .openSettings: model.settingsOpen = true
        case .toggleSidebar: model.sidebarOpen.toggle()
        case .shortcuts: model.overlay = .shortcuts
        case .addProject: model.overlay = .addProject(editing: nil)
        case .addWorkflow:
            model.mainViewMode = .workflows
            features.workflows.newWorkflow()
        case .openTasks: model.mainViewMode = .tasks
        case .createTask:
            model.mainViewMode = .tasks
            features.tasks.openNewTask()
        case .stopServer: actions.stopSessionsAndServer()
        case .toggleLayout: actions.setLayout(model.layoutMode(store) == .grid ? .tabs : .grid)
        case .welcomeGuide: model.openOnboarding()
        case .newTerminal: actions.newTerminal(resolvedProject, actions.activeWorktree)
        case .copyMCPURL:
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(PaletteCommands.mcpURL, forType: .string)
        case .settings(let category):
            features.settings.category = category
            model.settingsOpen = true
        case .focus(let id):
            model.mainViewMode = .sessions
            model.minimizedIDs.remove(id)
            model.setFocused(id)
        case .focusCard(let id):
            model.mainViewMode = .sessions
            model.minimizedIDs.remove(id)
            model.focus(id)
        case .resume(let r): actions.resumeRecent(r)
        case .allProjects: model.select(project: nil)
        case .project(let name): model.select(project: name)
        case .runWorkflow(let id):
            if let wf = features.workflows.workflows.first(where: { $0.id == id }) { features.workflows.runNow(wf) }
        case .launch(let agent, let name, let wt):
            if let p = store.project(named: name) { actions.newSession(p, wt, agent: agent) }
        case .launchNewWorktree(let agent, let name):
            if let p = store.project(named: name) { actions.newWorktreeSession(p, agent: agent) }
        case .terminalIn(let name, let wt):
            if let p = store.project(named: name) { actions.newTerminal(p, wt) }
        case .statusFilter(let f): model.statusFilter = f
        case .sort(let m): model.sortMode = m
        }
    }
}

// MARK: View

/// CommandPalette.tsx (⌘K): a search field over every command, grouped until you type.
struct CommandPaletteView: View {
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions
    let features: FeatureStores

    /// The query an offscreen render opens with.
    @MainActor static var renderQuery: String?

    static let width: CGFloat = 560
    static let maxHeight: CGFloat = 420

    @State private var query = CommandPaletteView.renderQuery ?? ""
    @State private var active = 0
    @State private var recent: [RecentSession] = []
    @State private var shown = false
    @FocusState private var focused: Bool

    private var runner: PaletteRunner { PaletteRunner(model: model, store: store, actions: actions, features: features) }

    var body: some View {
        let commands = PaletteCommands.build(runner.sources(recent: recent))
        let results = PaletteSearch.results(commands, query: query)
        let sections = PaletteSearch.sections(results, query: query)
        let searching = !PaletteSearch.trimmed(query).isEmpty
        VStack(spacing: 0) {
            searchRow(results)
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0, pinnedViews: .sectionHeaders) {
                        if results.isEmpty {
                            Text("No matching commands")
                                .font(.ui(14))
                                .foregroundStyle(Theme.gray600)
                                .frame(maxWidth: .infinity)
                                .padding(.horizontal, 16)
                                .padding(.vertical, 32)
                        }
                        ForEach(sections) { section in
                            Section {
                                ForEach(section.items, id: \.command.id) { item in
                                    row(item.command, index: item.index, searching: searching).id(item.command.id)
                                }
                            } header: {
                                if let cat = section.category { header(cat.label) }
                            }
                        }
                    }
                    .padding(.vertical, 4)
                }
                .scrollIndicators(.never)
                .onChange(of: active) { _, i in
                    if results.indices.contains(i) { proxy.scrollTo(results[i].id) }
                }
            }
            .frame(maxHeight: Self.maxHeight - 45 - 33 - 2)
            .fixedSize(horizontal: false, vertical: true)
            footer
        }
        .frame(width: Self.width)
        .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusXl))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusXl).strokeBorder(Theme.white(0.08), lineWidth: 1))
        .clipShape(RoundedRectangle(cornerRadius: Theme.radiusXl))
        .shadow(color: .black.opacity(0.5), radius: 24, y: 12)
        .opacity(shown ? 1 : 0)
        .scaleEffect(shown ? 1 : 0.98)
        .offset(y: shown ? 0 : -8)
        .onAppear {
            focused = Self.renderQuery == nil
            withAnimation(.easeOut(duration: 0.15)) { shown = true }
        }
        .onChange(of: query) { _, _ in active = 0 }
        .task {
            if let fixture = model.recentFixture {
                recent = fixture
            } else {
                recent = (try? await store.recentSessions(projectPath: nil)) ?? []
            }
            for p in store.projects where PaletteCommands.isLocal(p) { await store.loadGitInfo(p.path) }
            if features.workflows.workflows.isEmpty { await features.workflows.reload() }
        }
    }

    private func searchRow(_ results: [PaletteCommand]) -> some View {
        HStack(spacing: 12) {
            LucideIcon(.search, size: 16, strokeWidth: 1.5).foregroundStyle(Theme.gray500)
            TextField("", text: $query, prompt: Text("Type a command...").foregroundStyle(Theme.gray600))
                .textFieldStyle(.plain)
                .font(.ui(14))
                .foregroundStyle(Theme.gray200)
                .focused($focused)
                .onKeyPress(.downArrow) {
                    active = PaletteSearch.step(active, by: 1, count: results.count)
                    return .handled
                }
                .onKeyPress(.upArrow) {
                    active = PaletteSearch.step(active, by: -1, count: results.count)
                    return .handled
                }
                .onKeyPress(.return) {
                    if results.indices.contains(active) { runner.run(results[active]) }
                    return .handled
                }
                .onKeyPress(.escape) {
                    model.overlay = nil
                    return .handled
                }
            PaletteKey("ESC", color: Theme.gray600, horizontal: 6)
        }
        .frame(height: 20)
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }
    }

    private func header(_ label: String) -> some View {
        Text(label.uppercased())
            .font(.ui(11, weight: .medium))
            .tracking(0.55)
            .foregroundStyle(Theme.gray500)
            .lineHeight(16.5, size: 11)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .background(Theme.surfaceOverlay)
    }

    private func row(_ cmd: PaletteCommand, index: Int, searching: Bool) -> some View {
        let isActive = index == active
        return Button { runner.run(cmd) } label: {
            HStack(spacing: 12) {
                icon(cmd.icon)
                    .foregroundStyle(Theme.gray500)
                    .frame(width: 20)
                VStack(alignment: .leading, spacing: 0) {
                    label(cmd.label, searching: searching)
                        .font(.ui(13))
                        .foregroundStyle(isActive ? Color.white : Theme.gray300)
                        .lineLimit(1)
                        .lineHeight(19.5, size: 13)
                    if let sub = cmd.sublabel {
                        Text(sub)
                            .font(.ui(10))
                            .foregroundStyle(Theme.gray600)
                            .lineLimit(1)
                            .lineHeight(15, size: 10)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                if let key = cmd.shortcut { PaletteKey(key, color: Theme.gray500, horizontal: 6) }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(isActive ? Theme.white(0.06) : .clear)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { if $0 { active = index } }
    }

    private func label(_ text: String, searching: Bool) -> Text {
        guard searching, let parts = PaletteSearch.highlight(text, query: query) else { return Text(text) }
        var s = AttributedString(parts.before)
        var m = AttributedString(parts.match)
        m.foregroundColor = .white
        m.font = .ui(13, weight: .semibold)
        s.append(m)
        s.append(AttributedString(parts.after))
        return Text(s)
    }

    @ViewBuilder private func icon(_ icon: PaletteIcon) -> some View {
        switch icon {
        case .none: Color.clear.frame(width: 14, height: 14)
        case .lucide(let glyph): LucideIcon(glyph, size: 14, strokeWidth: 1.5)
        case .agent(let agent): AgentIcon(agent, size: 14)
        case .file(let name): FileTypeIcon(name: name, size: 14)
        }
    }

    private var footer: some View {
        HStack(spacing: 16) {
            hint("↑↓", "navigate")
            hint("↵", "select")
            hint("esc", "close")
            Spacer(minLength: 0)
        }
        .font(.ui(11))
        .foregroundStyle(Theme.gray600)
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
    }

    private func hint(_ key: String, _ label: String) -> some View {
        HStack(spacing: 4) {
            PaletteKey(key, color: Theme.gray600, horizontal: 4, vertical: 0)
            Text(label)
        }
    }
}

/// The palette's `<kbd>`: mono 10px on white/0.04 with a hairline.
private struct PaletteKey: View {
    let text: String
    let color: Color
    let horizontal: CGFloat
    let vertical: CGFloat

    init(_ text: String, color: Color, horizontal: CGFloat, vertical: CGFloat = 2) {
        self.text = text
        self.color = color
        self.horizontal = horizontal
        self.vertical = vertical
    }

    var body: some View {
        Text(text)
            .font(.system(size: 10, design: .monospaced))
            .foregroundStyle(color)
            .padding(.horizontal, horizontal)
            .padding(.vertical, vertical)
            .background(RoundedRectangle(cornerRadius: Theme.radius).fill(Theme.white(0.04)))
            .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.hairline, lineWidth: 1))
    }
}
