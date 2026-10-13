import SwiftUI
import VornCore
import VornSettings
import VornUI
import VornWorkflows

/// The left sidebar (ProjectSidebar.tsx): workspace, view switch, projects, footer.
struct Sidebar: View {
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions
    let features: FeatureStores

    @Environment(\.terminalSnapshot) private var snapshot

    var body: some View {
        VStack(spacing: 0) {
            SidebarHeader(model: model, store: store)
            if snapshot {
                list.frame(minHeight: 0, maxHeight: .infinity, alignment: .top).clipped().layoutPriority(-1)
            } else {
                ScrollView { list }.scrollIndicators(.never)
            }
            SidebarFooter(model: model, store: store, settings: features.settings)
        }
        .frame(width: model.sidebarWidth)
        .frame(maxHeight: .infinity)
        .background(Theme.surfacePanel)
        .overlay(alignment: .trailing) {
            Rectangle().fill(Theme.hairline).frame(width: 1)
        }
        .overlay(alignment: .trailing) { ResizeHandle(width: $model.sidebarWidth) }
    }

    private var list: some View {
        VStack(alignment: .leading, spacing: 2) {
            switch model.mainViewMode {
            case .sessions:
                if model.sidebarViewMode == .sessionsFlat {
                    FlatSessionsSection(model: model, store: store, actions: actions)
                } else {
                    ProjectsSection(model: model, store: store, actions: actions)
                }
            case .tasks: TasksProjectsSection(model: model, store: store)
            case .workflows: WorkflowsSidebarSection(store: features.workflows)
            }
        }
        .padding(.horizontal, 12)
        .padding(.bottom, 8)
    }
}

/// The 6pt drag strip on the sidebar's right edge.
private struct ResizeHandle: View {
    @Binding var width: CGFloat
    @State private var start: CGFloat?

    var body: some View {
        Hovering { hover in
            Rectangle()
                .fill(hover || start != nil ? Theme.white(0.08) : .clear)
                .frame(width: 6)
                .contentShape(Rectangle())
                .gesture(DragGesture(minimumDistance: 0, coordinateSpace: .global)
                    .onChanged { g in
                        let base = start ?? width
                        if start == nil { start = width }
                        width = min(max(base + g.translation.width, Theme.sidebarMinWidth), Theme.sidebarMaxWidth)
                    }
                    .onEnded { _ in start = nil })
                .onHover { inside in
                    if inside { NSCursor.resizeLeftRight.push() } else { NSCursor.pop() }
                }
        }
        .offset(x: 3)
    }
}

// MARK: Header

struct SidebarHeader: View {
    @Bindable var model: AppModel
    let store: VornStore

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 0) {
                WorkspaceSwitcher(model: model, store: store)
                IconButton(.panelLeft, size: 16, padding: 4, help: "Toggle sidebar") { model.sidebarOpen.toggle() }
                    .tooltip("Toggle sidebar", shortcut: "⌘B")
            }
            .padding(.leading, Theme.trafficLightPad)
            .padding(.trailing, 12)
            .frame(height: Theme.toolbarHeight)

            HStack(spacing: 4) {
                ForEach(MainViewMode.allCases, id: \.self) { mode in
                    ViewModeButton(mode: mode, active: model.mainViewMode == mode) { model.mainViewMode = mode }
                }
                Spacer(minLength: 0)
            }
            .padding(.vertical, 8)
            .padding(.horizontal, 12)
        }
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }
    }
}

extension MainViewMode {
    var icon: LucideGlyph {
        switch self {
        case .sessions: .monitor
        case .tasks: .listTodo
        case .workflows: .workflow
        }
    }
}

private struct ViewModeButton: View {
    let mode: MainViewMode
    let active: Bool
    let action: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                HStack(spacing: 6) {
                    LucideIcon(mode.icon, size: 14)
                    if active { Text(mode.label).font(.ui(12, weight: .medium)) }
                }
                .foregroundStyle(active ? Color.white : hover ? Theme.gray300 : Theme.gray500)
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(RoundedRectangle(cornerRadius: Theme.radiusLg)
                    .fill(active ? Theme.white(0.1) : hover ? Theme.white(0.04) : .clear))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .tooltip(mode.label, shortcut: mode.shortcut)
    }
}

/// The workspace picker at the top of the sidebar (WorkspaceSwitcher.tsx).
private struct WorkspaceSwitcher: View {
    @Bindable var model: AppModel
    let store: VornStore

