import SwiftUI
import VornCore
import VornTerminals
import VornUI

/// The tabs layout (TabView.tsx): one strip of sessions that doubles as the top bar, the active one below.
struct TabsView<Empty: View>: View {
    @Bindable var model: AppModel
    let store: VornStore
    let engines: EngineHolder
    let actions: ShellActions
    let features: FeatureStores
    let context: (TerminalSession) -> CardContext
    @ViewBuilder let empty: () -> Empty

    var body: some View {
        let tabs = model.visibleSessions(store, includeMinimized: true)
        let shortcutIDs = model.visibleSessions(store).map(\.id)
        // chooseActiveTab: a live session stays active even when the filters hide its tab.
        let active = model.activeTabID.flatMap(store.session) ?? tabs.first
        var tabActions = actions.cardActions
        tabActions.close = { id in close(id, tabs: tabs.map(\.id), actions: actions.cardActions) }
        return VStack(spacing: 0) {
            HStack(spacing: 0) {
                if !model.sidebarOpen {
                    AppNavCluster(model: model, workflows: features.workflows).padding(.leading, 8)
                }
                HStack(alignment: .bottom, spacing: 4) {
                    ForEach(tabs) { s in
                        SessionTab(session: s, isActive: s.id == active?.id,
                                   shortcut: shortcutIDs.firstIndex(of: s.id).flatMap { $0 < 9 ? $0 + 1 : nil },
                                   actions: tabActions) {
                            model.showTab(s.id)
                        }
                    }
                    NewTabButtons(model: model, actions: actions)
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 4)
                .frame(minHeight: 40, alignment: .bottom)
                HStack(spacing: 4) {
                    SessionDock(model: model, store: store, features: features, includeMinimized: false)
                    HeadlessBadge(model: model, store: store)
                    ViewOptionsButton(model: model, store: store)
                    VDivider().padding(.horizontal, 2)
                    RecentSessionsButton(model: model)
                }
                .padding(.horizontal, 8)
            }
            .padding(.leading, model.sidebarOpen ? 0 : Theme.trafficLightPad)
            .frame(minHeight: 40)
            .background(Theme.surfaceOverlay)
            .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }

            if let active {
                let ctx = context(active)
                HStack(spacing: 0) {
                    SessionStage(session: active, engine: engines.engine, focusRequest: model.focusRequests[active.id] ?? 0,
                                 actions: actions.cardActions, context: ctx)
                    if let column = ctx.paneColumn {
                        Rectangle().fill(Theme.hairline).frame(width: 1)
                        column.frame(width: 380)
                    }
                }
            } else {
                Group {
                    if model.statusFilter != .all { NoMatchingAgents() } else { empty() }
                }
                .padding(16)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
    }

    /// TabView.tsx `handleCloseTab`: the tab beside it becomes active, then the session closes.
    private func close(_ id: String, tabs: [String], actions: CardActions) {
        if model.expandedSessionID == id { model.setFocused(nil) }
        if model.activeTabID == id, let i = tabs.firstIndex(of: id) {
            model.activeTabID = tabs.indices.contains(i + 1) ? tabs[i + 1] : i > 0 ? tabs[i - 1] : nil
        }
        actions.close(id)
    }
}

/// One tab: status, name, and on hover the session's controls in place of its ⌘N badge.
private struct SessionTab: View {
    let session: TerminalSession
    let isActive: Bool
    let shortcut: Int?
    let actions: CardActions
    let select: () -> Void
    @State private var hovering = false
    @State private var renaming = false
    @State private var draft = ""
    @State private var confirmClose = false
    @State private var dropTargeted = false

    var body: some View {
        HStack(spacing: 8) {
            AgentStatusIcon(session.agentType, status: session.status, size: 16)
            if renaming {
                TextField("", text: $draft)
                    .textFieldStyle(.plain)
                    .font(.ui(12))
                    .frame(width: 100)
                    .onSubmit {
                        let name = draft.trimmingCharacters(in: .whitespaces)
                        if !name.isEmpty { actions.rename(session.id, name) }
                        renaming = false
                        actions.endRename()
                    }
                    .onExitCommand { renaming = false; actions.endRename() }
            } else {
                Text(session.title).lineLimit(1).truncationMode(.tail).help(tooltip)
            }
            Spacer(minLength: 0)
        }
        .font(.ui(13))
        .foregroundStyle(isActive ? Color.white : hovering ? Theme.gray300 : Theme.gray500)
        .padding(.leading, 12)
        .padding(.trailing, 40)
        .frame(minWidth: 120, maxWidth: 260, minHeight: 36, maxHeight: 36)
        .overlay(alignment: .trailing) { trailing.padding(.trailing, 8) }
        .overlay(alignment: .bottom) { Rectangle().fill(isActive ? Color.white : .clear).frame(height: 1) }
        .overlay(alignment: .leading) { Rectangle().fill(dropTargeted ? Theme.white(0.5) : .clear).frame(width: 2) }
        .contentShape(Rectangle())
        .onTapGesture(perform: select)
        .onRightClick { p in actions.contextMenu(session.id, p) }
        .onHover { hovering = $0 }
        .modifier(SessionDragSource(sessionID: session.id, title: session.title, enabled: actions.reorder != nil && !renaming))
        .modifier(SessionDropTarget(sessionID: session.id, reorder: actions.reorder, targeted: $dropTargeted))
        .onChange(of: actions.renameRequest == session.id, initial: true) { _, asked in
            if asked { draft = session.title; renaming = true }
        }
    }

