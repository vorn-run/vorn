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
        try await reviewPage(client, store: store)
        #expect(await store.signIn(connectionId: "no-such-connection")
            == SignInResult(ok: false, error: "This connection does not sign in through a Vorn window."))
    }

    /// A gate whose review page the step before it built, served by vornd for this round only.
    private func reviewPage(_ client: VornClient, store: WorkflowsStore) async throws {
        let id = "wf-review-\(UUID().uuidString.prefix(8))"
        try await client.createWorkflow(WorkflowDefinition(
            id: id, name: "Review it",
            nodes: [
                WorkflowNode(id: "trigger", type: .trigger, label: "Trigger", config: ["triggerType": "manual"]),
                WorkflowNode(id: "find", type: .script, label: "Find issues", slug: "find", config: [
                    "scriptType": "bash", "scriptContent": "echo '<h1>Findings</h1><p>found</p>'",
                ], position: .init(x: 0, y: 120)),
                WorkflowNode(id: "check", type: .approval, label: "Check findings", slug: "check", config: [
                    "message": "Look over the findings.", "view": "{{steps.find.output}}",
                    "feedback": ["from": "find", "maxRounds": 2],
                ], position: .init(x: 0, y: 240)),
            ],
            edges: [
                WorkflowEdge(id: "e0", source: "trigger", target: "find"),
                WorkflowEdge(id: "e1", source: "find", target: "check"),
            ],
            workspaceId: "personal"))
        _ = try await client.runWorkflow(id: id)
        var run: WorkflowExecution?
        let parked = await until(timeout: 20) {
            run = try? await client.listRuns(workflowId: id, limit: 5).first
            return run?.nodeStates.contains { $0.nodeId == "check" && $0.viewToken != nil } == true
        }
        #expect(parked, "run: \(String(describing: run?.status))")
        let parkedRun = try #require(run)
        let gate = try #require(parkedRun.nodeStates.first { $0.nodeId == "check" })
        let token = try #require(gate.viewToken)

        let url = try #require(await store.gateViewURL(runId: parkedRun.runId, nodeId: "check", token: token))
        let (body, response) = try await URLSession.shared.data(from: url)
        #expect((response as? HTTPURLResponse)?.statusCode == 200)
        #expect(String(decoding: body, as: UTF8.self).contains("<h1>Findings</h1>"))
        let csp = (response as? HTTPURLResponse)?.value(forHTTPHeaderField: "Content-Security-Policy") ?? ""
        #expect(csp.contains("default-src 'none'"), "csp: \(csp)")

        let wrong = try #require(await store.gateViewURL(runId: parkedRun.runId, nodeId: "check", token: "nope"))
        let (_, refused) = try await URLSession.shared.data(from: wrong)
        #expect((refused as? HTTPURLResponse)?.statusCode == 404)

        store.openReview(parkedRun, nodeId: "check", workflowName: "Review it")
        #expect(store.surfaces.review == GateReviewTarget(runId: parkedRun.runId, nodeId: "check", workflowName: "Review it"))
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