    private var current: Workspace {
        store.workspaces.first { $0.id == model.activeWorkspace } ?? .personal
    }

    var body: some View {
        Menu {
            ForEach(store.workspaces) { ws in
                Button(ws.name) {
                    model.activeWorkspace = ws.id
                    model.select(project: nil)
                }
            }
        } label: {
            Hovering { hover in
                HStack(spacing: 8) {
                    WorkspaceIcon(icon: current.icon, color: current.iconColor, size: 14)
                    Text(current.name).font(.ui(13, weight: .medium)).lineLimit(1)
                    Spacer(minLength: 0)
                    LucideIcon(.chevronDown, size: 12).foregroundStyle(Theme.gray500)
                }
                .foregroundStyle(Theme.gray200)
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(hover ? Theme.white(0.06) : .clear))
                .contentShape(Rectangle())
            }
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
    }
}

// MARK: Projects

struct ProjectsSection: View {
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions

    private var projects: [Project] { store.projects.filter { $0.workspace == model.activeWorkspace } }

    private var workspaceSessionCount: Int {
        let names = Set(projects.map(\.name))
        return store.sessions.filter { names.contains($0.projectName) }.count
    }

    /// ProjectsSection.tsx: the "active" filter, then the chosen order.
    private var sorted: [Project] {
        var list = projects
        if model.worktreeFilter == .active { list = list.filter { !store.sessions(in: $0).isEmpty } }
        switch model.projectSort {
        case .manual: return list
        case .name: return list.sorted { $0.name.localizedCompare($1.name) == .orderedAscending }
        case .recent:
            func last(_ p: Project) -> Double { store.sessions(in: p).map { store.lastActivity[$0.id] ?? 0 }.max() ?? 0 }
            return list.sorted { last($0) > last($1) }
        }
    }

    var body: some View {
        SectionHeader(title: "Projects", open: $model.projectsOpen) {
            SidebarOptionsButton(model: model)
            IconButton(.folderPlus, size: 13, strokeWidth: 1.5, padding: 2, radius: Theme.radius,
                       color: Theme.gray600, hoverFill: Theme.white(0.08), help: "Add project") {
                model.contextMenu = nil
                model.overlay = .addProject(editing: nil)
            }
        }
        if model.projectsOpen {
            NavRow(icon: .layers, label: "All Projects", badge: workspaceSessionCount,
                   active: model.activeProject == nil) { model.select(project: nil) }
            if projects.isEmpty {
                Text("No projects")
                    .font(.ui(13))
                    .foregroundStyle(Theme.gray600)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 4)
            }
            ForEach(sorted) { p in
                ProjectItem(project: p, model: model, store: store, actions: actions)
                    .padding(.top, 2)
            }
        }
    }
}

/// The projects that scope the task board (TasksProjectsSection.tsx).
struct TasksProjectsSection: View {
    @Bindable var model: AppModel
    let store: VornStore
    @State private var open = true

    var body: some View {
        let projects = store.projects.filter { $0.workspace == model.activeWorkspace }
        SectionHeader(title: "Projects", open: $open) {}
        if open {
            NavRow(icon: .layers, label: "All Projects", active: model.activeProject == nil) { model.select(project: nil) }
            if projects.isEmpty {
                Text("No projects")
                    .font(.ui(13))
                    .foregroundStyle(Theme.gray600)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 4)
            }
            ForEach(projects) { p in
                NavRow(label: p.name, active: model.activeProject == p.name) {
                    model.select(project: p.name)
                } icon: {
                    ProjectIcon(icon: p.icon, color: p.iconColor, size: 14)
                }
            }
        }
    }
}

/// `pt-3 pb-1.5`: chevron, uppercase title, trailing actions.
struct SectionHeader<Actions: View>: View {
    let title: String
    @Binding var open: Bool
    @ViewBuilder let actions: () -> Actions

