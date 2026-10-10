import SwiftUI
import VornCore
import VornUI

/// The 40pt bar over the main area (App.tsx): nav cluster when the sidebar is
/// closed, the dock, the breadcrumb, and the grid's tools.
struct TopBar: View {
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions

    var body: some View {
        ZStack {
            HStack(spacing: 0) {
                HStack(spacing: 4) {
                    if !model.sidebarOpen { AppNavCluster(model: model) }
                    if model.mainViewMode == .sessions { SessionDock(model: model, store: store) }
                }
                Spacer(minLength: 0)
                HStack(spacing: 4) {
                    if model.mainViewMode != .tasks && model.mainViewMode != .workflows {
                        IconButton(.slidersHorizontal, size: 16, strokeWidth: 1.5, padding: 4,
                                   hoverFill: Theme.white(0.06), help: "Filter & sort") {}
                        VDivider().padding(.horizontal, 2)
                        IconButton(.rotateCcw, size: 16, strokeWidth: 1.5, padding: 4,
                                   hoverColor: Theme.gray200, hoverFill: Theme.white(0.06), help: "Recent sessions") {}
                        NewSessionMenu(store: store, actions: actions)
                    }
                }
            }
            Breadcrumb(model: model, store: store)
                .frame(maxWidth: 400)
        }
        .padding(.leading, model.sidebarOpen ? 12 : Theme.trafficLightPad)
        .padding(.trailing, 12)
        .frame(height: Theme.toolbarHeight)
        .background(Theme.surfaceBase)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }
    }
}

private struct AppNavCluster: View {
    @Bindable var model: AppModel

    var body: some View {
        HStack(spacing: 4) {
            IconButton(.panelLeft, size: 16, padding: 4) { model.sidebarOpen.toggle() }
                .tooltip("Toggle sidebar", shortcut: "⌘B")
            VDivider().padding(.horizontal, 2)
            MainViewPills(model: model)
        }
    }
}

private struct MainViewPills: View {
    @Bindable var model: AppModel

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
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
                .tooltip(mode.label, shortcut: mode.shortcut)
            }
        }
        .padding(2)
        .background(Theme.white(0.04), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
    }
}

/// Minimized cards and sessions waiting on approval (SessionDock.tsx, expanded).
private struct SessionDock: View {
    @Bindable var model: AppModel
    let store: VornStore

    var body: some View {
        let items = store.sessions.filter { model.minimizedIDs.contains($0.id) }
        HStack(spacing: 6) {
            ForEach(items.prefix(4)) { s in
                MinimizedPill(session: s) {
                    model.minimizedIDs.remove(s.id)
                    model.focus(s.id)
                }
            }
            if items.count > 4 {
                Text("+\(items.count - 4)")
                    .font(.ui(11, weight: .medium))
                    .foregroundStyle(Theme.gray400)
                    .padding(.horizontal, 6)
                    .frame(minWidth: 28, minHeight: 26)
                    .background(Theme.surfaceRaised, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                    .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.hairline, lineWidth: 1))
            }
        }
    }
}

private struct MinimizedPill: View {
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
                    if let wt {
                        chevron
                        HStack(spacing: 4) {
                            LucideIcon(.gitBranch, size: 11).foregroundStyle(Theme.gray500)
                            Text(wt.branch).lineLimit(1).frame(maxWidth: 120).fixedSize(horizontal: true, vertical: false)
                            LucideIcon(.chevronDown, size: 10).foregroundStyle(Theme.gray500)
                        }
                        .foregroundStyle(Color.white)
                    }
                } else {
                    Text(name).foregroundStyle(Color.white).lineLimit(1)
                }
            }
            .font(.ui(13))
        }
    }

    private var chevron: some View {
        LucideIcon(.chevronRight, size: 10).foregroundStyle(Theme.gray600).padding(.horizontal, 2)
    }
}

/// The top bar's "+": new session or terminal in a project (GridContextMenu.tsx).
private struct NewSessionMenu: View {
    let store: VornStore
    let actions: ShellActions

    var body: some View {
        Menu {
            Menu("New session in…") {
                ForEach(store.projects) { p in Button(p.name) { actions.newSession(p, nil) } }
            }
            Menu("New terminal in…") {
                ForEach(store.projects) { p in Button(p.name) { actions.newTerminal(p, nil) } }
            }
        } label: {
            Hovering { hover in
                LucideIcon(.plus, size: 16)
                    .foregroundStyle(hover ? Color.white : Theme.gray400)
                    .padding(4)
                    .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(hover ? Theme.white(0.06) : .clear))
                    .contentShape(Rectangle())
            }
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .tooltip("New session", shortcut: "⌘N")
    }
}
