import SwiftUI
import VornCore
import VornTasks
import VornTerminals
import VornUI
import VornWorkflows

/// The title-bar row over the main area: the view switch, the dock, the breadcrumb, and the
/// view's tools grouped in glass. Content passes under it.
struct TopBar: View {
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions
    let features: FeatureStores

    var body: some View {
        ZStack {
            HStack(spacing: 8) {
                AppNavCluster(model: model, workflows: features.workflows, labels: true)
                if model.mainViewMode == .sessions {
                    SessionDock(model: model, store: store, features: features,
                                includeMinimized: model.layoutMode(store) == .grid)
                }
                Spacer(minLength: 0)
                NavigationGlassGroup {
                    ToolbarGroup {
                        switch model.mainViewMode {
                        case .sessions:
                            ViewOptionsButton(model: model, store: store)
                            DockButton(model: model, store: store, features: features,
                                       includeMinimized: model.layoutMode(store) == .grid)
                            HeadlessBadge(model: model, store: store)
                            RecentSessionsButton(model: model)
                            NewSessionButton(model: model)
                        case .tasks:
                            TaskViewOptionsButton(store: features.tasks)
                            ToolbarSymbolButton("plus", help: "Add task") { features.tasks.openNewTask() }
                        case .workflows:
                            WorkflowsHeader(store: features.workflows)
                        }
                    }
                }
            }
            Breadcrumb(model: model, store: store, actions: actions)
                .frame(maxWidth: 400)
        }
        .padding(.leading, model.sidebarOpen ? 0 : Theme.trafficLightPad)
        .padding(.trailing, 12)
        .frame(height: Theme.toolbarHeight)
    }
}

/// A capsule of glass around a row of toolbar controls.
struct ToolbarGroup<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        HStack(spacing: 2) { content }
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .navigationGlass(Capsule())
    }
}

/// The sidebar toggle while the sidebar is closed, then the view switch.
struct AppNavCluster: View {
    @Bindable var model: AppModel
    let workflows: WorkflowsStore
    var labels = false

    var body: some View {
        NavigationGlassGroup {
            HStack(spacing: 8) {
                if !model.sidebarOpen {
                    ToolbarSymbolButton("sidebar.left", help: "Toggle sidebar", shortcut: "⌘B") { model.sidebarOpen.toggle() }
                }
                ViewSwitch(model: model, waiting: workflows.waitingApprovals.count, labels: labels)
            }
        }
    }
}

/// Sessions, Tasks and Workflows as one glass segmented control; the chosen one raised in a neutral fill.
struct ViewSwitch: View {
    @Bindable var model: AppModel
    let waiting: Int
    var labels = true

    var body: some View {
        HStack(spacing: 2) {
            ForEach(MainViewMode.allCases, id: \.self) { mode in
                segment(mode)
            }
        }
        .padding(3)
        .navigationGlass(Capsule())
    }

    private func segment(_ mode: MainViewMode) -> some View {
        let active = model.mainViewMode == mode
        let help = mode == .workflows && waiting > 0 ? "Workflows · \(waiting) awaiting review" : mode.label
        return Hovering { hover in
            Button { model.mainViewMode = mode } label: {
                HStack(spacing: 6) {
                    Image(systemName: mode.symbol)
                        .font(.system(size: 12, weight: .regular))
                        .symbolRenderingMode(.monochrome)
                    if labels { Text(mode.label).font(.ui(12, weight: .medium)) }
                }
                .foregroundStyle(active ? Theme.ink : hover ? Theme.white(0.7) : Theme.white(0.45))
                .padding(.horizontal, labels ? 12 : 8)
                .frame(height: 28)
                .background { if active { RaisedSegment() } }
                .overlay(alignment: .topTrailing) {
                    if mode == .workflows && waiting > 0 { StatusDot(Theme.bronzo) }
                }
                .contentShape(Capsule())
            }
            .buttonStyle(.plain)
        }
        .tooltip(help, shortcut: mode.shortcut)
        .accessibilityLabel(help)
        .accessibilityAddTraits(active ? .isSelected : [])
    }
}

/// The selected segment: a neutral fill with a top highlight and a soft drop.
private struct RaisedSegment: View {
    var body: some View {
        Capsule()
            .fill(Theme.white(0.10))
            .overlay(Capsule().strokeBorder(
                LinearGradient(colors: [Theme.white(0.16), .clear], startPoint: .top, endPoint: .center), lineWidth: 0.5))
            .shadow(color: .black.opacity(0.3), radius: 1, y: 1)
    }
}

