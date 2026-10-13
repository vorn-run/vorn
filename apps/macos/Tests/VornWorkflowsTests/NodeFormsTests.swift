import Foundation
import Testing
@testable import VornCore
@testable import VornWorkflows

struct VariablePickerTests {
    @Test func typingOpensFiltersAndCloses() {
        #expect(VarPicker.afterTyping("echo {{", cursor: 7, isOpen: false, triggerPos: nil) == .open(at: 5))
        #expect(VarPicker.afterTyping("echo {{ste", cursor: 10, isOpen: true, triggerPos: 5) == .filter("ste"))
        #expect(VarPicker.afterTyping("echo {{a}}", cursor: 10, isOpen: true, triggerPos: 5) == .close)
        #expect(VarPicker.afterTyping("echo x", cursor: 6, isOpen: false, triggerPos: nil) == .none)
    }

    @Test func insertReplacesFromTheTrigger() {
        let r = VarPicker.insert("{{steps.a.output}}", into: "run {{ou now", start: 4, end: 8)
        #expect(r.value == "run {{steps.a.output}} now" && r.cursor == 22)
    }

    @Test func itemsGroupStepsThenCategoriesInOrder() {
        let group = StepVariableGroup(
            nodeId: "a", label: "Draft", slug: "draft", nodeType: .launchAgent,
            keys: [.init(key: "output", label: "Output", description: "d")])
        let items = VarPicker.items(
            stepGroups: [group],
            contextVars: [.init("{{inputs.goal}}", "Goal", .inputs), .init("{{task.title}}", "Title", .task)])
        #expect(items.map(\.pattern) == ["{{steps.draft.output}}", "{{task.title}}", "{{inputs.goal}}"])
        #expect(VarPicker.grouped(items).map(\.name) == ["Draft", "Task", "Run Inputs"])
        #expect(VarPicker.filtered(items, "goal").map(\.key) == ["{{inputs.goal}}"])
    }
}

struct StepEditsTests {
    @Test func numbersFollowTheFormsRules() {
        #expect(StepEdits.loopMax([:], "").num("maxIterations") == 1)
        #expect(StepEdits.loopMax([:], "4.7").num("maxIterations") == 4)
        #expect(StepEdits.approvalTimeout(["timeoutMs": 5], "0")["timeoutMs"] == nil)
        #expect(StepEdits.approvalTimeout([:], "1.5").num("timeoutMs") == 1500)
        #expect(StepEdits.agentTimeout([:], "2.5")?.num("timeoutMs") == 180_000)
        #expect(StepEdits.agentTimeout(["timeoutMs": 60_000], "")?["timeoutMs"] == nil)
        #expect(StepEdits.agentTimeout([:], "x") == nil)
    }

    @Test func worktreeNoneClearsTheBranch() {
        let c: JSONValue = ["branch": "f", "worktreeMode": "new", "useWorktree": true]
        let next = StepEdits.agentWorktreeMode(c, "none", hasBranch: true, priorSteps: 0)
        #expect(next?["branch"] == nil && next?["useWorktree"] == nil && next?.str("worktreeMode") == "none")
        #expect(StepEdits.agentWorktreeMode([:], "new", hasBranch: false, priorSteps: 0) == nil)
    }

    @Test func contextSentinelsAreDroppedWhenTheTriggerIsNotContextual() {
        let c: JSONValue = ["projectName": "{{context.projectName}}", "projectPath": "{{context.projectPath}}", "useWorktree": "fromContext"]
        let next = StepEdits.agentClearContext(c, contextual: false)
        #expect(next?.str("projectName") == "" && next?.str("worktreeMode") == "none" && next?["useWorktree"] == nil)
        #expect(StepEdits.agentClearContext(c, contextual: true) == nil)
    }

    @Test func schemaMustBeAnObject() {
        #expect(StepEdits.parseSchema("[1]") == .error("Schema must be a JSON object"))
        #expect(StepEdits.parseSchema("{") == .error("Not valid JSON"))
        #expect(StepEdits.parseSchema("  ") == .set(nil))
    }

    @Test func onceFieldReadsTypedTimeAsLocal() {
        let utc = FormHelpers.parseISO("2026-11-02T09:30:00.000Z")!
        #expect(TriggerEdits.runAtText(utc) == "11/02/2026, 09:30 AM")
        let berlin = TimeZone(identifier: "Europe/Berlin")!
        #expect(FormHelpers.isoString(TriggerEdits.runAtFromField(utc, zone: berlin)) == "2026-11-02T08:30:00.000Z")
    }
}