    var body: some View {
        HStack(spacing: 6) {
            Button { open.toggle() } label: {
                HStack(spacing: 6) {
                    LucideIcon(.chevronRight, size: 10)
                        .foregroundStyle(Theme.gray600)
                        .rotationEffect(.degrees(open ? 90 : 0))
                    Text(title.uppercased())
                        .font(.ui(11, weight: .medium))
                        .tracking(0.55)
                        .foregroundStyle(Theme.gray500)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            Spacer(minLength: 0)
            HStack(spacing: 2) { actions() }
        }
        .padding(.top, 12)
        .padding(.bottom, 6)
    }
}

/// `px-2 py-1.5 rounded-md text-[13px]`: a sidebar row with icon, label and count.
struct NavRow<Icon: View>: View {
    let icon: Icon
    let label: String
    var badge: Int = 0
    let active: Bool
    let action: () -> Void

    init(label: String, badge: Int = 0, active: Bool, action: @escaping () -> Void, @ViewBuilder icon: () -> Icon) {
        self.icon = icon()
        self.label = label
        self.badge = badge
        self.active = active
        self.action = action
    }

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                HStack(spacing: 8) {
                    icon
                    Text(label).lineLimit(1)
                    Spacer(minLength: 0)
                    if badge > 0 {
                        Text("\(badge)").font(.ui(12)).foregroundStyle(Theme.gray500)
                    }
                }
                .font(.ui(13))
                .foregroundStyle(active || hover ? Color.white : Theme.gray300)
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
                .background(RoundedRectangle(cornerRadius: Theme.radiusMd)
                    .fill(active ? Theme.white(0.08) : hover ? Theme.white(0.04) : .clear))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }
}

extension NavRow where Icon == LucideIcon {
    init(icon: LucideGlyph, label: String, badge: Int = 0, active: Bool, action: @escaping () -> Void) {
        self.init(label: label, badge: badge, active: active, action: action) {
            LucideIcon(icon, size: 14, strokeWidth: 1.5)
        }
    }
}

/// `p-0.5 rounded text-gray-500 hover:text-white hover:bg-white/[0.08]`.
private func rowAction(_ icon: LucideGlyph, _ help: String, stroke: CGFloat = 1.5, color: Color = Theme.gray500,
                       hoverColor: Color = .white, action: @escaping () -> Void) -> some View {
    IconButton(icon, size: 14, strokeWidth: stroke, padding: 2, radius: Theme.radius, color: color,
               hoverColor: hoverColor, hoverFill: Theme.white(0.08), help: help, action: action)
}

struct ProjectItem: View {
    let project: Project
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions
    @State private var hover = false
    /// Branches whose sessions are folded away, keyed by worktree path ("__main__" for the main tree).
    @State private var collapsed: Set<String> = []

    private var sessions: [TerminalSession] { store.sessions(in: project) }
    private var isGit: Bool { store.gitRepos[project.path] == true }
    private var worktrees: [Worktree] { store.worktrees[project.path] ?? [] }
    private var expanded: Bool { model.expandedProjects[project.name] ?? !sessions.isEmpty }
    private var active: Bool { model.activeProject == project.name && model.activeWorktreePath == nil }
    private var mode: SidebarViewMode { model.sidebarViewMode }
    private var showSessions: Bool { mode == .worktreesSessions }

    /// ProjectItem.tsx's sortedWorktrees: filter, then name or most recent activity.
    private var sortedWorktrees: [Worktree] {
        var list = worktrees.filter { !$0.isMain }
        if model.worktreeFilter == .active {
            list = list.filter { wt in sessions.contains { $0.worktreePath == wt.path } }
        }
        let byName: (Worktree, Worktree) -> Bool = { $0.name.localizedCompare($1.name) == .orderedAscending }
        guard model.worktreeSort == .recent else { return list.sorted(by: byName) }
        func last(_ wt: Worktree) -> Double {
            sessions.filter { $0.worktreePath == wt.path }.map { store.lastActivity[$0.id] ?? 0 }.max() ?? 0
        }
        return list.sorted { last($0) != last($1) ? last($0) > last($1) : byName($0, $1) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            row
            if expanded { body(expanded: ()) .padding(.leading, 8) }
        }
    }

