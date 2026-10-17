import Foundation
import Testing
@testable import VornApp
import VornCore
@testable import VornPanes

/// The changes panel, commit, branch switch, files, editor and panel terminals against a throwaway vornd.
@MainActor
struct GitPanesLiveTests {
    private func connect(_ server: TestVornd) async throws -> (VornStore, AppModel, ShellActions) {
        let store = VornStore(dataDir: server.dataDir, readOnly: false)
        store.start()
        #expect(await liveUntil(timeout: 15) { store.phase == .connected })
        try #require(store.phase == .connected)
        let model = AppModel()
        model.persistsSidebar = false
        model.panes = PanesModel(persists: false)
        model.panes.confirmDiscard = { true }
        return (store, model, ShellActions(store: store, model: model))
    }

    @Test func changesCommitAndBranchSwitch() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let (store, model, actions) = try await connect(server)
        defer { store.stop() }
        let repo = server.root.appendingPathComponent("repo")
        try makeGitRepo(at: repo, files: ["README.md": "one\n", "src/app.ts": "let a = 1\n"])
        try "one\ntwo\n".write(to: repo.appendingPathComponent("README.md"), atomically: true, encoding: .utf8)
        try git(repo, "branch", "other")

        let client = store.client
        let diff = try #require(try await client.diffFull(repo.path))
        #expect(diff.files.map(\.filePath) == ["README.md"])
        #expect(diff.stat.insertions == 1)
        #expect(DiffRow.parse(diff.files[0].diff).contains { $0.kind == .add && $0.text == "+two" })

        // Commit (the dialog's call), then the tree is clean and the log has it.
        #expect(try await client.commit(cwd: repo.path, message: "Add a line", includeUnstaged: true) == nil)
        #expect(try await client.diffFull(repo.path)?.files.isEmpty == true)
        #expect(try git(repo, "log", "-1", "--format=%s").trimmingCharacters(in: .whitespacesAndNewlines) == "Add a line")
        #expect(try await client.commit(cwd: repo.path, message: "Nothing", includeUnstaged: true) != nil)

        // Branch switcher from the card.
        try await store.addProject(Project(name: "repo", path: repo.path))
        let shell = try await store.createShell(cwd: repo.path, project: store.project(named: "repo"))
        let picked = await actions.gitActions.listBranches(repo.path)
        #expect(Set(picked) == ["main", "other"])
        let err = await actions.gitActions.checkout(shell, "other")
        #expect(err == nil)
        #expect(try git(repo, "branch", "--show-current").trimmingCharacters(in: .whitespacesAndNewlines) == "other")
        #expect(store.session(shell.id)?.branch == "other")
        #expect(await actions.gitActions.checkout(shell, "missing") != nil)

