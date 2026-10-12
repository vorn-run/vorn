import Foundation
import VornCore
import VornTasks
import VornWorkflows

/// The Tasks and Workflows stores, on the shell's vornd connection.
@MainActor
public final class FeatureStores {
    public let tasks: TasksStore
    public let workflows: WorkflowsStore

    public init(client: VornClient) {
        tasks = TasksStore(service: client)
        workflows = WorkflowsStore(backend: LiveWorkflowsBackend(client: client))
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
                model.focus(id)
            } else {
                model.lastError = "This task's session has ended"
            }
        }
        tasks.onToast = { [weak model] message, _ in model?.lastError = message }
        tasks.workflows = workflows
        tasks.launchSession = { [weak store] payload in
            guard let store else { throw VornError.notConnected }
            return try await store.createSession(payload)
        }
        tasks.focusSession = { [weak model] id in
            model?.mainViewMode = .sessions
            model?.focus(id)
        }
        tasks.writeSession = { [weak store] id, data in try await store?.write(id, data) }
    }

    /// The board follows the sidebar: the active project, else the workspace's projects.
    func scope(_ model: AppModel, _ store: VornStore) -> TasksScope {
        if let p = model.activeProject { return .project(p) }
        return .projects(Set(store.projects.filter { $0.workspace == model.activeWorkspace }.map(\.name)))
    }
}