    private var row: some View {
        HStack(spacing: 8) {
            ZStack {
                if hover {
                    Button { model.expandedProjects[project.name] = !expanded } label: {
                        LucideIcon(.chevronRight, size: 12, strokeWidth: 2.5)
                            .foregroundStyle(Theme.gray500)
                            .rotationEffect(.degrees(expanded ? 90 : 0))
                    }
                    .buttonStyle(.plain)
                } else {
                    ProjectIcon(icon: project.icon, color: project.iconColor, size: 14)
                }
            }
            .frame(width: 14, height: 14)
            Text(project.name).lineLimit(1).truncationMode(.tail)
            Spacer(minLength: 0)
            if hover {
                HStack(spacing: 2) {
                    rowAction(.terminal, "New terminal") { actions.newTerminal(project, nil) }
                    rowAction(.plus, "New session", stroke: 2) { actions.newSession(project, nil) }
                    if isGit {
                        rowAction(.folderGit2, "New worktree", color: Theme.inkSecondary) { actions.newWorktree(project) }
                    }
                    rowAction(.ellipsis, "More", stroke: 2) {
                        model.contextMenu = nil
                        model.overlay = model.overlay == .projectMenu(project.name) ? nil : .projectMenu(project.name)
                    }
                }
            } else if mode == .worktrees, !sessions.isEmpty {
                Text("\(sessions.count)").font(.ui(12)).foregroundStyle(Theme.gray600)
            }
        }
        .font(.ui(13))
        .foregroundStyle(active || hover ? Color.white : Theme.gray300)
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .frame(height: 30)
        .background(RoundedRectangle(cornerRadius: Theme.radiusMd)
            .fill(active ? Theme.white(0.08) : hover ? Theme.white(0.04) : .clear))
        .contentShape(Rectangle())
        .onHover { hover = $0 }
        .onTapGesture {
            model.select(project: project.name)
            if model.expandedProjects[project.name] == nil, sessions.isEmpty { model.expandedProjects[project.name] = true }
        }
        .controlAnchor("project:\(project.name)")
    }

    @ViewBuilder private func body(expanded: Void) -> some View {
        if mode == .sessions || !isGit {
            if mode == .sessions || showSessions {
                if sessions.isEmpty {
                    Text("No active sessions")
                        .font(.ui(11))
                        .foregroundStyle(Theme.gray600)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                }
                ForEach(sessions) { s in
                    SessionItem(session: s, model: model, store: store, actions: actions, showBranch: true)
                }
            }
        } else {
            let main = worktrees.first { $0.isMain }
            let mainSessions = sessions.filter { $0.isWorktree != true }
            if let main {
                MainWorktreeRow(project: project, branch: main.branch, model: model, actions: actions, worktree: main,
                                count: showSessions ? 0 : mainSessions.count,
                                sessionsOpen: showSessions ? !collapsed.contains("__main__") : nil) { toggle("__main__") }
                if showSessions && !collapsed.contains("__main__") {
                    ForEach(mainSessions) { s in
                        SessionItem(session: s, model: model, store: store, actions: actions, showBranch: false)
                            .padding(.leading, 16)
                    }
                }
            }
            ForEach(sortedWorktrees) { wt in
                let wtSessions = sessions.filter { $0.worktreePath == wt.path }
                WorktreeItem(project: project, worktree: wt, model: model, actions: actions,
                             count: showSessions ? 0 : wtSessions.count,
                             sessionsOpen: showSessions ? !collapsed.contains(wt.path) : nil) { toggle(wt.path) }
                if showSessions && !collapsed.contains(wt.path) {
                    ForEach(wtSessions) { s in
                        SessionItem(session: s, model: model, store: store, actions: actions, showBranch: false)
                            .padding(.leading, 16)
                    }
                }
            }
        }
    }

    private func toggle(_ key: String) {
        if collapsed.contains(key) { collapsed.remove(key) } else { collapsed.insert(key) }
    }
}

private struct MainWorktreeRow: View {
    let project: Project
    let branch: String
    @Bindable var model: AppModel
    let actions: ShellActions
    let worktree: Worktree?
    let count: Int
    /// Whether its sessions show beneath it, when the view lists them.
    let sessionsOpen: Bool?
    let toggleSessions: () -> Void
    @State private var hover = false

    private var active: Bool {
        model.activeProject == project.name && model.activeWorktreePath == AppModel.mainWorktree
    }

