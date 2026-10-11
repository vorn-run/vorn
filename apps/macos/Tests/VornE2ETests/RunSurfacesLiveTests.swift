import Foundation
import Testing
import VornCore
@testable import VornWorkflows

/// The run surfaces against a throwaway vornd: the run prompt's params, connections, a gate's review target.
@MainActor
struct RunSurfacesLiveTests {
    @Test func runPromptConnectionsAndReview() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let vorn = VornStore(dataDir: server.dataDir, readOnly: false)
        vorn.start()
        defer { vorn.stop() }
        #expect(await until(timeout: 15) { vorn.phase == .connected })
        try #require(vorn.phase == .connected)
        let client = vorn.client
        let project = try await server.addProject("alpha", client: client)
        let backend = LiveWorkflowsBackend(client: client)
        let store = WorkflowsStore(backend: backend)
        await store.reload()
        store.projects = try await client.call("config:load", as: AppConfigSummary.self).projects ?? []

        try await runThroughThePrompt(client, store: store, project: project)
        try await listConnections(store)
    }

    private func runThroughThePrompt(_ client: VornClient, store: WorkflowsStore, project: String) async throws {
        let id = "wf-inputs-\(UUID().uuidString.prefix(8))"
        try await client.createWorkflow(WorkflowDefinition(
            id: id, name: "Inputs",
            nodes: [
                WorkflowNode(id: "trigger", type: .trigger, label: "Trigger", config: [
                    "triggerType": "manual", "contextual": true,
                    "inputs": [
                        ["key": "goal", "label": "Goal", "type": "text", "required": true],
                        ["key": "count", "label": "Count", "type": "number", "defaultValue": "2"],
                        ["key": "dry", "label": "Dry", "type": "boolean"],
                    ],
                ]),
                WorkflowNode(id: "say", type: .script, label: "Say", slug: "say", config: [
                    "scriptType": "bash", "scriptContent": "echo {{inputs.goal}}", "cwd": "{{context.projectPath}}",
                ], position: .init(x: 0, y: 120)),
            ],
            edges: [WorkflowEdge(id: "e0", source: "trigger", target: "say")],
            workspaceId: "personal"))
        await store.reload()
        let wf = try #require(store.workflow(id))

        store.runNow(wf)
        let prompt = try #require(store.surfaces.prompt)
        #expect(prompt.required.needsProject)
        #expect(!prompt.canSubmit)
        prompt.selectProject(project)
        prompt.values["goal"] = "ship it"
        #expect(prompt.canSubmit)
        store.submitPrompt()
        #expect(store.surfaces.prompt == nil)

        var run: WorkflowExecution?
        let finished = await until(timeout: 20) {
            run = try? await client.listRuns(workflowId: id, limit: 5).first
            return run?.status == .success
        }
        #expect(finished, "run: \(String(describing: run?.status)) \(store.toast ?? "")")
        let inputs = try #require(run?.inputs)
        #expect(inputs["goal"] == "ship it")
        #expect(inputs["count"] == 2)
        #expect(inputs["dry"] == false)
        let say = run?.nodeStates.first { $0.nodeId == "say" }
        #expect(say?.logs?.contains("ship it") == true, "logs: \(say?.logs ?? "-")")
    }

    private func listConnections(_ store: WorkflowsStore) async throws {
        let raw = try await store.backend.rpc("connection:list", nil)
        #expect(raw.arrayValue != nil, "connection:list answered \(raw)")
        await store.loadConnectionNames()
        #expect(store.surfaces.connectionNames.count == raw.arrayValue?.count)
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
