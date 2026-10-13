import Foundation
import Testing
import VornCore
@testable import VornTasks
@testable import VornTerminals
import VornWorkflows

/// Drives a throwaway vornd (own HOME and data dir) the way the app does: a shell, the task board, the workflows.
/// Needs `VORND_BIN` (and `VORN_SESSIOND_BIN`); skipped without it unless `VORN_E2E_REQUIRED=1`.
@MainActor
struct LiveAppTests {
    @Test func shellTasksAndWorkflows() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }

        let store = VornStore(dataDir: server.dataDir, readOnly: false)
        store.start()
        defer { store.stop() }
        #expect(await until(timeout: 15) { store.phase == .connected })
        try #require(store.phase == .connected)

        try await typeIntoAShell(store)
        try await addAndMoveTasks(store.client, project: try await server.addProject("alpha", client: store.client))
        try await listWorkflowAndRuns(store.client)
    }

    private func typeIntoAShell(_ store: VornStore) async throws {
        let shell = try await store.createShell(cwd: NSTemporaryDirectory())
        let engine = TerminalEngine(socket: try #require(store.endpoint?.gridSocket), readOnly: false)
        defer { engine.shutdown() }
        let pane = engine.makePane(sessionID: shell.id)
        pane.attach(cols: 80, rows: 24)
        await engine.settle(timeout: 8)
        #expect(pane.view != nil, "state \(pane.state) error \(engine.lastError ?? "-")")

        let marker = "e2e-\(Int.random(in: 1000...9999))"
        pane.text("echo \(marker)-ok\r")
        let echoed = await until(timeout: 10) {
            await engine.settle(timeout: 0.1)
            pane.contentChanged()
            return pane.screenText().components(separatedBy: "\(marker)-ok").count > 2
        }
        #expect(echoed, "screen: \(pane.screenText())")
        try await store.close(shell.id)
    }

    private func addAndMoveTasks(_ client: VornClient, project: String) async throws {
        let tasks = TasksStore(service: client)
        await tasks.reload()
        let first = try #require(await tasks.create(TaskDraft(projectName: project, title: "First")))
        let second = try #require(await tasks.create(TaskDraft(projectName: project, title: "Second")))

        tasks.move(taskId: second, before: first)
        #expect(await until {
            let stored = (try? await client.listTasks()) ?? []
            return stored.filter { $0.projectName == project }.sorted { $0.order < $1.order }.map(\.id) == [second, first]
        })

        tasks.drop(taskId: first, on: .inProgress)
        #expect(await until { (try? await client.getTask(id: first))??.status == .inProgress })
        await tasks.reload()
        #expect(tasks.tasks.first { $0.id == first }?.status == .inProgress)
    }

    private func listWorkflowAndRuns(_ client: VornClient) async throws {
        let id = "wf-e2e-\(UUID().uuidString.prefix(8))"
        try await client.createWorkflow(WorkflowDefinition(
            id: id, name: "E2E echo",
            nodes: [
                WorkflowNode(id: "trigger", type: .trigger, label: "Trigger", config: ["triggerType": "manual"]),
                WorkflowNode(id: "say", type: .script, label: "Say", slug: "say", config: [
                    "scriptType": "bash", "scriptContent": "echo hi", "cwd": .string(NSTemporaryDirectory()),
                ], position: .init(x: 0, y: 120)),
            ],
            edges: [WorkflowEdge(id: "e0", source: "trigger", target: "say")],
            workspaceId: "personal"
        ))

        let backend = LiveWorkflowsBackend(client: client)
        let workflows = WorkflowsStore(backend: backend)
        await workflows.reload()
        #expect(workflows.workflow(id)?.name == "E2E echo")

        _ = try await backend.run(workflowId: id, inputs: nil)
        #expect(await until(timeout: 15) {
            await workflows.reload()
            return workflows.runs.contains { $0.workflowId == id && $0.status == .success }
        }, "runs: \(workflows.runs.map { "\($0.workflowId) \($0.status)" })")
        let runs = try await backend.listRuns(workflowId: id, limit: 10)
        #expect(runs.count == 1)
    }

    @discardableResult
    private func until(timeout: TimeInterval = 8, _ check: @MainActor () async -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if await check() { return true }
            try? await Task.sleep(for: .milliseconds(100))
        }
        return false
    }
}

/// A vornd on a random port with a temporary HOME and data dir, never the real one.
struct TestVornd {
    let process: Process
    let root: URL
    var dataDir: URL { root.appendingPathComponent("d") }

    static func launch() throws -> TestVornd? {
        let env = ProcessInfo.processInfo.environment
        guard let vornd = env["VORND_BIN"], FileManager.default.isExecutableFile(atPath: vornd) else {
            if env["VORN_E2E_REQUIRED"] == "1" { Issue.record("VORND_BIN does not name a vornd binary") }
            return nil
        }
        let sessiond = env["VORN_SESSIOND_BIN"]
            ?? URL(fileURLWithPath: vornd).deletingLastPathComponent().appendingPathComponent("vorn-sessiond").path
        // A short root keeps the grid socket path under 104 bytes.
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ve-\(UUID().uuidString.prefix(8))")
        let home = root.appendingPathComponent("home")
        let data = root.appendingPathComponent("d")
        try FileManager.default.createDirectory(at: home, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: data, withIntermediateDirectories: true)
        try "PS1='%1~ %# '\n".write(to: home.appendingPathComponent(".zshrc"), atomically: true, encoding: .utf8)
        precondition(data.standardizedFileURL != VornEndpoint.defaultDataDir(environment: [:]).standardizedFileURL)

        let process = Process()
        process.executableURL = URL(fileURLWithPath: vornd)
        process.arguments = [
            "--data-dir", data.path, "--port", "\(Int.random(in: 52000..<60000))", "--host", "127.0.0.1",
            "--sessiond", sessiond, "--exit-with-stdin", "--log-file", root.appendingPathComponent("vornd.log").path,
        ]
        // It and its sessiond end with this test run, and never run this machine's Tailscale.
        let tied = ["VORN_EXIT_WITH": String(ProcessInfo.processInfo.processIdentifier), "VORN_TAILSCALE": "none"]
        process.environment = env.merging(["HOME": home.path, "SHELL": "/bin/zsh"].merging(tied) { $1 }) { $1 }
        process.standardInput = Pipe()
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
        return TestVornd(process: process, root: root)
    }

    /// Adds a project folder under the temp root to vornd's config.
    func addProject(_ name: String, client: VornClient) async throws -> String {
        guard case .object(var config) = try await client.call("config:load") else {
            throw VornError.badResponse("config:load")
        }
        let path = root.appendingPathComponent(name)
        try FileManager.default.createDirectory(at: path, withIntermediateDirectories: true)
        var projects = config["projects"]?.arrayValue ?? []
        projects.append(.object(["name": .string(name), "path": .string(path.path), "preferredAgents": .array([])]))
        config["projects"] = .array(projects)
        let saved: JSONValue? = .object(config)
        try await client.call("config:save", saved)
        return name
    }

    func stop() {
        process.terminate()
        process.waitUntilExit()
        try? FileManager.default.removeItem(at: root)
    }
}
