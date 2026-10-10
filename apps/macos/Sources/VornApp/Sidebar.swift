import SwiftUI
import VornCore
import VornUI

/// The left sidebar (ProjectSidebar.tsx): workspace, view switch, projects, footer.
struct Sidebar: View {
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions

    @Environment(\.terminalSnapshot) private var snapshot

    var body: some View {
        VStack(spacing: 0) {
            SidebarHeader(model: model, store: store)
            if snapshot {
                list.frame(minHeight: 0, maxHeight: .infinity, alignment: .top).clipped().layoutPriority(-1)
            } else {
                ScrollView { list }.scrollIndicators(.never)
            }
            SidebarFooter()
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
            ProjectsSection(model: model, store: store, actions: actions)
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
    var icon: Lucide {
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

    var body: some View {
        SectionHeader(title: "Projects", open: $model.projectsOpen) {
            IconButton(.listFilter, size: 13, strokeWidth: 1.5, padding: 2, radius: Theme.radius,
                       color: Theme.gray600, hoverFill: Theme.white(0.08), help: "Filter & sort") {}
            IconButton(.folderPlus, size: 13, strokeWidth: 1.5, padding: 2, radius: Theme.radius,
                       color: Theme.gray600, hoverFill: Theme.white(0.08), help: "Add project") {}
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
            ForEach(projects) { p in
                ProjectItem(project: p, model: model, store: store, actions: actions)
                    .padding(.top, 2)
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
struct NavRow: View {
    let icon: Lucide
    let label: String
    var badge: Int = 0
    let active: Bool
    let action: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                HStack(spacing: 8) {
                    LucideIcon(icon, size: 14, strokeWidth: 1.5)
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

/// `p-0.5 rounded text-gray-500 hover:text-white hover:bg-white/[0.08]`.
private func rowAction(_ icon: Lucide, _ help: String, stroke: CGFloat = 1.5, color: Color = Theme.gray500,
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

    private var sessions: [TerminalSession] { store.sessions(in: project) }
    private var isGit: Bool { store.gitRepos[project.path] == true }
    private var worktrees: [Worktree] { store.worktrees[project.path] ?? [] }
    private var expanded: Bool { model.expandedProjects[project.name] ?? !sessions.isEmpty }
    private var active: Bool { model.activeProject == project.name && model.activeWorktreePath == nil }

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
                        rowAction(.folderGit2, "New worktree", color: Theme.inkSecondary) {}
                    }
                    rowAction(.ellipsis, "More", stroke: 2) {}
                }
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
    }

    @ViewBuilder private func body(expanded: Void) -> some View {
        if !isGit {
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
        } else {
            let main = worktrees.first { $0.isMain }
            MainWorktreeRow(project: project, branch: main?.branch ?? "main", model: model, actions: actions,
                            worktree: main)
            ForEach(sessions.filter { $0.isWorktree != true }) { s in
                SessionItem(session: s, model: model, store: store, actions: actions, showBranch: false)
                    .padding(.leading, 16)
            }
            ForEach(worktrees.filter { !$0.isMain }.sorted { $0.name < $1.name }) { wt in
                WorktreeItem(project: project, worktree: wt, model: model, actions: actions)
                ForEach(sessions.filter { $0.worktreePath == wt.path }) { s in
                    SessionItem(session: s, model: model, store: store, actions: actions, showBranch: false)
                        .padding(.leading, 16)
                }
            }
        }
    }
}

private struct MainWorktreeRow: View {
    let project: Project
    let branch: String
    @Bindable var model: AppModel
    let actions: ShellActions
    let worktree: Worktree?
    @State private var hover = false

    private var active: Bool {
        model.activeProject == project.name && model.activeWorktreePath == AppModel.mainWorktree
    }

    var body: some View {
        HStack(spacing: 8) {
            LucideIcon(hover ? .chevronRight : .gitBranch, size: 14, strokeWidth: 1.5)
                .foregroundStyle(Theme.gray500)
            Text(branch).lineLimit(1)
            Spacer(minLength: 0)
            if hover {
                HStack(spacing: 2) {
                    rowAction(.terminal, "New terminal") { actions.newTerminal(project, worktree) }
                    rowAction(.plus, "New session", stroke: 2) { actions.newSession(project, worktree) }
                }
            }
        }
        .font(.ui(13))
        .foregroundStyle(active ? Color.white : Theme.gray400)
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .frame(height: 30)
        .background(RoundedRectangle(cornerRadius: Theme.radiusMd)
            .fill(active ? Theme.white(0.08) : hover ? Theme.white(0.04) : .clear))
        .contentShape(Rectangle())
        .onHover { hover = $0 }
        .onTapGesture {
            model.activeProject = project.name
            model.activeWorktreePath = AppModel.mainWorktree
        }
    }
}

private struct WorktreeItem: View {
    let project: Project
    let worktree: Worktree
    @Bindable var model: AppModel
    let actions: ShellActions
    @State private var hover = false

    private var active: Bool { model.activeWorktreePath == worktree.path }

    var body: some View {
        HStack(spacing: 0) {
            LucideIcon(.folderGit2, size: 14, strokeWidth: 1.5)
                .foregroundStyle(Theme.gray500)
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
                        rowAction(.pencil, "Rename worktree") {}
                    }
                }
            }
            .foregroundStyle(active ? Color.white : Theme.gray400)
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .overlay(alignment: .leading) {
                if active { Rectangle().fill(Color.white).frame(width: 1) }
            }
        }
        .contentShape(Rectangle())
        .onHover { hover = $0 }
        .onTapGesture {
            model.activeProject = project.name
            model.activeWorktreePath = worktree.path
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

    private var selected: Bool { model.selectedSessionID == session.id }

    var body: some View {
        HStack(spacing: 8) {
            AgentStatusIcon(session.agentType, status: session.status, size: 14)
            Text(session.title).lineLimit(1).truncationMode(.tail)
            if showBranch, let b = session.branch {
                Text(b).font(.ui(10)).foregroundStyle(Theme.gray600).lineLimit(1)
            }
            Spacer(minLength: 0)
            if hover {
                IconButton(.x, size: 12, padding: 2, radius: Theme.radius, color: Theme.gray500,
                           hoverColor: Theme.red400, help: "Close session") { actions.close(session.id) }
            }
        }
        .font(.ui(12))
        .foregroundStyle(selected || hover ? Color.white : Theme.gray400)
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .frame(height: 26)
        .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(hover ? Theme.white(0.04) : .clear))
        .overlay(alignment: .leading) {
            if selected { Rectangle().fill(Color.white).frame(width: 1).padding(.vertical, 4) }
        }
        .contentShape(Rectangle())
        .onHover { hover = $0 }
        .onTapGesture {
            model.minimizedIDs.remove(session.id)
            model.focus(session.id)
        }
    }
}

// MARK: Footer

struct SidebarFooter: View {
    var body: some View {
        HStack(spacing: 2) {
            IconButton(.circleHelp, size: 16, strokeWidth: 1.5, padding: 6, color: Theme.gray500,
                       hoverColor: Theme.gray200, hoverFill: Theme.white(0.04), help: "Welcome Guide") {}
            IconButton(.settings, size: 16, strokeWidth: 1.5, padding: 6, color: Theme.gray500,
                       hoverColor: Theme.gray200, hoverFill: Theme.white(0.04), help: "Settings") {}
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
    }
}