    var body: some View {
        HStack(spacing: 8) {
            if let open = sessionsOpen, hover {
                Button(action: toggleSessions) {
                    LucideIcon(.chevronRight, size: 12, strokeWidth: 2.5)
                        .foregroundStyle(Theme.gray500)
                        .rotationEffect(.degrees(open ? 90 : 0))
                        .frame(width: 14, height: 14)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Toggle sessions")
            } else {
                LucideIcon(.gitBranch, size: 14, strokeWidth: 1.5).foregroundStyle(Theme.gray500)
            }
            Text(branch).lineLimit(1)
            Spacer(minLength: 0)
            if hover {
                HStack(spacing: 2) {
                    rowAction(.terminal, "New terminal") { actions.newTerminal(project, worktree) }
                    rowAction(.plus, "New session", stroke: 2) { actions.newSession(project, worktree) }
                }
            } else if count > 0 {
                Text("\(count)").font(.ui(12)).foregroundStyle(Theme.gray600)
            }
        }
        .font(.ui(13))
        .foregroundStyle(active || hover ? Color.white : Theme.gray400)
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .frame(height: 30)
        .background(RoundedRectangle(cornerRadius: Theme.radiusMd)
            .fill(active ? Theme.white(0.08) : hover ? Theme.white(0.04) : .clear))
        .contentShape(Rectangle())
        .onHover { hover = $0 }
        .onTapGesture {
            model.toggle(worktree: AppModel.mainWorktree, in: project.name)
        }
    }
}

private struct WorktreeItem: View {
    let project: Project
    let worktree: Worktree
    @Bindable var model: AppModel
    let actions: ShellActions
    let count: Int
    let sessionsOpen: Bool?
    let toggleSessions: () -> Void
    @State private var hover = false
    @State private var renaming = false
    @State private var draft = ""

    private var active: Bool { model.activeWorktreePath == worktree.path }

    var body: some View {
        if renaming {
            HStack(spacing: 8) {
                LucideIcon(.folderGit2, size: 14, strokeWidth: 1.5).foregroundStyle(Theme.gray500)
                TextField("", text: $draft)
                    .textFieldStyle(.plain)
                    .font(.ui(12))
                    .foregroundStyle(Theme.gray200)
                    .onSubmit {
                        let name = draft.trimmingCharacters(in: .whitespaces)
                        if !name.isEmpty, name != worktree.name { actions.renameWorktree(project, worktree, to: name) }
                        renaming = false
                    }
                    .onExitCommand { renaming = false }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
        } else {
            row
        }
    }

    private var row: some View {
        HStack(spacing: 0) {
            Group {
                if let open = sessionsOpen, hover {
                    Button(action: toggleSessions) {
                        LucideIcon(.chevronRight, size: 12, strokeWidth: 2.5)
                            .foregroundStyle(Theme.gray500)
                            .rotationEffect(.degrees(open ? 90 : 0))
                            .frame(width: 14, height: 14)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Toggle sessions")
                } else {
                    LucideIcon(.folderGit2, size: 14, strokeWidth: 1.5).foregroundStyle(Theme.gray500)
                }
            }
            .padding(.leading, 8)
            HStack(spacing: 8) {
                VStack(alignment: .leading, spacing: 0) {
                    Text(worktree.name).font(.ui(13)).lineLimit(1)
                    HStack(spacing: 2) {
                        LucideIcon(.gitBranch, size: 8)
                        Text(worktree.branch).lineLimit(1)
                    }
                    .font(.ui(10))
                    .foregroundStyle(Theme.gray600)
                }
                Spacer(minLength: 0)
                if hover {
                    HStack(spacing: 2) {
                        rowAction(.terminal, "New terminal") { actions.newTerminal(project, worktree) }
                        rowAction(.plus, "New session", stroke: 2) { actions.newSession(project, worktree) }
                        rowAction(.pencil, "Rename worktree", stroke: 2) {
                            draft = worktree.name
                            renaming = true
                        }
                        rowAction(.trash2, "Remove worktree", hoverColor: Theme.red400) {
                            actions.requestRemoveWorktree(project, worktree)
                        }
                    }
                } else if count > 0 {
                    Text("\(count)").font(.ui(11)).foregroundStyle(Theme.gray600)
                }
            }
            .foregroundStyle(active || hover ? Color.white : Theme.gray400)
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .overlay(alignment: .leading) {
                if active { Rectangle().fill(Color.white).frame(width: 1) }
            }
        }
        .contentShape(Rectangle())
        .onHover { hover = $0 }
        .onTapGesture {
            model.toggle(worktree: worktree.path, in: project.name)
        }
    }
}

struct SessionItem: View {
    let session: TerminalSession
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions
    let showBranch: Bool
    @State private var hover = false

    private var selected: Bool { model.isShowing(session.id, store) }
    private var filesOpen: Bool { model.panes.filesPanes[session.id] != nil }
    private var terminalsOpen: Bool { model.panes.terminalsPanes[session.id] != nil }
    private var browserOpen: Bool { model.panes.browser.isOpen(session.id) }
    private var isShell: Bool { session.agentType == .shell }

    var body: some View {
        HStack(spacing: 8) {
            AgentStatusIcon(session.agentType, status: session.status, size: 14)
                .help("\(session.agentType.rawValue) · \(session.status.rawValue.capitalized)")
            VStack(alignment: .leading, spacing: 0) {
                Text(session.title).lineLimit(1).truncationMode(.tail)
                if showBranch, let b = session.branch {
                    Text(b).font(.ui(10)).foregroundStyle(Theme.gray600).lineLimit(1).truncationMode(.tail)
                }
            }
            Spacer(minLength: 0)
            HStack(spacing: 2) {
                paneToggle(.folderTree, filesOpen ? "Hide files" : "Show files", on: filesOpen) {
                    model.panes.toggleFilesPane(session.id)
                }
                if !isShell || terminalsOpen {
                    paneToggle(.squareTerminal, terminalsOpen ? "Close these terminals" : "Add a terminal", on: terminalsOpen) {
                        actions.toggleTerminalsPanel(session.id)
                    }
                }
                if !isShell || browserOpen {
                    paneToggle(.globe, browserOpen ? "Hide browser" : "Show browser", on: browserOpen) {
                        model.panes.browser.toggle(session.id)
                    }
                }
                if hover {
                    IconButton(.x, size: 12, padding: 2, radius: Theme.radius, color: Theme.gray500,
                               hoverColor: Theme.red400, hoverFill: Theme.white(0.08), help: "Close session") {
                        actions.close(session.id)
                        model.toasts.show("Session closed", .success)
                    }
                }
            }
        }
        .font(.ui(12))
        .foregroundStyle(selected || hover ? Color.white : Theme.gray400)
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .frame(minHeight: 26)
        .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(hover && !selected ? Theme.white(0.04) : .clear))
        .overlay(alignment: .leading) {
            if selected { Rectangle().fill(Color.white).frame(width: 1).padding(.vertical, 4) }
        }
        .contentShape(Rectangle())
        .onHover { hover = $0 }
        .onTapGesture { model.reveal(session.id, store) }
        .accessibilityElement(children: .contain)
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { model.reveal(session.id, store) }
    }

    /// A pane control: shown on hover, and kept lit while its pane is open.
    @ViewBuilder private func paneToggle(_ icon: LucideGlyph, _ help: String, on: Bool,
                                         action: @escaping () -> Void) -> some View {
        if on || hover {
            IconButton(icon, size: 12, padding: 2, radius: Theme.radius, color: on ? Theme.ink : Theme.gray500,
                       hoverColor: Theme.gray200, hoverFill: Theme.white(0.08), help: help, action: action)
        }
    }
}

// MARK: Footer

struct SidebarFooter: View {
    @Bindable var model: AppModel
    let store: VornStore
    @Bindable var settings: SettingsModel