    @ViewBuilder private var trailing: some View {
        ZStack(alignment: .trailing) {
            if let shortcut, !renaming {
                Text("⌘\(shortcut)")
                    .font(.system(size: 9, design: .monospaced))
                    .foregroundStyle(Theme.gray500)
                    .padding(.horizontal, 4)
                    .padding(.vertical, 2)
                    .background(RoundedRectangle(cornerRadius: Theme.radius).fill(Theme.white(0.06)))
                    .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.1), lineWidth: 1))
                    .opacity(hovering ? 0 : 1)
                    .allowsHitTesting(false)
            }
            HStack(spacing: 4) {
                if !renaming {
                    tabButton(.folderOpen, "Browse files") { actions.browseFiles(session.id) }
                    let terminalsOpen = actions.terminalsOpen(session.id)
                    if session.agentType != .shell || terminalsOpen {
                        tabButton(.squareTerminal, terminalsOpen ? "Close terminals" : "Add a terminal") { actions.addTerminal(session.id) }
                    }
                    if session.agentType != .shell || actions.browserOpen(session.id) {
                        tabButton(.globe, "Open browser") { actions.openBrowser(session.id) }
                    }
                    tabButton(.pencil, "Rename session") {
                        draft = session.title
                        renaming = true
                    }
                }
                tabButton(.x, "Close session", close: true) { confirmClose = true }
                    .popover(isPresented: $confirmClose, arrowEdge: .bottom) {
                        ConfirmClose { confirmClose = false } onConfirm: {
                            confirmClose = false
                            actions.close(session.id)
                        }
                    }
            }
            .padding(.leading, 4)
            .background(hovering ? Theme.surfaceOverlay : .clear)
            .opacity(hovering ? 1 : 0)
        }
    }

    private func tabButton(_ icon: LucideGlyph, _ help: String, close: Bool = false,
                           action: @escaping () -> Void) -> some View {
        IconButton(icon, size: 13, padding: 3.5, radius: Theme.radius, color: Theme.gray200,
                   hoverColor: close ? Theme.gray200 : Theme.gray300,
                   hoverFill: close ? Theme.white(0.1) : .clear, help: help, action: action)
    }

    private var tooltip: String {
        var lines = [session.title, session.status.rawValue.capitalized]
        if session.agentType == .shell {
            if let cwd = session.shellCwd { lines.append(cwd) }
        } else if let b = session.branch {
            lines.append((session.isWorktree == true ? "Worktree: " : "Branch: ") + b)
        }
        lines.append(session.projectName)
        return lines.joined(separator: "\n")
    }
}

/// "+" for a session in the active place, the chevron for the full menu.
private struct NewTabButtons: View {
    @Bindable var model: AppModel
    let actions: ShellActions

    var body: some View {
        HStack(spacing: 0) {
            button(.plus, 14, width: 28, "New session") {
                actions.newSession(actions.activeProject ?? actions.store.projects.first, actions.activeWorktree)
            }
            button(.chevronDown, 12, width: 22, "More session options") { model.toggle(.newMenu) }
                .controlAnchor("newMenu")
        }
    }

    private func button(_ icon: LucideGlyph, _ size: CGFloat, width: CGFloat, _ help: String,
                        action: @escaping () -> Void) -> some View {
        Hovering { hover in
            Button(action: action) {
                LucideIcon(icon, size: size, strokeWidth: 1.5)
                    .foregroundStyle(hover ? Color.white : Theme.gray200)
                    .frame(width: width, height: 36)
                    .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(hover ? Theme.white(0.1) : .clear))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .help(help)
    }
}

/// RecentSessionsButton.tsx.
struct RecentSessionsButton: View {
    @Bindable var model: AppModel

    var body: some View {
        let open = model.overlay == .recent
        IconButton(.rotateCcw, size: 16, strokeWidth: 1.5, padding: 4, color: open ? .white : Theme.gray400,
                   hoverColor: Theme.gray200, hoverFill: Theme.white(0.06), help: "Recent sessions") {
            model.toggle(.recent)
        }
        .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(open ? Theme.white(0.1) : .clear))
        .controlAnchor("recent")
    }
}