struct MinimizedPill: View {
    let session: TerminalSession
    let restore: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: restore) {
                HStack(spacing: 6) {
                    Circle().fill(dot).frame(width: 6, height: 6)
                    AgentIcon(session.agentType, size: 14)
                    Text(session.title)
                        .font(.ui(11, weight: .medium))
                        .foregroundStyle(Theme.gray200)
                        .lineLimit(1)
                        .frame(maxWidth: 120)
                        .fixedSize(horizontal: true, vertical: false)
                    if let b = session.branch {
                        Text("·").font(.ui(10)).foregroundStyle(Theme.gray600)
                        HStack(spacing: 2) {
                            LucideIcon(session.isWorktree == true ? .folderGit2 : .gitBranch, size: 9, strokeWidth: 1.5)
                            Text(b).lineLimit(1)
                        }
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundStyle(Theme.gray500)
                        .frame(maxWidth: 90)
                        .fixedSize(horizontal: true, vertical: false)
                    }
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 4)
                .background(Theme.surfaceRaised, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd)
                    .strokeBorder(hover ? Theme.white(0.12) : Theme.hairline, lineWidth: 1))
            }
            .buttonStyle(.plain)
        }
        .help("Click to restore")
    }

    private var dot: Color {
        switch session.status {
        case .running: Theme.ink
        case .waiting: Theme.bronzo
        case .error: Theme.danger
        case .idle: Theme.inkGhost
        }
    }
}

/// Project, worktree and branch of the grid's filter (ToolbarBreadcrumb.tsx).
private struct Breadcrumb: View {
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions
    @State private var picking = false
    @State private var switching = false

    var body: some View {
        if let name = model.activeProject {
            HStack(spacing: 2) {
                if let path = model.activeWorktreePath {
                    let project = store.project(named: name)
                    let wts = project.flatMap { store.worktrees[$0.path] } ?? []
                    let wt = path == AppModel.mainWorktree ? wts.first { $0.isMain } : wts.first { $0.path == path }
                    Button(name) { model.activeWorktreePath = nil }
                        .buttonStyle(.plain)
                        .foregroundStyle(Theme.gray500)
                        .lineLimit(1)
                    if path != AppModel.mainWorktree, let wt {
                        chevron
                        Text(wt.name).foregroundStyle(Theme.gray400).lineLimit(1)
                    }
                    if let wt, let project {
                        chevron
                        branchButton(wt, project)
                    }
                } else {
                    Text(name).foregroundStyle(Color.white).lineLimit(1)
                }
            }
            .font(.ui(13))
        }
    }

    /// useBranchSwitcher: checks out a branch in the filtered tree.
    private func branchButton(_ wt: Worktree, _ project: Project) -> some View {
        let cwd = wt.isMain ? project.path : wt.path
        return Hovering { hover in
            Button { picking.toggle() } label: {
                HStack(spacing: 4) {
                    LucideIcon(.gitBranch, size: 11).foregroundStyle(Theme.gray500)
                    Text(wt.branch).lineLimit(1).frame(maxWidth: 120).fixedSize(horizontal: true, vertical: false)
                    LucideIcon(.chevronDown, size: 10).foregroundStyle(Theme.gray500)
                }
                .foregroundStyle(Color.white)
                .padding(.horizontal, 4)
                .background(RoundedRectangle(cornerRadius: Theme.radius)
                    .fill(picking ? Theme.white(0.08) : hover ? Theme.white(0.06) : .clear))
                .opacity(switching ? 0.5 : 1)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(switching)
            .accessibilityLabel("Switch branch (current: \(wt.branch))")
        }
        .popover(isPresented: $picking, arrowEdge: .bottom) {
            BranchPicker(projectPath: project.path, currentBranch: wt.branch, git: actions.gitActions) { pick in
                picking = false
                guard pick != wt.branch, !switching else { return }
                switching = true
                Task {
                    await actions.checkout(cwd: cwd, projectPath: project.path, branch: pick)
                    switching = false
                }
            }
        }
    }

    private var chevron: some View {
        LucideIcon(.chevronRight, size: 10).foregroundStyle(Theme.gray600).padding(.horizontal, 2)
    }
}

/// The top bar's "+": the grid menu under it (GridContextMenu.tsx).
private struct NewSessionButton: View {
    @Bindable var model: AppModel

    var body: some View {
        ToolbarSymbolButton("plus", help: "New session", shortcut: "⌘N", isOn: model.overlay == .newMenu) {
            model.toggle(.newMenu)
        }
        .controlAnchor("newMenu")
    }
}
