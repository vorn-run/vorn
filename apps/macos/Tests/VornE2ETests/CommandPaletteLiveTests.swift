import Foundation
import Testing
@testable import VornApp
import VornCore

/// The command palette (CommandPalette.tsx) against a throwaway vornd: it lists the daemon's sessions, projects and
/// workflows, and running a command does the thing.
@MainActor
struct CommandPaletteLiveTests {
    @Test func listsAndRunsCommands() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let store = VornStore(dataDir: server.dataDir, readOnly: false)
        store.start()
        defer { store.stop() }
        #expect(await until(timeout: 15) { store.phase == .connected })
        try #require(store.phase == .connected)
        let model = AppModel()
        model.persistsSidebar = false
        let actions = ShellActions(store: store, model: model)
        let features = FeatureStores(client: store.client)
        features.bind(store: store, model: model)
        let runner = PaletteRunner(model: model, store: store, actions: actions, features: features)

        let dir = server.root.appendingPathComponent("alpha")
        try makeRepo(at: dir)
        try await store.addProject(Project(name: "alpha", path: dir.path))
        let project = try #require(store.project(named: "alpha"))
        await store.loadGitInfo(project.path)
        actions.newTerminal(project, nil)
        #expect(await until { store.sessions(in: project).count == 1 })
        let shell = try #require(store.sessions(in: project).first)

        let workflowId = "wf-palette-\(UUID().uuidString.prefix(8))"
        try await store.client.createWorkflow(WorkflowDefinition(
            id: workflowId, name: "Palette run",
            nodes: [
                WorkflowNode(id: "trigger", type: .trigger, label: "Trigger", config: ["triggerType": "manual"]),
                WorkflowNode(id: "say", type: .script, label: "Say", slug: "say",
                             config: ["scriptType": "bash", "scriptContent": "echo hi"], position: .init(x: 0, y: 120)),
            ],
            edges: [WorkflowEdge(id: "e0", source: "trigger", target: "say")],
            workspaceId: "personal"))
        await features.workflows.reload()

        // ⌘K opens the palette and Esc closes it.
        model.toggle(.commandPalette)
        #expect(model.overlay == .commandPalette)
        #expect(model.escape())
        #expect(model.overlay == nil)

        func commands() -> [PaletteCommand] { PaletteCommands.build(runner.sources(recent: [])) }
        func run(_ id: String) throws {
            model.overlay = .commandPalette
            let cmd = try #require(commands().first { $0.id == id }, "no \(id)")
            runner.run(cmd)
            #expect(model.overlay != .commandPalette)
        }
        let ids = Set(commands().map(\.id))
        #expect(ids.contains("terminal:\(shell.id)"))
        #expect(ids.contains("project:alpha"))
        #expect(ids.contains("workflow:\(workflowId)"))
        #expect(ids.contains("terminal:project:alpha"))
        #expect(commands().first { $0.id == "action:stop-sessions-and-server" }?.sublabel == "1 running")

        // Projects and filters.
        try run("project:alpha")
        #expect(model.activeProject == "alpha")
        try run("project:all")
        #expect(model.activeProject == nil)
        try run("filter:status:waiting")
        #expect(model.statusFilter == .waiting)
        try run("filter:sort:created")
        #expect(model.sortMode == .created)
        model.statusFilter = .all

        // A terminal row focuses that session.
        try run("terminal:\(shell.id)")
        #expect(model.expandedSessionID == shell.id && model.mainViewMode == .sessions)
        model.expandedSessionID = nil

        // Typing "terminal in alpha" ranks the project's terminal first, and Enter starts one.
        let top = try #require(PaletteSearch.results(commands(), query: "terminal in alpha").first)
        #expect(top.id == "terminal:project:alpha")
        model.overlay = .commandPalette
        runner.run(top)
        #expect(await until { store.sessions(in: project).count == 2 })

        // New Terminal Session uses the active project.
        model.select(project: "alpha")
        try run("action:new-terminal")
        #expect(await until { store.sessions(in: project).count == 3 })

        // Running a workflow starts a run in vornd.
        try run("workflow:\(workflowId)")
        #expect(await until { (try? await store.client.listRuns(workflowId: workflowId))?.isEmpty == false })

        // Layout, views, panels and settings.
        try run("action:toggle-layout")
        #expect(await until { store.defaults?.layoutMode == "tabs" })
        try run("action:toggle-layout")
        #expect(await until { store.defaults?.layoutMode == "grid" })
        try run("action:create-task")
        #expect(model.mainViewMode == .tasks && features.tasks.dialogOpen)
        features.tasks.dialogOpen = false
        try run("action:open-tasks")
        #expect(model.mainViewMode == .tasks)
        try run("action:add-workflow")
        #expect(model.mainViewMode == .workflows)
        try run("action:manage-worktrees")
        #expect(model.settingsOpen && features.settings.category == .worktrees)
        model.settingsOpen = false
        try run("action:toggle-sidebar")
        #expect(!model.sidebarOpen)
        try run("action:shortcuts-panel")
        #expect(model.overlay == .shortcuts)
        try run("action:add-project")
        #expect(model.overlay == .addProject(editing: nil))
        try run("action:welcome-guide")
        #expect(model.overlay == .onboarding && model.onboardingStep == 0)
    }

    private func makeRepo(at url: URL) throws {
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        try "hello\n".write(to: url.appendingPathComponent("README.md"), atomically: true, encoding: .utf8)
        for args in [["init", "-q", "-b", "main"], ["add", "."],
                     ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "init"]] {
            let git = Process()
            git.executableURL = URL(fileURLWithPath: "/usr/bin/git")
            git.arguments = ["-C", url.path] + args
            try git.run()
            git.waitUntilExit()
        }
    }

    private func until(timeout: TimeInterval = 20, _ check: @MainActor () async -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if await check() { return true }
            try? await Task.sleep(for: .milliseconds(100))
        }
        return false
    }
}
