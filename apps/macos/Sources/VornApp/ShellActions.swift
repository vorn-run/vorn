import SwiftUI
import VornCore
import VornTerminals
import VornUI

/// The things the shell's controls ask vornd to do, bound to one store and model.
@MainActor
struct ShellActions {
    let store: VornStore
    let model: AppModel

    var defaultAgent: AgentType { store.defaults?.defaultAgent ?? .claude }

    func newTerminal(_ project: Project?, _ worktree: Worktree?) {
        run {
            let cwd = worktree?.path ?? project?.path
            let s = try await store.createShell(cwd: cwd, project: project, worktree: worktree)
            model.focus(s.id)
        }
    }

    func newSession(_ project: Project?, _ worktree: Worktree?, agent: AgentType? = nil) {
        guard let project else { return }
        var payload = CreateTerminalPayload(agentType: agent ?? defaultAgent, projectName: project.name,
                                            projectPath: project.path)
        if let worktree, !worktree.isMain {
            payload.existingWorktreePath = worktree.path
            payload.branch = worktree.branch
        }
        run {
            let s = try await store.createSession(payload)
            model.focus(s.id)
        }
    }

    func close(_ id: String) {
        run {
            try await store.close(id)
            if model.selectedSessionID == id { model.selectedSessionID = nil }
            if model.expandedSessionID == id { model.expandedSessionID = nil }
        }
    }

    func rename(_ id: String, _ name: String) {
        run { try await store.rename(id, to: name) }
    }

    var activeProject: Project? { model.activeProject.flatMap(store.project(named:)) }

    var activeWorktree: Worktree? {
        guard let p = activeProject, let path = model.activeWorktreePath else { return nil }
        let wts = store.worktrees[p.path] ?? []
        return path == AppModel.mainWorktree ? wts.first { $0.isMain } : wts.first { $0.path == path }
    }

    var cardActions: CardActions {
        var a = CardActions()
        a.select = { id in model.selectedSessionID = id }
        a.expand = { id in model.expandedSessionID = model.expandedSessionID == id ? nil : id }
        a.minimize = { id in
            model.minimizedIDs.insert(id)
            if model.selectedSessionID == id { model.selectedSessionID = nil }
        }
        a.close = close
        a.rename = rename
        return a
    }

    private func run(_ work: @escaping @MainActor () async throws -> Void) {
        Task { @MainActor in
            do { try await work() } catch { model.lastError = error.localizedDescription }
        }
    }
}