        // The status bar opens the changes panel; Open in lists at least Finder.
        actions.cardActions.showChanges(shell.id)
        #expect(model.panes.diffSessionID == shell.id)
        #expect(await actions.gitActions.detectIDEs().contains { $0.id == "finder" })
    }

    @Test func filesEditorAndPanelTerminals() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let (store, model, actions) = try await connect(server)
        defer { store.stop() }
        let repo = server.root.appendingPathComponent("home/files")
        try makeGitRepo(at: repo, files: ["src/app.ts": "let a = 1\n", "README.md": "hi\n"])
        try await store.addProject(Project(name: "files", path: repo.path))
        let owner = try await store.createShell(cwd: repo.path, project: store.project(named: "files"))

        // The tree lists the root and a folder on demand.
        let tree = FileTreeModel(client: store.client, cwd: repo.path, remoteHostId: nil)
        await tree.load()
        #expect(tree.root?.map(\.name).contains("src") == true)
        let src = try #require(tree.root?.first { $0.name == "src" })
        await tree.toggle(src)
        #expect(tree.rows().contains { $0.entry?.name == "app.ts" })
        tree.filter = "app"
        #expect(tree.rows().compactMap(\.entry?.name) == ["src", "app.ts"])

        // The editor: a draft is kept, saved to disk, and a change on disk under an edit is caught.
        let path = repo.appendingPathComponent("src/app.ts").path
        let key = PaneID.fileTab(owner.id, path)
        EditorDrafts.shared.forget(key)
        let buffer = FileBuffer(client: store.client, filePath: path, remoteHostId: nil, draftKey: key)
        await buffer.load()
        #expect(buffer.ready && buffer.content == "let a = 1\n", "\(String(describing: buffer.readOnly)) \(buffer.content ?? "nil") \(buffer.ready)")
        buffer.draft = "let a = 2\n"
        #expect(buffer.dirty)
        await buffer.keepDraft()
        #expect(EditorDrafts.shared.read(key, filePath: path)?.text == "let a = 2\n")
        await buffer.save()
        #expect(try String(contentsOfFile: path, encoding: .utf8) == "let a = 2\n")
        #expect(!buffer.dirty && EditorDrafts.shared.read(key, filePath: path) == nil)
        try await Task.sleep(for: .milliseconds(20))
        try "let a = 3 // agent\n".write(toFile: path, atomically: true, encoding: .utf8)
        buffer.draft = "let a = 4\n"
        await buffer.save()
        #expect(buffer.conflict)
        #expect(try String(contentsOfFile: path, encoding: .utf8) == "let a = 3 // agent\n")
        await buffer.save(force: true)
        #expect(try String(contentsOfFile: path, encoding: .utf8) == "let a = 4\n" && !buffer.conflict)

        // A draft left by a quit comes back on reopen.
        EditorDrafts.shared.write(key, filePath: path, text: "unsaved\n", base: buffer.base)
        let reopened = FileBuffer(client: store.client, filePath: path, remoteHostId: nil, draftKey: key)
        await reopened.load()
        #expect(reopened.draft == "unsaved\n" && reopened.dirty && !reopened.conflict)
        EditorDrafts.shared.forget(key)

        // Browse files opens the pane; Add a terminal claims a new shell in the owner's tree.
        actions.cardActions.browseFiles(owner.id)
        #expect(model.panes.filesPanes[owner.id] != nil)
        actions.cardActions.addTerminal(owner.id)
        #expect(await liveUntil { model.panes.terminalsPanes[owner.id]?.terminals.count == 1 })
        let held = try #require(model.panes.terminalsPanes[owner.id]?.terminals.first)
        #expect(store.session(held)?.projectPath == repo.path)
        #expect(!model.visibleSessions(store).contains { $0.id == held })
        // Closing the owner ends the shells its panel holds.
        actions.close(owner.id)
        #expect(await liveUntil { store.session(held) == nil && store.session(owner.id) == nil })
    }

    @Test(.disabled("Cleanup is a built-in extension since #825; the paused Mac app is not ported to it")) func worktreeCleanupIsOfferedWhenItsLastSessionEnds() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let (store, model, actions) = try await connect(server)
        defer { store.stop() }
        let repo = server.root.appendingPathComponent("wt")
        try makeGitRepo(at: repo)
        try await store.updateConfig { c in
            c["agentCommands"] = ["claude": ["command": "/usr/bin/true", "args": [], "headlessArgs": []]]
        }
        try await store.addProject(Project(name: "wt", path: repo.path))
        let wt = try await store.createWorktree(projectPath: repo.path, branch: "main", name: "tidy")
        var offered: (String, String)?
        store.onWorktreeCleanup = { p, w in offered = (p, w) }
        var payload = CreateTerminalPayload(agentType: .claude, projectName: "wt", projectPath: repo.path)
        payload.existingWorktreePath = wt.path
        payload.branch = wt.branch
        let agent = try await store.createSession(payload)
        try await Task.sleep(for: .seconds(1))
        try await store.write(agent.id, "exit\r")
        #expect(await liveUntil(timeout: 15) { offered != nil })
        #expect(offered?.1.contains("tidy") == true)

        // The toast's Remove takes the worktree away under a progress toast.
        actions.offerWorktreeCleanup(projectPath: repo.path, worktreePath: wt.path)
        let toast = try #require(model.toasts.items.last)
        #expect(toast.message.contains("last session ended"))
        toast.actions.last?.run(toast.id)
        #expect(await liveUntil { model.toasts.items.contains { $0.message == "Worktree removed" } })
        #expect(!FileManager.default.fileExists(atPath: wt.path))
    }
}
