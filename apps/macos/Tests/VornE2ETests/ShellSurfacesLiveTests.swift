import Foundation
import Testing
@testable import VornApp
import VornCore

/// The shell's Wave 1 surfaces against a throwaway vornd: view options, the card and grid menus' actions, tabs,
/// shortcuts, the ended strip, recent sessions, and the sidebar's project and worktree actions.
@MainActor
struct ShellSurfacesLiveTests {
    @Test func sessionsSurfaces() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let (store, model, actions) = try await connect(server)
        defer { store.stop() }

        let dir = server.root.appendingPathComponent("alpha")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try await store.addProject(Project(name: "alpha", path: dir.path))
        let project = try #require(store.project(named: "alpha"))

        // Grid and card menus: "New terminal" in a project.
        actions.newTerminal(project, nil)
        #expect(await until { store.sessions(in: project).count == 1 })
        actions.newTerminal(project, nil)
        #expect(await until { store.sessions(in: project).count == 2 })
        let ids = store.sessions(in: project).map(\.id)
        #expect(await until { model.selectedSessionID == ids.last })

        // View options: the status filter hides what it does not match, and "All" brings them back.
        model.statusFilter = .error
        #expect(model.visibleSessions(store).isEmpty)
        #expect(model.hasActiveViewOptions(store))
        model.statusFilter = .all
        #expect(model.visibleSessions(store).map(\.id) == ids)

        // Shortcuts: ⌘1-9 jumps, ⌘] and ⌘[ wrap, Esc unwinds focus before selection.
        model.jump(to: 1, store)
        #expect(model.selectedSessionID == ids[0])
        model.cycleFocus(store, by: 1)
        #expect(model.selectedSessionID == ids[1])
        model.cycleFocus(store, by: 1)
        #expect(model.selectedSessionID == ids[0])
        model.expandedSessionID = ids[0]
        model.cycleFocus(store, by: -1)
        #expect(model.expandedSessionID == ids[1])
        #expect(model.escape())
        #expect(model.expandedSessionID == nil && model.selectedSessionID == ids[1])
        #expect(model.escape())
        #expect(model.selectedSessionID == nil)

        // F2 renames through vornd.
        model.focus(ids[0])
        model.startRename()
        #expect(model.renamingSessionID == ids[0])
        actions.rename(ids[0], "Renamed")
        #expect(await until { store.session(ids[0])?.displayName == "Renamed" })

        // Tabs layout is saved to the config and the active tab follows ⌘N.
        actions.setLayout(.tabs)
        #expect(model.layoutMode(store) == .tabs)
        #expect(await until { store.defaults?.layoutMode == "tabs" && model.layoutOverride == nil })
        model.jump(to: 2, store)
        #expect(model.activeTabID == ids[1])
        actions.setLayout(.grid)
        #expect(await until { store.defaults?.layoutMode == "grid" })

        // Ended strip: a shell that exits stays on screen as ended, and closing it removes it.
        try await store.write(ids[1], "exit\r")
        #expect(await until(timeout: 10) { store.endedIDs.contains(ids[1]) })
        actions.close(ids[1])
        #expect(await until { store.session(ids[1]) == nil })

        // Recent sessions answers for the project (nothing on disk in a fresh HOME).
        let recent = try await store.recentSessions(projectPath: project.path)
        #expect(recent.isEmpty)
    }

    @Test func sidebarProjectsAndWorktrees() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let (store, model, actions) = try await connect(server)
        defer { store.stop() }

        // Add project dialog: add, then edit icon, colour and name; the rest of the entry is kept.
        let path = server.root.appendingPathComponent("repo")
        try makeRepo(at: path)
        actions.saveProject(Project(name: "repo", path: path.path, icon: "Code", iconColor: "#3b82f6"), replacing: nil)
        #expect(await until { store.project(named: "repo") != nil })
        #expect(await until { store.gitRepos[path.path] == true })
        model.select(project: "repo")
        actions.saveProject(Project(name: "renamed", path: path.path, icon: "Rocket", iconColor: "#22c55e"),
                            replacing: "repo")
        #expect(await until { store.project(named: "renamed")?.icon == "Rocket" })
        #expect(store.project(named: "repo") == nil)
        #expect(await until { model.activeProject == "renamed" })
        let project = try #require(store.project(named: "renamed"))

        // Worktrees: new, rename, and a clean idle one is removed without asking.
        actions.newWorktree(project)
        #expect(await until(timeout: 15) { (store.worktrees[path.path] ?? []).contains { !$0.isMain } })
        var wt = try #require(store.worktrees[path.path]?.first { !$0.isMain })
        #expect(await store.worktreeDirty(wt.path) == false)
        actions.renameWorktree(project, wt, to: "renamed-tree")
        #expect(await until { store.worktrees[path.path]?.contains { $0.name == "renamed-tree" } == true })
        wt = try #require(store.worktrees[path.path]?.first { $0.name == "renamed-tree" })
        actions.requestRemoveWorktree(project, wt)
        #expect(await until(timeout: 15) { !(store.worktrees[path.path] ?? []).contains { !$0.isMain } })
        #expect(model.overlay == nil)

        // A worktree with a session asks first; the cleanup dialog's action closes the session and removes it.
        actions.newWorktree(project)
        #expect(await until(timeout: 15) { (store.worktrees[path.path] ?? []).contains { !$0.isMain } })
        wt = try #require(store.worktrees[path.path]?.first { !$0.isMain })
        actions.newTerminal(project, wt)
        #expect(await until { store.sessions.contains { $0.worktreePath == wt.path } })
        actions.requestRemoveWorktree(project, wt)
        #expect(await until { model.overlay == .removeWorktree(projectPath: path.path, worktreePath: wt.path) })
        model.overlay = nil
        actions.removeWorktree(projectPath: path.path, worktreePath: wt.path, force: false)
        #expect(await until(timeout: 15) { !(store.worktrees[path.path] ?? []).contains { !$0.isMain } })
        #expect(await until { !store.sessions.contains { $0.worktreePath == wt.path } })

        // Project menu: delete.
        actions.removeProject("renamed")
        #expect(await until { store.project(named: "renamed") == nil })
        #expect(await until { model.activeProject == nil })
    }

    private func connect(_ server: TestVornd) async throws -> (VornStore, AppModel, ShellActions) {
        let store = VornStore(dataDir: server.dataDir, readOnly: false)
        store.start()
        #expect(await until(timeout: 15) { store.phase == .connected })
        try #require(store.phase == .connected)
        let model = AppModel()
        model.persistsSidebar = false
        return (store, model, ShellActions(store: store, model: model))
    }

    private func makeRepo(at url: URL) throws {
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        try "hello\n".write(to: url.appendingPathComponent("README.md"), atomically: true, encoding: .utf8)
        for args in [["init", "-q", "-b", "main"], ["add", "."], ["commit", "-q", "-m", "init"]] {
            let git = Process()
            git.executableURL = URL(fileURLWithPath: "/usr/bin/git")
            git.arguments = ["-C", url.path] + args
            git.environment = [
                "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.com",
                "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@example.com",
                "HOME": url.deletingLastPathComponent().path, "PATH": "/usr/bin:/bin",
            ]
            try git.run()
            git.waitUntilExit()
            #expect(git.terminationStatus == 0, "git \(args.joined(separator: " "))")
        }
    }

    @discardableResult
    private func until(timeout: TimeInterval = 20, _ check: @MainActor () async -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if await check() { return true }
            try? await Task.sleep(for: .milliseconds(100))
        }
        return false
    }
}