    var body: some View {
        let status = settings.updater?.status ?? settings.updateStatus
        let pending = status.isPending
        let showDot = pending && model.updateBannerDismissed
        VStack(spacing: 0) {
            if pending && !model.updateBannerDismissed {
                let live = store.sessions.filter { store.ended[$0.id] == nil }
                UpdateBanner(status: status,
                             cost: updateCostLine(sessionCount: live.count, aTurnIsRunning: live.contains { $0.status == .running },
                                                  sessionsSurvive: true),
                             onDismiss: { model.updateBannerDismissed = true },
                             onRestart: { settings.updater?.install() })
            }
            HStack(spacing: 2) {
                IconButton(.circleHelp, size: 16, strokeWidth: 1.5, padding: 6, color: Theme.gray500,
                           hoverColor: Theme.gray200, hoverFill: Theme.white(0.04), help: "Welcome Guide") { model.openOnboarding() }
                IconButton(.settings, size: 16, strokeWidth: 1.5, padding: 6, color: Theme.gray500,
                           hoverColor: Theme.gray200, hoverFill: Theme.white(0.04),
                           help: showDot ? "Settings — update ready  ⌘," : "Settings  ⌘,") {
                    if showDot { settings.category = .updates }
                    model.settingsOpen = true
                }
                .overlay(alignment: .topTrailing) {
                    if showDot {
                        Circle().fill(Theme.bronzo).frame(width: 5, height: 5)
                            .overlay(Circle().strokeBorder(Theme.surfacePanel, lineWidth: 2).padding(-2))
                            .padding(2)
                    }
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
        }
    }
}
