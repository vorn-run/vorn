import Foundation
import Testing
@testable import VornCore
@testable import VornWorkflows

private func workflow(trigger: JSONValue, steps: [WorkflowNode] = []) -> WorkflowDefinition {
    WorkflowDefinition(
        id: "wf", name: "WF", nodes: [WorkflowNode(id: "t", type: .trigger, label: "Trigger", config: trigger)] + steps)
}

private let alpha = Project(name: "alpha", path: "/p/alpha")
private let beta = Project(name: "beta", path: "/p/beta")

@Suite struct RunInputsTests {
    @Test func needsPromptForInputsOrAContextualRunWithoutSource() {
        let plain = workflow(trigger: ["triggerType": "manual"])
        #expect(!RunInputs.needsPrompt(plain))
        let withInputs = workflow(trigger: ["triggerType": "manual", "inputs": [["key": "goal"]]])
        #expect(RunInputs.needsPrompt(withInputs))
        let contextual = workflow(trigger: ["triggerType": "manual", "contextual": true])
        #expect(RunInputs.needsPrompt(contextual))
        #expect(!RunInputs.needsPrompt(contextual, context: ManualRunContext(source: ["id": "s"])))
        // Inputs on a non-manual trigger and keys a template cannot reference do not count.
        #expect(!RunInputs.needsPrompt(workflow(trigger: ["triggerType": "recurring", "inputs": [["key": "a"]]])))
        #expect(!RunInputs.needsPrompt(workflow(trigger: ["triggerType": "manual", "inputs": [["key": "a-b"]]])))
    }

    @Test func parsesEveryKindAndItsInitialValue() {
        let wf = workflow(trigger: ["triggerType": "manual", "inputs": [
            ["key": "t", "label": "Title", "required": true],
            ["key": "n", "type": "number", "defaultValue": "2.5"],
            ["key": "b", "type": "boolean", "defaultValue": "true"],
            ["key": "s", "type": "select", "options": [["value": "x", "label": "X"], ["value": "y"]]],
            ["key": "p", "type": "project"], ["key": "br", "type": "branch"], ["key": "ta", "type": "textarea"],
            ["key": "odd", "type": "mystery"],
        ]])
        let defs = RunInputs.inputs(wf)
        #expect(defs.map(\.type) == [.text, .number, .boolean, .select, .project, .branch, .textarea, .text])
        #expect(defs[0].displayLabel == "Title" && defs[1].displayLabel == "n")
        #expect(defs[3].options.map(\.label) == ["X", "y"])
        let values = RunInputs.initialValues(defs)
        #expect(values["n"] == .number(2.5))
        #expect(values["b"] == .bool(true))
        #expect(values["t"] == .string(""))
    }

    @Test func validatesRequiredAnswers() {
        let text = WorkflowInputDef(key: "a", required: true)
        let flag = WorkflowInputDef(key: "f", type: .boolean, required: true)
        let num = WorkflowInputDef(key: "n", type: .number)
        #expect(!RunInputs.areValid([text], ["a": "  "]))
        #expect(RunInputs.areValid([text], ["a": "x"]))
        #expect(RunInputs.areValid([flag], ["f": false]))
        #expect(!RunInputs.areValid([flag], [:]))
        #expect(RunInputs.areValid([num], ["n": ""]))
        #expect(RunInputs.parseNumber(" 4 ") == .number(4))
        #expect(RunInputs.parseNumber("4x") == .string(""))
        #expect(RunInputs.parseNumber("") == .string(""))
    }

    @Test func detectsTheSourceFieldsStepsReference() {
        let wf = workflow(trigger: ["triggerType": "manual", "contextual": true], steps: [
            WorkflowNode(id: "s", type: .script, label: "S", config: ["scriptContent": "cd {{context.cwd}}"]),
            WorkflowNode(id: "a", type: .launchAgent, label: "A", config: ["useWorktree": "fromContext"]),
        ])
        #expect(RunInputs.requiredFields(wf) == .init(needsProject: true, needsBranch: true, needsWorktree: true))
    }
}

@Suite @MainActor struct RunPromptTests {
    @Test func contextualParamsCarryASourceFromTheFields() {
        let wf = workflow(trigger: ["triggerType": "manual", "contextual": true, "inputs": [["key": "goal", "required": true]]])
        let prompt = RunPrompt(workflow: wf, context: nil, targetNodeId: "n2", projects: [alpha, beta])
        #expect(prompt.required.needsProject)
        #expect(prompt.projectName == "alpha")
        #expect(!prompt.canSubmit)
        prompt.values["goal"] = "go"
        prompt.selectProject("beta")
        prompt.branch = "dev"
        prompt.useWorktree = true
        #expect(prompt.canSubmit)
        let params = prompt.params(now: Date(timeIntervalSince1970: 1.5))
        #expect(params["workflowId"] == "wf")
        #expect(params["targetNodeId"] == "n2")
        let source = params["context"]?["source"]
        #expect(source?["id"] == "prompt:wf:1500")
        #expect(source?["projectName"] == "beta" && source?["projectPath"] == "/p/beta")
        #expect(source?["branch"] == "dev" && source?["isWorktree"] == true && source?["agentType"] == "shell")
        #expect(params["context"]?["inputs"] == ["goal": "go"])
    }

