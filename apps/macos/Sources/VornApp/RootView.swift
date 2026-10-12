import SwiftUI
import VornCore
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
                content
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .background(Theme.surfaceBase)
        }
        .background(Theme.surfaceBase)
        .overlay(alignment: .topLeading) { if fauxChrome { FauxTrafficLights() } }
        .overlayPreferenceValue(ControlAnchors.self) { anchors in
            OverlayHost(model: model, store: store, actions: actions, features: features, anchors: anchors)
        }
        .overlay {
            if model.settingsOpen {
                SettingsPage(model: features.settings) { model.settingsOpen = false }
                    .transition(.opacity)
            }
        }
        .overlay(alignment: .bottom) { ErrorToast(model: model) }
        .workflowsMenuHost(features.workflows)
        .environment(\.colorScheme, .dark)
        .onChange(of: store.endpoint, initial: true) { _, ep in
            engines.update(socket: ep?.gridSocket, readOnly: store.readOnly)
        }
        .onChange(of: store.sessions.map(\.id)) { _, ids in
            let live = Set(ids)
            model.minimizedIDs.formIntersection(live)
            if let s = model.selectedSessionID, !live.contains(s) { model.selectedSessionID = nil }
            if let s = model.expandedSessionID, !live.contains(s) { model.expandedSessionID = nil }
        }
        .onChange(of: features.scope(model, store), initial: true) { _, scope in features.tasks.scope = scope }
        .onChange(of: model.activeWorkspace, initial: true) { _, id in features.workflows.workspaceId = id }
        .onAppear { features.bind(store: store, model: model) }
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

    private func cardContext(_ s: TerminalSession) -> CardContext {
        CardContext(ended: store.ended[s.id], diffStat: store.diffStats[s.worktreePath ?? s.projectPath])
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

private struct ErrorToast: View {
    @Bindable var model: AppModel

    var body: some View {
        if let message = model.lastError {
            HStack(spacing: 8) {
                Text(message).font(.ui(12)).foregroundStyle(Theme.gray200).lineLimit(2)
                IconButton(.x, size: 12, padding: 2, color: Theme.gray500) { model.lastError = nil }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusLg))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.08), lineWidth: 1))
            .padding(.bottom, 16)
            .task(id: message) {
                try? await Task.sleep(for: .seconds(6))
                if model.lastError == message { model.lastError = nil }
            }
        }
    }
}

