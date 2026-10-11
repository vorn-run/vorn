import AppKit
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
        a.contextMenu = { id, point in
            model.overlay = nil
            model.contextMenu = ContextMenuState(target: .card(id), point: point)
        }
        a.resume = resume
        a.write = { id, data in run { try await store.write(id, data) } }
        a.prompt = prompt
        a.renameRequest = model.renamingSessionID
        a.endRename = { model.renamingSessionID = nil }
        return a
    }

    func resume(_ id: String) {
        run {
            try await store.resume(id)
            model.focus(id)
        }
    }

    /// The composer's agent mode: an agent in the shell's directory, started on `text`.
    func prompt(_ id: String, _ text: String) {
        guard let s = store.session(id) else { return }
        let project = store.project(named: s.projectName)
        var payload = CreateTerminalPayload(agentType: defaultAgent, projectName: s.projectName,
                                            projectPath: project?.path ?? s.projectPath, initialPrompt: text)
        if s.isWorktree == true, let wt = s.worktreePath {
            payload.existingWorktreePath = wt
            payload.branch = s.branch
        }
        run {
            let created = try await store.createSession(payload)
            model.focus(created.id)
        }
    }

    /// A conversation from the recent list, resumed in a new pane.
    func resumeRecent(_ r: RecentSession) {
        let project = store.projects.first { $0.path == r.projectPath }
        let name = project?.name ?? URL(fileURLWithPath: r.projectPath).lastPathComponent
        var payload = CreateTerminalPayload(agentType: r.agentType, projectName: name, projectPath: r.projectPath)
        payload.resumeSessionId = r.sessionId
        model.overlay = nil
        run {
            let s = try await store.createSession(payload)
            model.focus(s.id)
        }
    }

    /// A session on a new worktree cut from the project's current branch.
    func newWorktreeSession(_ project: Project, agent: AgentType? = nil) {
        let branch = store.worktrees[project.path]?.first { $0.isMain }?.branch
        let payload = CreateTerminalPayload(agentType: agent ?? defaultAgent, projectName: project.name,
                                            projectPath: project.path, branch: branch, useWorktree: true)
        run {
            let s = try await store.createSession(payload)
            model.focus(s.id)
        }
    }

    func setLayout(_ mode: LayoutMode) {
        model.layoutOverride = mode
        if mode == .tabs, model.activeTabID == nil { model.activeTabID = model.selectedSessionID }
        run {
            try await store.setDefault("layoutMode", .string(mode.rawValue))
            model.layoutOverride = nil
        }
    }

    func restoreAll() {
        for one in store.coldRestored { store.showRestored(one.session.id) }
        model.bannerDismissed = true
    }

    func killHeadless(_ id: String) {
        run {
            try await store.killHeadless(id)
            await store.reloadHeadless()
        }
    }

    /// The dialog's submit: adds, or replaces the project it was opened on.
    func saveProject(_ project: Project, replacing original: String?) {
        run {
            if let original {
                try await store.updateProject(original, project)
                if model.activeProject == original { model.activeProject = project.name }
            } else {
                try await store.addProject(project)
            }
        }
    }

    func removeProject(_ name: String) {
        run {
            try await store.removeProject(named: name)
            if model.activeProject == name { model.select(project: nil) }
        }
    }

    /// The project row's "New worktree": a generated name off `main`.
    func newWorktree(_ project: Project) {
        run {
            try await store.createWorktree(projectPath: project.path, branch: "main", name: WorktreeNames.generate())
            model.expandedProjects[project.name] = true
        }
    }

    func renameWorktree(_ project: Project, _ worktree: Worktree, to name: String) {
        run { try await store.renameWorktree(projectPath: project.path, worktreePath: worktree.path, to: name) }
    }

    /// Removes a clean, idle worktree at once; anything else asks first (WorktreeCleanupDialog.tsx).
    func requestRemoveWorktree(_ project: Project, _ worktree: Worktree) {
        let busy = store.sessions.contains { $0.worktreePath == worktree.path }
        run {
            if !busy, await store.worktreeDirty(worktree.path) == false {
                try await store.removeWorktree(projectPath: project.path, worktreePath: worktree.path)
            } else {
                model.overlay = .removeWorktree(projectPath: project.path, worktreePath: worktree.path)
            }
        }
    }

    func removeWorktree(projectPath: String, worktreePath: String, force: Bool) {
        let ids = store.sessions.filter { $0.worktreePath == worktreePath }.map(\.id)
        run {
            if !ids.isEmpty {
                for id in ids { try? await store.close(id) }
                try? await Task.sleep(for: .milliseconds(500))
            }
            try await store.removeWorktree(projectPath: projectPath, worktreePath: worktreePath, force: force)
        }
    }

    /// File › Stop Sessions and Server: ends vornd on purpose, then quits.
    func stopSessionsAndServer() {
        Task { @MainActor in
            try? await store.stopServer()
            NSApplication.shared.terminate(nil)
        }
    }

    func run(_ work: @escaping @MainActor () async throws -> Void) {
        Task { @MainActor in
            do { try await work() } catch { model.lastError = error.localizedDescription }
        }
    }
}