    @Test func aCallersContextPassesThroughWithTheInputs() {
        let wf = workflow(trigger: ["triggerType": "manual", "contextual": true, "inputs": [["key": "n", "type": "number"]]])
        let ctx = ManualRunContext(task: ["id": "task-1"])
        let prompt = RunPrompt(workflow: wf, context: ctx, targetNodeId: nil, projects: [])
        #expect(!prompt.required.needsProject)
        prompt.values["n"] = RunInputs.parseNumber("3")
        let params = prompt.params()
        #expect(params["context"] == ["task": ["id": "task-1"], "inputs": ["n": 3]])
        #expect(params["targetNodeId"] == nil)
    }

    @Test func storeOpensThePromptThenRunsItsParams() async throws {
        let wf = workflow(trigger: ["triggerType": "manual", "inputs": [["key": "goal"]]])
        let backend = FixtureWorkflowsBackend(workflows: [wf], runs: [])
        let store = WorkflowsStore(backend: backend)
        store.projects = [alpha, beta]
        store.runNow(wf)
        let prompt = try #require(store.surfaces.prompt)
        #expect(prompt.projects.map(\.name) == ["alpha", "beta"])
        prompt.values["goal"] = "x"
        store.submitPrompt()
        #expect(store.surfaces.prompt == nil)
        for _ in 0..<50 where await backend.writes.isEmpty { try await Task.sleep(for: .milliseconds(10)) }
        #expect(await backend.writes == ["rpc workflow:run"])
    }

    @Test func aWorkflowWithoutATriggerToasts() {
        let store = WorkflowsStore(backend: FixtureWorkflowsBackend(workflows: [], runs: []))
        store.runNow(WorkflowDefinition(id: "x", name: "Bare"))
        #expect(store.toast == "\"Bare\" has no trigger — add one in the editor first")
        #expect(store.surfaces.prompt == nil)
    }
}

@Suite struct OrderedJSONNumberTests {
    @Test func printsNumbersAsJavaScriptDoes() {
        let cases: [(Double, String)] = [
            (0, "0"), (-0.0, "0"), (3, "3"), (-42, "-42"), (2.5, "2.5"), (0.1, "0.1"), (1e-6, "0.000001"),
            (1e-7, "1e-7"), (1.5e-10, "1.5e-10"), (123456789012, "123456789012"), (1e20, "100000000000000000000"),
            (1e21, "1e+21"), (9.3e18, "9300000000000000000"), (1.2345e25, "1.2345e+25"), (0.000123, "0.000123"),
            (.infinity, "null"),
        ]
        for (n, js) in cases { #expect(OJSON.format(n) == js, "\(n)") }
    }
}

@Suite @MainActor struct MissedSchedulesTests {
    @Test func readsBothWireShapes() {
        let list = MissedSchedule.list([
            ["workflowId": "a", "workflowName": "A", "missedAt": "2026-01-01T00:00:00Z"],
            ["workflow": ["id": "b", "name": "B"], "scheduledFor": "2026-01-02T00:00:00Z"],
            ["workflowId": "c"],
            ["nothing": true],
        ])
        #expect(list.map(\.workflowId) == ["a", "b", "c"])
        #expect(list.map(\.name) == ["A", "B", "c"])
        #expect(list[1].scheduledFor == "2026-01-02T00:00:00Z")
        #expect(list[0].scheduledLabel.hasPrefix("Scheduled for "))
        #expect(MissedSchedule.list("nope").isEmpty)
    }

    @Test func decodesThePush() {
        let event = WorkflowEvent(.notification(method: "scheduler:missed", params: [["workflowId": "a"]]))
        guard case .schedulerMissed(let json)? = event else {
            Issue.record("not a missed push")
            return
        }
        #expect(MissedSchedule.list(json).map(\.workflowId) == ["a"])
        #expect(WorkflowEvent.topics.contains("scheduler:missed"))
    }

