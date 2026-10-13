import SwiftUI
import VornCore
import VornTasks
import VornTerminals
import VornUI
import VornWorkflows

/// The 40pt bar over the main area (App.tsx): nav cluster when the sidebar is
/// closed, the dock, the breadcrumb, and the grid's tools.
struct TopBar: View {
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions
    let features: FeatureStores

    var body: some View {
        ZStack {
            HStack(spacing: 0) {
                HStack(spacing: 4) {
                    if !model.sidebarOpen { AppNavCluster(model: model, workflows: features.workflows) }
                    if model.mainViewMode == .sessions {
                        SessionDock(model: model, store: store, features: features,
                                    includeMinimized: model.layoutMode(store) == .grid)
                        HeadlessBadge(model: model, store: store)
                    }
                }
                Spacer(minLength: 0)
                HStack(spacing: 4) {
                    switch model.mainViewMode {
                    case .sessions:
                        ViewOptionsButton(model: model, store: store)
                        VDivider().padding(.horizontal, 2)
                        RecentSessionsButton(model: model)
                        NewSessionButton(model: model)
                    case .tasks:
                        TaskViewOptionsButton(store: features.tasks)
                        IconButton(.plus, size: 16, padding: 4, hoverFill: Theme.white(0.06), help: "Add task") {
                            features.tasks.openNewTask()
                        }
                    case .workflows:
                        WorkflowsHeader(store: features.workflows)
                    }
                }
            }
            Breadcrumb(model: model, store: store, actions: actions)
                .frame(maxWidth: 400)
        }
        .padding(.leading, model.sidebarOpen ? 12 : Theme.trafficLightPad)
        .padding(.trailing, 12)
        .frame(height: Theme.toolbarHeight)
        .background(Theme.surfaceBase)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }
    }
}

struct AppNavCluster: View {
    @Bindable var model: AppModel
    let workflows: WorkflowsStore

    var body: some View {
        HStack(spacing: 4) {
            IconButton(.panelLeft, size: 16, padding: 4) { model.sidebarOpen.toggle() }
                .tooltip("Toggle sidebar", shortcut: "⌘B")
            VDivider().padding(.horizontal, 2)
            MainViewPills(model: model, waiting: workflows.waitingApprovals.count)
        }
    }
}

private struct MainViewPills: View {
    @Bindable var model: AppModel
    let waiting: Int

    var body: some View {
        HStack(spacing: 2) {
            ForEach(MainViewMode.allCases, id: \.self) { mode in
                let active = model.mainViewMode == mode
                Hovering { hover in
                    Button { model.mainViewMode = mode } label: {
                        LucideIcon(mode.icon, size: 14)
                            .foregroundStyle(active ? Color.white : hover ? Theme.gray300 : Theme.gray500)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 4)
                            .background(RoundedRectangle(cornerRadius: Theme.radiusMd)
                                .fill(active ? Theme.white(0.1) : .clear))
                            .overlay(alignment: .topTrailing) {
                                if mode == .workflows && waiting > 0 { WaitingBadge(count: waiting) }
                            }
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
                .tooltip(mode == .workflows && waiting > 0 ? "Workflows · \(waiting) awaiting review" : mode.label,
                         shortcut: mode.shortcut)
            }
        }
        .padding(2)
        .background(Theme.white(0.04), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
    }
}

/// The count of gates awaiting review on the Workflows pill.
private struct WaitingBadge: View {
    let count: Int

    var body: some View {
        Text(count > 9 ? "9+" : "\(count)")
            .font(.system(size: 9, design: .monospaced))
            .monospacedDigit()
            .foregroundStyle(Theme.surfaceBase)
            .padding(.horizontal, 3)
            .frame(minWidth: 12, minHeight: 12, maxHeight: 12)
            .background(Theme.bronzo, in: Capsule())
            .offset(x: 2, y: -2)
            .accessibilityHidden(true)
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
        let open = model.overlay == .newMenu
        IconButton(.plus, size: 16, padding: 4, color: open ? .white : Theme.gray400, hoverFill: Theme.white(0.06)) {
            model.toggle(.newMenu)
        }
        .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(open ? Theme.white(0.06) : .clear))
        .tooltip("New session", shortcut: "⌘N")
        .controlAnchor("newMenu")
    }
}
