import SwiftUI
import VornCore
import VornTerminals
import VornUI

/// The window's content: sidebar, top bar, and the active view.
public struct RootView: View {
    @Bindable var model: AppModel
    let store: VornStore
    let engines: EngineHolder
    var fauxChrome = false

    public init(model: AppModel, store: VornStore, engines: EngineHolder, fauxChrome: Bool = false) {
        self.model = model
        self.store = store
        self.engines = engines
        self.fauxChrome = fauxChrome
    }

    private var actions: ShellActions { ShellActions(store: store, model: model) }

    public var body: some View {
        HStack(spacing: 0) {
            if model.sidebarOpen {
                Sidebar(model: model, store: store, actions: actions)
            }
            VStack(spacing: 0) {
                TopBar(model: model, store: store, actions: actions)
                content
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .background(Theme.surfaceBase)
        }
        .background(Theme.surfaceBase)
        .overlay(alignment: .topLeading) { if fauxChrome { FauxTrafficLights() } }
        .overlay(alignment: .bottom) { ErrorToast(model: model) }
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
    }

    @ViewBuilder private var content: some View {
        switch model.mainViewMode {
        case .sessions:
            sessions
        case .tasks, .workflows:
            Theme.surfaceBase
        }
    }

    @ViewBuilder private var sessions: some View {
        let visible = model.visibleSessions(store)
        if case .failed(let why) = store.phase, store.sessions.isEmpty {
            ConnectionNotice(text: "Waiting for vornd… \(why)")
        } else if let id = model.expandedSessionID, let s = visible.first(where: { $0.id == id }) {
            AgentCardView(session: s, index: visible.firstIndex(of: s) ?? 0, engine: engines.engine,
                          isSelected: true, isDimmed: false, isExpanded: true,
                          focusRequest: model.focusRequests[s.id] ?? 0, actions: actions.cardActions)
        } else {
            SessionGridView(sessions: visible, engine: engines.engine, selectedID: model.selectedSessionID,
                            focusRequest: model.focusRequests, actions: actions.cardActions,
                            onDoubleClickEmpty: { quickLaunch() }) {
                PromptLauncher(projects: store.projects.filter { $0.workspace == model.activeWorkspace },
                               activeProject: model.activeProject, defaultAgent: actions.defaultAgent,
                               branchFor: { p in store.worktrees[p.path]?.first { $0.isMain }?.branch },
                               isGitRepo: { p in store.gitRepos[p.path] == true },
                               tip: model.launcherTip) { payload in
                    let s = try await store.createSession(payload)
                    model.focus(s.id)
                }
            }
        }
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

/// Close/minimise/zoom where the window puts them, for renders without a window.
struct FauxTrafficLights: View {
    var body: some View {
        HStack(spacing: 8) {
            light(0xFF5F57, 0xE14640)
            light(0xFEBC2E, 0xDFA023)
            light(0x28C840, 0x1AAB29)
        }
        .padding(.leading, 16)
        .padding(.top, 14)
    }

    private func light(_ fill: UInt32, _ edge: UInt32) -> some View {
        Circle()
            .fill(Color(hex: fill))
            .overlay(Circle().strokeBorder(Color(hex: edge), lineWidth: 0.5))
            .frame(width: 12, height: 12)
    }
}