    @Test func runsTheSelectedOnesThatStillExist() async throws {
        let wf = WorkflowDefinition(
            id: "a", name: "A", nodes: [WorkflowNode(id: "t", type: .trigger, label: "T", config: ["triggerType": "recurring"])])
        let backend = FixtureWorkflowsBackend(workflows: [wf], runs: [])
        let store = WorkflowsStore(backend: backend)
        await store.reload()
        store.showMissed([
            MissedSchedule(workflowId: "a", name: "A", scheduledFor: ""),
            MissedSchedule(workflowId: "gone", name: "Gone", scheduledFor: ""),
            MissedSchedule(workflowId: "b", name: "B", scheduledFor: ""),
        ])
        #expect(store.surfaces.missedSelected == ["a", "gone", "b"])
        #expect(store.surfaces.hasModal)
        store.toggleMissed("b")
        store.runMissedSelected()
        #expect(store.surfaces.missed.isEmpty)
        for _ in 0..<50 where await backend.writes.isEmpty { try await Task.sleep(for: .milliseconds(10)) }
        #expect(await backend.writes == ["rpc workflow:run"])

        store.showMissed([MissedSchedule(workflowId: "a", name: "A", scheduledFor: "")])
        store.skipMissed()
        #expect(store.surfaces.missed.isEmpty)
    }
}

@Suite struct GateReviewTests {
    @Test func producerIsTheStepThePageTemplateReads() {
        let nodes = [WorkflowNode(id: "f", type: .script, label: "Find issues", slug: "find", config: [:])]
        #expect(GateReviewModal.producer(view: "{{ steps.find.output }}", nodes: nodes) == "Find issues")
        #expect(GateReviewModal.producer(view: "{{steps.other.output}}", nodes: nodes) == nil)
        #expect(GateReviewModal.producer(view: "<h1>static</h1>", nodes: nodes) == nil)
        #expect(GateReviewModal.producer(view: nil, nodes: nodes) == nil)
    }

    @Test func signInButtonFindsTheConnectionAStepActsThrough() {
        let action = WorkflowNode(id: "a", type: .callConnectorAction, label: "A", config: ["connectionId": "c1"])
        let poll = WorkflowNode(id: "t", type: .trigger, label: "T", config: ["triggerType": "connectorPoll", "connectionId": "c2"])
        let manual = WorkflowNode(id: "m", type: .trigger, label: "M", config: ["triggerType": "manual", "connectionId": "c3"])
        #expect(SignInButton.connectionId(action) == "c1")
        #expect(SignInButton.connectionId(poll) == "c2")
        #expect(SignInButton.connectionId(manual) == nil)
        #expect(SignInButton.connectionId(nil) == nil)
    }
}

@Suite struct BrowserSignInTests {
    private let answer: JSONValue = [
        "name": "Tracker",
        "browser": [
            "signInUrl": "https://tracker.example.com/login",
            "origins": ["https://tracker.example.com", "https://*.cdn.example.com"],
            "check": [
                "url": "https://tracker.example.com/api/me",
                "headers": ["x-requested-with": "vorn", "Cookie": "a=b", "sec-fetch-mode": "cors"],
                "identity": ["user.email", "user.id"],
            ],
        ],
    ]

    @Test func readsTheTargetAndDropsHeadersThePageMayNotSet() throws {
        let target = try #require(BrowserSignIn(answer))
        #expect(target.name == "Tracker")
        #expect(target.signInURL.absoluteString == "https://tracker.example.com/login")
        #expect(target.checkHeaders == ["x-requested-with": "vorn"])
        #expect(target.requestHeaders == ["x-requested-with": "vorn", "accept": "application/json"])
        #expect(BrowserSignIn(.null) == nil)
    }

    @Test func checksStayOnTheDeclaredOrigins() {
        let origins = ["https://tracker.example.com", "https://*.cdn.example.com"]
        #expect(BrowserSignIn.within(origins, URL(string: "https://tracker.example.com/api")!))
        #expect(BrowserSignIn.within(origins, URL(string: "https://a.cdn.example.com/x")!))
        #expect(!BrowserSignIn.within(origins, URL(string: "https://cdn.example.com/x")!))
        #expect(!BrowserSignIn.within(origins, URL(string: "http://tracker.example.com/api")!))
        #expect(!BrowserSignIn.within(origins, URL(string: "https://tracker.example.com:8443/api")!))
        #expect(!BrowserSignIn.within(origins, URL(string: "https://evil.com/api")!))
    }

    @Test func identityNamesTheFirstValueAndTheRestInParentheses() {
        let paths = ["user.email", "user.id", "user.none"]
        #expect(BrowserSignIn.identity(#"{"user":{"email":"a@b.c","id":42}}"#, paths: paths) == "a@b.c (42)")
        #expect(BrowserSignIn.identity(#"{"user":{"email":" ","id":7}}"#, paths: paths) == "7")
        #expect(BrowserSignIn.identity(#"{"user":{"email":true}}"#, paths: paths) == nil)
        #expect(BrowserSignIn.identity("not json", paths: paths) == nil)
    }
}
