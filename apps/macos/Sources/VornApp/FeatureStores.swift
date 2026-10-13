import Foundation
import VornCore
import VornSettings
import VornTasks
import VornWorkflows

/// The Tasks and Workflows stores, on the shell's vornd connection.
@MainActor
public final class FeatureStores {
    public let tasks: TasksStore
    public let workflows: WorkflowsStore
    public let settings: SettingsModel
    let notifier = AppNotifier()

    public init(client: VornClient) {
        tasks = TasksStore(service: client)
        workflows = WorkflowsStore(backend: LiveWorkflowsBackend(client: client))
        settings = SettingsModel(client: client)
    }

    /// Points the board's hooks at the shell: live sessions, focusing one, and the toast.
    func bind(store: VornStore, model: AppModel) {
        tasks.isSessionLive = { [weak store] task in
            guard let id = task.assignedSessionId, let store else { return false }
            return store.sessions.contains { $0.id == id }
        }
        tasks.openSession = { [weak store, weak model] task in
            guard let store, let model else { return }
            if let id = task.assignedSessionId, store.sessions.contains(where: { $0.id == id }) {
                model.mainViewMode = .sessions
                model.setFocused(id)
            } else {
                model.lastError = "This task's session has ended"
            }
        }
        notifier.bind(store: store, model: model, workflows: workflows)
        tasks.onToast = { [weak model] message, _ in model?.lastError = message }
        settings.toasts = model.toasts
        settings.onToast = { [weak model] message, kind in
            model?.toasts.show(message, kind == .error ? .error : kind == .success ? .success : .info)
        }
        settings.plugins.sessionIds = { [weak store] in store?.sessions.map(\.id) ?? [] }
        // Settings covers the page, so it closes first; nil opens a new workflow.
        settings.plugins.openWorkflowEditor = { [weak model, weak workflows] id in
            model?.settingsOpen = false
            model?.mainViewMode = .workflows
            if let id { workflows?.openWorkflow(id) } else { workflows?.newWorkflow() }
        }
        tasks.workflows = workflows
        tasks.launchSession = { [weak store] payload in
            guard let store else { throw VornError.notConnected }
            return try await store.createSession(payload)
        }
        tasks.focusSession = { [weak model] id in
            model?.mainViewMode = .sessions
            model?.setFocused(id)
        }
        tasks.writeSession = { [weak store] id, data in try await store?.write(id, data) }
        workflows.taskTitles = { [weak tasks] in
            Dictionary((tasks?.tasks ?? []).map { ($0.id, $0.title) }, uniquingKeysWith: { a, _ in a })
        }
        workflows.taskContext = { [weak tasks] id in tasks?.runTaskContext(id) }
        // As the editor does: the step's agent reopens in a session, and the page closes.
        workflows.resumeSession = { [weak tasks, weak workflows] r in
            tasks?.resumeStep(r)
            workflows?.showAllRuns()
        }
        workflows.openTask = { [weak tasks, weak workflows] id in
            tasks?.selectedTaskId = id
            workflows?.showAllRuns()
        }
    }

    /// Opens a run from anywhere in the app (showRun).
    func showRun(_ runId: String, model: AppModel) {
        model.mainViewMode = .workflows
        workflows.showRun(runId)
    }

    /// The board follows the sidebar: the active project, else the workspace's projects.
    func scope(_ model: AppModel, _ store: VornStore) -> TasksScope {
        if let p = model.activeProject { return .project(p) }
        return .projects(Set(store.projects.filter { $0.workspace == model.activeWorkspace }.map(\.name)))
    }
}
