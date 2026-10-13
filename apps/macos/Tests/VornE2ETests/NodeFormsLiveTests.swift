import Foundation
import Testing
import VornCore
@testable import VornWorkflows

/// Every step form's writes against a throwaway vornd: edited through the editor model, saved, read back.
@MainActor
struct NodeFormsLiveTests {
    @Test func everyFormWritesWhatTodaysFormsWrite() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let vorn = VornStore(dataDir: server.dataDir, readOnly: false)
        vorn.start()
        defer { vorn.stop() }
        #expect(await waitUntil(timeout: 15) { vorn.phase == .connected })
        try #require(vorn.phase == .connected)
        let client = vorn.client
        _ = try await server.addProject("alpha", client: client)
        let backend = LiveWorkflowsBackend(client: client)
        let store = WorkflowsStore(backend: backend)
        store.editorMemory = .ephemeral()
        await store.reload()
        store.projects = try await client.call("config:load", as: AppConfigSummary.self).projects ?? []
        let alpha = try #require(store.projects.first { $0.name == "alpha" })

        let profile = try await backend.rpc("connection:create", [
            "connectorId": "http", "name": "API", "filters": ["baseUrl": "https://api.example.com"],
            "syncIntervalMinutes": 5, "statusMapping": .object([:]),
        ]).str("id") ?? ""
        let github = try await backend.rpc("connection:create", [
            "connectorId": "github", "name": "GitHub", "filters": .object([:]), "syncIntervalMinutes": 15,
            "statusMapping": .object([:]),
        ]).str("id") ?? ""
        let data = NodeFormsData(backend: backend)
        await data.load()
        #expect(data.connections.contains { $0.id == profile && $0.connectorId == "http" })

        let id = "wf-forms-\(UUID().uuidString.prefix(8))"
        func node(_ id: String, _ type: WorkflowNodeType, _ label: String, _ config: JSONValue) -> WorkflowNode {
            WorkflowNode(id: id, type: type, label: label, slug: TemplateVars.slugify(label), config: config)
        }
        let order = ["t", "a", "s", "h", "c", "l", "g", "x", "k"]
        try await client.createWorkflow(WorkflowDefinition(
            id: id, name: "Forms live",
            nodes: [
                WorkflowNode(id: "t", type: .trigger, label: "Trigger", config: ["triggerType": "manual"]),
                node("a", .launchAgent, "Draft", ["agentType": "claude", "projectName": "", "projectPath": ""]),
                node("s", .script, "Build", ["scriptType": "bash", "scriptContent": ""]),
                node("h", .httpRequest, "Notify", ["method": "GET", "url": "", "body": "{{steps.build.output}}"]),
                node("c", .condition, "Passed", ["variable": "", "operator": "equals", "value": ""]),
                node("l", .loop, "Retry", ["mode": "repeat", "maxIterations": 3, "bodyNodeIds": .array([])]),
                node("g", .approval, "Ship it", ["message": ""]),
                node("x", .callConnectorAction, "Open issue", ["connectionId": "", "action": "", "args": .object([:])]),
                node("k", .createTaskFromItem, "Make task", ["project": "fromConnection", "initialStatus": "todo"]),
            ],
            edges: zip(order, order.dropFirst()).map { WorkflowEdge(id: "e-\($0)", source: $0, target: $1) },
            enabled: true))
        await store.reload()
        await store.openWorkflow(id).value
        let editor = try #require(store.editor)
        #expect(editor.nodes.count == 9)

        func edit(_ nodeId: String, _ change: (JSONValue) -> JSONValue?) {
            guard var n = editor.nodes.first(where: { $0.id == nodeId }), let next = change(n.config) else { return }
            n.config = next
            editor.updateNode(n)
        }

        // Trigger: contextual, one select input.
        edit("t") { TriggerEdits.toggleContextual($0) }
        edit("t") { c in
            var input = InputEdits.add([])[0]
            input = InputEdits.setType(input, "select")
            input = InputEdits.setOptions(input, "s, m")
            return TriggerEdits.setInputs(c, [input])
        }

