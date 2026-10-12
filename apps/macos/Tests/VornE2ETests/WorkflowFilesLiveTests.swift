import Foundation
import Testing
import VornCore
@testable import VornWorkflows

/// The sidebar row's menu and workflow files against a throwaway vornd.
@MainActor
struct WorkflowFilesLiveTests {
    @Test func exportImportAndRowMenu() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let vorn = VornStore(dataDir: server.dataDir, readOnly: false)
        vorn.start()
        defer { vorn.stop() }
        #expect(await until(timeout: 15) { vorn.phase == .connected })
        try #require(vorn.phase == .connected)
        let client = vorn.client
        let project = try await server.addProject("alpha", client: client)
        let store = WorkflowsStore(backend: LiveWorkflowsBackend(client: client))
        store.projects = try await client.call("config:load", as: AppConfigSummary.self).projects ?? []
        store.activeProjectName = project
        let projectPath = try #require(store.projects.first?.path)

        let id = "wf-files-\(UUID().uuidString.prefix(8))"
        try await client.createWorkflow(WorkflowDefinition(
            id: id, name: "Nightly files",
            nodes: [
                WorkflowNode(id: "trigger", type: .trigger, label: "Trigger", config: [
                    "triggerType": "recurring", "cron": "0 3 * * *",
                ]),
                WorkflowNode(id: "say", type: .script, label: "Say", slug: "say", config: [
                    "scriptType": "bash", "scriptContent": "echo hi", "cwd": .string(projectPath),
                ], position: .init(x: 0, y: 120)),
            ],
            edges: [WorkflowEdge(id: "e0", source: "trigger", target: "say")],
            enabled: true, workspaceId: "personal"))
        await store.reload()
        let wf = try #require(store.workflow(id))

        // Export through the save dialog's stand-in, into a temp file.
        let file = server.root.appendingPathComponent("export.json")
        store.workflowFiles = WorkflowFileIO(
            save: { _, contents, _ in
                try contents.write(to: file, atomically: true, encoding: .utf8)
                return true
            },
            pick: { try WorkflowFiles.read(file) })
        store.exportWorkflow(wf)
        #expect(await until { store.toast == "Exported \"Nightly files\"" }, "toast: \(store.toast ?? "-")")
        let text = try String(contentsOf: file, encoding: .utf8)
        #expect(!text.contains(projectPath), "the file still names this machine's path")

        // The row menu: the schedule toggle, then delete.
        store.setEnabled(wf, false)
        #expect(await until { (try? await client.workflow(id: id))??.enabled == false })
        store.delete(wf)
        #expect(await until { (try? await client.workflow(id: id)) ?? nil == nil })

        // Import it back: a new workflow, then the same file again updates it.
        store.toast = nil
        store.importWorkflow()
        #expect(await until { store.toast?.hasPrefix("Imported \"Nightly files\"") == true }, "toast: \(store.toast ?? "-")")
        #expect(store.toast == "Imported \"Nightly files\" — enable it when ready")
        let imported = try #require(try await client.listWorkflows().first { $0.name == "Nightly files" })
        let say = imported.nodes.first { $0.id == "say" }
        #expect(say?.config["cwd"]?.stringValue == projectPath)
        #expect(say?.config["scriptContent"]?.stringValue == "echo hi")

        store.toast = nil
        store.importWorkflow()
        #expect(await until { store.toast == "Updated \"Nightly files\"" }, "toast: \(store.toast ?? "-")")
        #expect(try await client.listWorkflows().filter { $0.name == "Nightly files" }.count == 1)

        // A step still unbound: the toast says what it needs and Review opens it there.
        let hookId = "wf-hook-\(UUID().uuidString.prefix(8))"
        try await client.createWorkflow(WorkflowDefinition(
            id: hookId, name: "Hook",
            nodes: [
                WorkflowNode(id: "trigger", type: .trigger, label: "Trigger", config: ["triggerType": "manual"]),
                WorkflowNode(id: "call", type: .httpRequest, label: "Call", slug: "call", config: [
                    "profileConnectionId": "conn-elsewhere", "method": "GET", "path": "/",
                ], position: .init(x: 0, y: 120)),
            ],
            edges: [WorkflowEdge(id: "e0", source: "trigger", target: "call")],
            workspaceId: "personal"))
        await store.reload()
        store.exportWorkflow(try #require(store.workflow(hookId)))
        #expect(await until { store.toast == "Exported \"Hook\"" }, "toast: \(store.toast ?? "-")")
        store.delete(try #require(store.workflow(hookId)))
        #expect(await until { (try? await client.workflow(id: hookId)) ?? nil == nil })
        store.toast = nil
        store.importWorkflow()
        #expect(await until { store.toast?.hasPrefix("Imported \"Hook\"") == true }, "toast: \(store.toast ?? "-")")
        #expect(store.toast == "Imported \"Hook\" — still needs an HTTP profile", "toast: \(store.toast ?? "-")")
        let review = try #require(store.toastAction)
        #expect(review.label == "Review")
        review.run()
        let reopened = try #require(store.editingWorkflowId)
        #expect(store.importedRequirements?.workflowId == reopened)
        #expect(store.importedRequirements?.requirements.count == 1)
        store.closeEditor()

        // Not a workflow file.
        let junk = server.root.appendingPathComponent("notes.txt")
        try "hello".write(to: junk, atomically: true, encoding: .utf8)
        store.importDroppedWorkflow([junk])
        #expect(await until { store.toast == "That is not a workflow file" }, "toast: \(store.toast ?? "-")")
    }

    @discardableResult
    func until(timeout: TimeInterval = 10, _ check: @MainActor () async -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if await check() { return true }
            try? await Task.sleep(for: .milliseconds(150))
        }
        return false
    }
}
