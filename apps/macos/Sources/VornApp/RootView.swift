import SwiftUI
import VornCore
import VornPanes
import VornSettings
import VornTasks
import VornTerminals
import VornUI
import VornWorkflows

/// The window's content: sidebar, top bar, and the active view.
public struct RootView: View {
    @Bindable var model: AppModel
    let store: VornStore
    let engines: EngineHolder
    let features: FeatureStores
    var fauxChrome = false

    public init(model: AppModel, store: VornStore, engines: EngineHolder, features: FeatureStores,
                fauxChrome: Bool = false) {
        self.model = model
        self.store = store
        self.engines = engines
        self.features = features
        self.fauxChrome = fauxChrome
    }

    private var actions: ShellActions { ShellActions(store: store, model: model) }

    public var body: some View {
        HStack(spacing: 0) {
            if model.sidebarOpen {
                Sidebar(model: model, store: store, actions: actions, features: features)
            }
            VStack(spacing: 0) {
                // A workflow's page draws its own bar, as the inline editor does today.
                if showsTopBar {
                    TopBar(model: model, store: store, actions: actions, features: features)
                }
                HStack(spacing: 0) {
                    if !(model.panes.diffMaximized && changesSession != nil) {
                        content.frame(maxWidth: .infinity, maxHeight: .infinity)
                    } else {
                        Color.clear.frame(width: 48)
                    }
                    if let s = changesSession {
                        ChangesPanel(panes: model.panes, session: s, stat: store.diffStats[s.worktreePath ?? s.projectPath],
                                     project: model.activeProject.flatMap(store.project(named:)), services: actions.paneServices) {
                            model.commitOpen = true
                        }
                        .id(s.id)
                        .frame(width: model.panes.diffMaximized ? nil : model.panes.diffWidth)
                        .frame(maxWidth: model.panes.diffMaximized ? .infinity : nil)
                    }
                }
            }
            .background(Theme.surfaceBase)
        }
        .background(Theme.surfaceBase)
        .overlay(alignment: .topLeading) { if fauxChrome { FauxTrafficLights() } }
        .overlayPreferenceValue(ControlAnchors.self) { anchors in
            OverlayHost(model: model, store: store, actions: actions, features: features, anchors: anchors)
        }
        .overlay {
            if model.commitOpen, let s = changesSession {
                let cwd = s.worktreePath ?? s.projectPath
                CommitDialog(cwd: cwd, branch: s.branch, stat: store.diffStats[cwd], client: store.client, toasts: model.toasts,
                             onClose: { model.commitOpen = false }) {
                    model.panes.diffSessionID = nil
                    Task {
                        await store.refreshDiffStat(cwd)
                        model.panes.diffSessionID = s.id
                    }
                }
            }
        }
        .overlay {
            if model.settingsOpen {
                SettingsPage(model: features.settings) { model.settingsOpen = false }
                    .transition(.opacity)
            }
        }
        .overlay(alignment: .bottomTrailing) { ToastStack(center: model.toasts) }
        .workflowsMenuHost(features.workflows)
        .environment(\.colorScheme, .dark)
        .onChange(of: store.endpoint, initial: true) { _, ep in
            engines.update(socket: ep?.gridSocket, readOnly: store.readOnly)
        }
        .onChange(of: store.sessions.map(\.id)) { _, ids in
            let live = Set(ids)
            if store.phase == .connected { model.panes.prune(liveSessions: live) }
            model.minimizedIDs.formIntersection(live)
            if let s = model.selectedSessionID, !live.contains(s) { model.selectedSessionID = nil }
            if let s = model.expandedSessionID, !live.contains(s) { model.expandedSessionID = nil }
        }
        .onChange(of: features.scope(model, store), initial: true) { _, scope in features.tasks.scope = scope }
        .onChange(of: model.activeWorkspace, initial: true) { _, id in features.workflows.workspaceId = id }
        .onAppear {
            features.bind(store: store, model: model)
            store.onWorktreeCleanup = { p, w in actions.offerWorktreeCleanup(projectPath: p, worktreePath: w) }
            model.panes.onSurfaceCard = { id in model.minimizedIDs.remove(id) }
        }
        .task(id: diffPaths) {
            // Uncommitted changes have no event; the status bars poll while their trees are on screen.
            while !Task.isCancelled {
                for path in diffPaths { await store.refreshDiffStat(path) }
                try? await Task.sleep(for: .seconds(10))
            }
        }
    }

    /// A workflow's page draws its own bar, as do the tabs strip and a focused session's header.
    private var showsTopBar: Bool {
        switch model.mainViewMode {
        case .workflows: features.workflows.editingWorkflow == nil
        case .tasks: true
        case .sessions: model.layoutMode(store) == .grid && focusedSession == nil
        }
    }

    private var focusedSession: TerminalSession? {
        model.expandedSessionID.flatMap { id in model.visibleSessions(store, includeMinimized: true).first { $0.id == id } }
    }