        // Launch agent.
        edit("a") { StepEdits.agentSetAgent($0, "codex") }
        edit("a") { $0.merged(["model": .string("gpt-5")]) }
        edit("a") { StepEdits.agentProject($0, alpha) }
        edit("a") { StepEdits.agentPromptSource($0, "queue", firstTaskId: nil) }
        edit("a") { StepEdits.agentPromptSource($0, "inline", firstTaskId: nil) }
        edit("a") { StepEdits.emptyIsAbsent($0, "prompt", "Write about {{inputs.input}}") }
        edit("a") { StepEdits.agentBranch($0, "feature/x") }
        edit("a") { StepEdits.agentWorktreeMode($0, "new", hasBranch: true, priorSteps: 0) }
        edit("a") { StepEdits.agentToggleHeadless($0) }
        edit("a") { c in
            guard case .set(let schema) = StepEdits.parseSchema(#"{"type":"object","properties":{"verdict":{"type":"string"}}}"#)
            else { return nil }
            return c.merged(["outputSchema": schema])
        }
        edit("a") { StepEdits.agentTimeout($0, "30") }
        edit("a") { StepEdits.agentTimeout($0, "-1") }
        edit("a") { StepEdits.agentArgs($0, " --a   --b ") }
        #expect(StepEdits.agentWorktreeMode(editor.nodes[1].config, "fromStep", hasBranch: true, priorSteps: 0) == nil)

        // Script.
        edit("s") { $0.merged(["scriptType": "python"]) }
        edit("s") { StepEdits.scriptProject($0, alpha) }
        edit("s") { $0.merged(["scriptContent": "print({{steps.draft.verdict}})"]) }
        edit("s") { StepEdits.emptyIsAbsent($0, "secretsFrom", profile) }
        edit("s") { StepEdits.scriptArgs($0, "--x  y") }

        // HTTP.
        edit("h") { $0.merged(["method": "PUT", "url": "/v1/items"]) }
        edit("h") { StepEdits.emptyIsAbsent($0, "profileConnectionId", profile) }
        edit("h") { StepEdits.httpHeaders($0, [("Accept", "a"), ("X-Id", "{{inputs.input}}"), ("Accept", "b")]) }

        // Condition.
        edit("c") { $0.merged(["variable": "{{steps.build.status}}", "operator": "notEquals", "value": "error"]) }

        // Loop.
        edit("l") { $0.merged(["mode": "forEach", "items": "{{steps.draft.verdict}}"]) }
        edit("l") { StepEdits.loopMax($0, "42") }
        edit("l") { StepEdits.loopUntil($0, variable: "{{steps.build.status}}") }

        // Approval.
        edit("g") { $0.merged(["message": "Ship {{steps.draft.output}}?"]) }
        edit("g") { StepEdits.emptyIsAbsent($0, "edit", "{{steps.draft.output}}") }
        edit("g") { StepEdits.emptyIsAbsent($0, "view", "") }
        let gateSteps = StepEdits.approvalSteps(editor.nodes.filter { ["t", "a", "s"].contains($0.id) })
        edit("g") { StepEdits.approvalFeedback($0, steps: gateSteps) }
        edit("g") { StepEdits.approvalRounds($0, steps: gateSteps, "1") }
        edit("g") { StepEdits.approvalTimeout($0, "90") }

        // Connector action.
        edit("x") { StepEdits.connectorConnection($0, github) }
        edit("x") {
            StepEdits.connectorAction($0, "createIssue", actions: [ConnectorActionDef(type: "createIssue", label: "Create issue")])
        }
        edit("x") { StepEdits.connectorArg($0, "title", "{{inputs.input}}") }

        // Create task.
        edit("k") { $0.merged(["project": "alpha", "initialStatus": "in_review"]) }

        // Header: rename (references follow the slug) and the error policy; undo and redo see both.
        let build = try #require(editor.nodes.first { $0.id == "s" })
        editor.updateNode(PanelEdits.setLabel(build, "Compile", others: editor.nodes))
        #expect(editor.nodes.first { $0.id == "h" }?.config.str("body") == "{{steps.compile.output}}")
        editor.updateNode(PanelEdits.setOnError(try #require(editor.nodes.first { $0.id == "c" }), "continue"))
        editor.undo()
        #expect(editor.nodes.first { $0.id == "c" }?.onError == nil)
        editor.redo()
        #expect(editor.nodes.first { $0.id == "c" }?.onError == "continue")

        let saved = await editor.persist()
        let back = try #require(try await backend.getWorkflow(saved.id))
        func cfg(_ id: String) -> JSONValue { back.nodes.first { $0.id == id }?.config ?? .null }

        let t = cfg("t")
        #expect(t.bool("contextual") == true)
        let input = try #require(t["inputs"]?.arrayValue?.first)
        #expect(input.str("key") == "input" && input.str("type") == "select" && input.str("label") == "")
        #expect(input["options"]?.arrayValue?.compactMap { $0.str("value") } == ["s", "m"])

        let a = cfg("a")
        #expect(a.str("agentType") == "codex" && a.str("model") == "gpt-5")
        #expect(a.str("projectName") == "alpha" && a.str("projectPath") == alpha.path)
        #expect(a.str("prompt") == "Write about {{inputs.input}}" && a["taskId"] == nil && a["taskFromQueue"] == nil)
        #expect(a.str("branch") == "feature/x" && a.str("worktreeMode") == "new" && a.bool("useWorktree") == true)
        #expect(a.bool("headless") == true && a.num("timeoutMs") == 1_800_000)
        #expect(a["outputSchema"]?["properties"]?["verdict"]?.str("type") == "string")
        #expect(a["args"]?.arrayValue?.compactMap(\.stringValue) == ["--a", "--b"])

        let s = cfg("s")
        #expect(s.str("scriptType") == "python" && s.str("cwd") == alpha.path && s.str("projectPath") == alpha.path)
        #expect(s.str("scriptContent") == "print({{steps.draft.verdict}})" && s.str("secretsFrom") == profile)
        #expect(s["args"]?.arrayValue?.compactMap(\.stringValue) == ["--x", "y"])
        let compiled = back.nodes.first { $0.id == "s" }
        #expect(compiled?.label == "Compile" && compiled?.slug == "compile")

        let h = cfg("h")
        #expect(h.str("method") == "PUT" && h.str("url") == "/v1/items" && h.str("profileConnectionId") == profile)
        #expect(h["headers"]?.objectValue == ["Accept": "b", "X-Id": "{{inputs.input}}"])
        #expect(h.str("body") == "{{steps.compile.output}}")

        let c = cfg("c")
        #expect(c.str("variable") == "{{steps.compile.status}}" && c.str("operator") == "notEquals" && c.str("value") == "error")
        #expect(back.nodes.first { $0.id == "c" }?.onError == "continue")

        let l = cfg("l")
        #expect(l.str("mode") == "forEach" && l.str("items") == "{{steps.draft.verdict}}" && l.num("maxIterations") == 10)
        #expect(l["until"] == ["variable": "{{steps.compile.status}}", "operator": "equals", "value": ""])

        let g = cfg("g")
        #expect(g.str("message") == "Ship {{steps.draft.output}}?" && g.str("edit") == "{{steps.draft.output}}" && g["view"] == nil)
        #expect(g["feedback"] == ["from": "a", "maxRounds": 2])
        #expect(g.num("timeoutMs") == 90_000)

        let x = cfg("x")
        #expect(x.str("connectionId") == github && x.str("action") == "createIssue" && x.str("actionLabel") == "Create issue")
        #expect(x["args"] == ["title": "{{inputs.input}}"])

        let k = cfg("k")
        #expect(k.str("project") == "alpha" && k.str("initialStatus") == "in_review")
    }

    @discardableResult
    func waitUntil(timeout: TimeInterval = 10, _ check: @MainActor () async -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if await check() { return true }
            try? await Task.sleep(for: .milliseconds(150))
        }
        return false
    }
}