    private var diffPaths: [String] {
        guard model.mainViewMode == .sessions, store.phase == .connected else { return [] }
        return Array(Set(model.visibleSessions(store).map { $0.worktreePath ?? $0.projectPath })).sorted()
    }

    private var changesSession: TerminalSession? {
        guard model.mainViewMode == .sessions, let id = model.panes.diffSessionID else { return nil }
        return store.session(id)
    }

    private func cardContext(_ s: TerminalSession) -> CardContext {
        var c = CardContext(ended: store.ended[s.id], diffStat: store.diffStats[s.worktreePath ?? s.projectPath])
        let panes = model.panes
        let entries = panes.columnEntries(s.id)
        if !entries.isEmpty {
            var services = actions.paneServices
            services.engine = engines.engine
            c.paneColumn = AnyView(PaneColumn(panes: panes, sessionID: s.id, services: services))
            c.splitRatio = panes.cardSplits[s.id]?.terminal ?? SplitRatio.default
            c.onSplitCommit = { panes.setCardSplit(s.id, terminal: $0) }
            c.paneMaximized = entries.contains { $0.id == panes.maximizedPaneId }
        }
        return c
    }

    /// Files popped out of `s`, as cells after its card.
    private func cards(after s: TerminalSession) -> [(id: String, view: AnyView)] {
        var services = actions.paneServices
        services.engine = engines.engine
        return model.panes.cards(ownedBy: s.id).filter { !model.minimizedIDs.contains($0) }.map { id in
            (id, AnyView(EditorCard(panes: model.panes, cardID: id, services: services)))
        }
    }

    @ViewBuilder private var content: some View {
        switch model.mainViewMode {
        case .sessions:
            sessions
        case .tasks:
            TasksView(store: features.tasks)
                .onDisappear { features.tasks.stop() }
        case .workflows:
            WorkflowsView(store: features.workflows) {
                if !model.sidebarOpen { AppNavCluster(model: model) }
            }
            .onDisappear { features.workflows.stop() }
        }
    }

    @ViewBuilder private var sessions: some View {
        let visible = model.visibleSessions(store)
        if case .failed(let why) = store.phase, store.sessions.isEmpty {
            ConnectionNotice(text: "Waiting for vornd… \(why)")
        } else if let s = focusedSession {
            focusedStage(s)
        } else if model.layoutMode(store) == .tabs {
            TabsView(model: model, store: store, engines: engines, actions: actions, features: features,
                     context: cardContext) { launcher }
        } else {
            VStack(spacing: 0) {
                RestoredBanner(model: model, store: store, actions: actions)
                SessionGridView(sessions: visible, engine: engines.engine, selectedID: model.selectedSessionID,
                                focusRequest: model.focusRequests, actions: actions.cardActions,
                                columns: max(0, model.gridColumns), isFiltered: model.statusFilter != .all,
                                context: cardContext, onDoubleClickEmpty: { quickLaunch() },
                                onContextMenu: { p in
                                    model.overlay = nil
                                    model.contextMenu = ContextMenuState(target: .grid, point: p)
                                }) { launcher }
                    .cards(after: cards(after:))
            }
        }
    }

    private var launcher: some View {
        PromptLauncher(projects: store.projects.filter { $0.workspace == model.activeWorkspace },
                       activeProject: model.activeProject, defaultAgent: actions.defaultAgent,
                       branchFor: { p in store.worktrees[p.path]?.first { $0.isMain }?.branch },
                       isGitRepo: { p in store.gitRepos[p.path] == true },
                       tip: model.launcherTip) { payload in
            let s = try await store.createSession(payload)
            model.focus(s.id)
        }
    }

    /// FocusedTerminal.tsx: one session fills the area, its header standing in for the titlebar.
    private func focusedStage(_ s: TerminalSession) -> some View {
        let ids = model.scopedSessions(store).map(\.id)
        var context = cardContext(s)
        if ids.count > 1, let i = ids.firstIndex(of: s.id) {
            context.nav = (i, ids.count)
            context.onPrevious = { model.cycleFocus(store, by: -1) }
            context.onNext = { model.cycleFocus(store, by: 1) }
        }
        if context.paneColumn != nil { context.fixedColumnWidth = 420 }
        if !model.sidebarOpen {
            context.leadingInset = Theme.trafficLightPad
            context.headerLeading = AnyView(AppNavCluster(model: model))
        }
        let visible = model.visibleSessions(store)
        return AgentCardView(session: s, index: visible.firstIndex(of: s) ?? 0, engine: engines.engine,
                             isSelected: true, isDimmed: false, isExpanded: true,
                             focusRequest: model.focusRequests[s.id] ?? 0, actions: actions.cardActions,
                             context: context, variant: .focused)
    }

    /// Double-click on the empty grid: the default agent in the active project.
    private func quickLaunch() {
        actions.newSession(actions.activeProject, actions.activeWorktree)
    }
}

private struct ConnectionNotice: View {
    let text: String
    var body: some View {
        Text(text)
            .font(.ui(12))
            .foregroundStyle(Theme.gray500)
            .multilineTextAlignment(.center)
            .padding(24)
    }
}
