import Foundation
import Testing
@testable import VornCore
@testable import VornWorkflows

func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
    try JSONDecoder().decode(T.self, from: Data(json.utf8))
}

func fixture(_ name: String) throws -> Data {
    let url = try #require(Bundle.module.url(forResource: name, withExtension: "json", subdirectory: "Fixtures"))
    return try Data(contentsOf: url)
}

func run(_ status: String, _ states: String = "[]", completedAt: String? = nil, extra: String = "") throws -> WorkflowExecution {
    let end = completedAt.map { #","completedAt":"\#($0)""# } ?? ""
    return try decode(
        WorkflowExecution.self,
        #"{"runId":"r1","workflowId":"w","startedAt":"2026-01-01T00:00:00Z","status":"\#(status)","nodeStates":\#(states)\#(end)\#(extra)}"#)
}

let nodes: [WorkflowNode] = try! decode(
    [WorkflowNode].self,
    #"""
    [{"id":"t","type":"trigger","label":"Trigger","config":{"triggerType":"manual"},"position":{"x":0,"y":0}},
     {"id":"b","type":"script","label":"Build","config":{"scriptType":"bash","scriptContent":"# hi\nmake all"},"position":{"x":0,"y":0}},
     {"id":"g","type":"approval","label":"Sign off","config":{"message":"Ship it?"},"position":{"x":0,"y":0}}]
    """#)

@Suite struct PresenterTests {
    @Test func bucketsRuns() throws {
        #expect(RunPresenter.bucket(of: try run("running")) == .running)
        #expect(RunPresenter.bucket(of: try run("running", #"[{"nodeId":"g","status":"waiting"}]"#)) == .waiting)
        #expect(RunPresenter.bucket(of: try run("success")) == .success)
        #expect(RunPresenter.bucket(of: try run("cancelled")) == .error)
    }

    @Test func statusLines() throws {
        #expect(RunPresenter.statusLine(try run("running", #"[{"nodeId":"g","status":"waiting"}]"#), nodes: nodes) == "Waiting at Sign off")
        #expect(RunPresenter.statusLine(try run("running", #"[{"nodeId":"g","status":"waiting","waitingFor":"signIn"}]"#), nodes: nodes)
            == "Waiting for sign-in at Sign off")
        #expect(RunPresenter.statusLine(try run("running", #"[{"nodeId":"b","status":"running"}]"#), nodes: nodes) == "Running Build")
        #expect(RunPresenter.statusLine(try run("error", #"[{"nodeId":"b","status":"error","error":"x"}]"#), nodes: nodes) == "Failed at Build")
        #expect(RunPresenter.statusLine(try run("cancelled"), nodes: nodes) == "Stopped")
        #expect(RunPresenter.statusLine(try run("success"), nodes: nodes) == "Completed")
    }

    @Test func rejectionReadsAsADecision() throws {
        let r = try run("error", #"""
            [{"nodeId":"g","status":"error","rejectedAt":"2026-01-01T00:00:01Z",
              "feedback":[{"round":1,"decision":"reject","comment":"not yet","at":"2026-01-01T00:00:01Z"}]}]
            """#)
        #expect(RunPresenter.isRejected(r))
        #expect(RunPresenter.dotStatus(r) == .cancelled)
        #expect(RunPresenter.statusLine(r, nodes: nodes) == "Rejected at Sign off · not yet")
    }

    @Test func detailLineJoinsTheParts() throws {
        let r = try run("running", #"[{"nodeId":"t","status":"success"},{"nodeId":"b","status":"success"},{"nodeId":"g","status":"waiting"}]"#)
        let now = try #require(TimeFormat.parse("2026-01-01T00:05:00Z"))
        let line = RunPresenter.detailLine(r, workflow: RunWorkflowRef(name: "W", nodes: nodes), deleted: true, now: now)
        #expect(line == "Waiting at Sign off · manual · 1 of 2 steps · deleted · 5m ago")
    }

    @Test func verdictTakesTheLastShortConclusion() throws {
        let r = try run("success", #"""
            [{"nodeId":"a","status":"success","structuredOutput":{"verdict":"ship"}},
             {"nodeId":"b","status":"success","structuredOutput":{"summary":"looks good"}}]
            """#)
        #expect(RunPresenter.verdict(r) == "looks good")
    }
}

@Suite struct TimeFormatTests {
    @Test func durations() {
        #expect(TimeFormat.runDuration("2026-01-01T00:00:00Z", nil) == "running...")
        #expect(TimeFormat.runDuration("2026-01-01T00:00:00.000Z", "2026-01-01T00:00:00.377Z") == "377ms")
        #expect(TimeFormat.runDuration("2026-01-01T00:00:00Z", "2026-01-01T00:00:01.400Z") == "1.4s")
        #expect(TimeFormat.runDuration("2026-01-01T00:00:00Z", "2026-01-01T00:02:13Z") == "2m 13s")
        #expect(TimeFormat.runDuration("2026-01-01T00:00:01Z", "2026-01-01T00:00:00Z") == "—")
    }

    @Test func relativeTimes() throws {
        let now = try #require(TimeFormat.parse("2026-01-03T00:00:00Z"))
        #expect(TimeFormat.relative("2026-01-02T23:59:30Z", now: now) == "Just now")
        #expect(TimeFormat.relative("2026-01-02T23:15:00Z", now: now) == "45m ago")
        #expect(TimeFormat.relative("2026-01-02T21:00:00Z", now: now) == "3h ago")
        #expect(TimeFormat.relative("2026-01-01T00:00:00Z", now: now) == "2d ago")
        #expect(TimeFormat.relative("garbage", now: now) == "Unknown")
    }

    @Test func elapsedTimer() throws {
        let now = try #require(TimeFormat.parse("2026-01-01T00:01:05Z"))
        #expect(TimeFormat.elapsed(since: "2026-01-01T00:00:00Z", now: now) == "1:05")
    }
}

@Suite struct GateRoundsTests {
    @Test func roundsNeedAFeedbackSource() throws {
        let none = try decode(JSONValue.self, #"{"message":"m"}"#)
        #expect(!GateRounds.canRequestChanges(none, round: 1))
        #expect(GateRounds.label(none, round: 1) == nil)
    }

    @Test func roundsStopAtTheLimit() throws {
        let config = try decode(JSONValue.self, #"{"feedback":{"from":"b","maxRounds":2}}"#)
        #expect(GateRounds.canRequestChanges(config, round: 1))
        #expect(!GateRounds.canRequestChanges(config, round: 2))
        #expect(GateRounds.label(config, round: 2) == "round 2 of 2")
    }

    @Test func maxRoundsIsClamped() throws {
        #expect(GateRounds.maxRounds(try decode(JSONValue.self, #"{"feedback":{"maxRounds":99}}"#)) == 10)
        #expect(GateRounds.maxRounds(try decode(JSONValue.self, #"{"feedback":{"maxRounds":0}}"#)) == 1)
        #expect(GateRounds.maxRounds(nil) == 3)
    }
}

@Suite struct CanvasTests {
    @Test func cardTextFollowsNodeShell() {
        #expect(NodeCardText.subtitle(nodes[0]) == "Click to run")
        #expect(NodeCardText.subtitle(nodes[1]) == "bash")
        #expect(NodeCardText.footer(nodes[1])?.text == "make all")
        #expect(NodeCardText.footer(nodes[1])?.mono == true)
        #expect(NodeCardText.subtitle(nodes[2]) == "Waits for approval")
        #expect(NodeCardText.footer(nodes[2])?.text == "Ship it?")
    }

    @Test func seededPositionsStackIntoATrunk() throws {
        let wf = try decode(
            WorkflowDefinition.self,
            #"""
            {"id":"w","name":"W","icon":"Zap","iconColor":"#fff","enabled":true,
             "nodes":[{"id":"t","type":"trigger","label":"T","config":{},"position":{"x":0,"y":0}},
                      {"id":"a","type":"script","label":"A","config":{},"position":{"x":0,"y":0}},
                      {"id":"b","type":"script","label":"B","config":{},"position":{"x":0,"y":0}}],
             "edges":[{"id":"1","source":"t","target":"a"},{"id":"2","source":"a","target":"b"}]}
            """#)
        let p = CanvasLayout.positions(wf)
        #expect(p["t"]!.y < p["a"]!.y)
        #expect(p["a"]!.y < p["b"]!.y)
        #expect(p["t"]!.x == p["b"]!.x)
    }
}

@MainActor
@Suite struct StoreTests {
    func store() async throws -> (WorkflowsStore, FixtureWorkflowsBackend) {
        let backend = try FixtureWorkflowsBackend(workflowsJSON: fixture("workflows"), runsJSON: fixture("runs"))
        let store = WorkflowsStore(backend: backend)
        await store.reload()
        return (store, backend)
    }

    @Test func loadsWorkflowsAndRunsNewestFirst() async throws {
        let (s, _) = try await store()
        #expect(s.workflows.count == 7)
        #expect(s.runs.count == 4)
        #expect(s.runs.map(\.startedAt) == s.runs.map(\.startedAt).sorted(by: >))
        #expect(s.selectedRun?.runId == s.runs.first?.runId)
    }

    @Test func filtersRunsAndCountsGates() async throws {
        let (s, _) = try await store()
        #expect(s.waitingCount == 1)
        #expect(s.waitingCount(for: "wf-release") == 1)
        s.runFilter = .error
        #expect(s.visibleRuns.map(\.workflowId) == ["wf-flaky"])
        s.tab = .review
        #expect(s.visibleRuns.map(\.workflowId) == ["wf-release"])
    }

    @Test func sidebarFilterSplitsScheduled() async throws {
        let (s, _) = try await store()
        s.sidebarFilter = .scheduled
        #expect(Set(s.sidebarWorkflows.map(\.id)).isSuperset(of: ["wf-nightly", "wf-weekly"]))
        #expect(!s.sidebarWorkflows.contains { $0.id == "wf-lint" })
        s.sidebarFilter = .manual
        #expect(s.sidebarWorkflows.allSatisfy { !$0.isScheduled })
    }

    @Test func upsertKeepsTheNameAndOrder() async throws {
        let (s, _) = try await store()
        var live = try #require(s.runs.first { $0.workflowId == "wf-flaky" })
        live.runId = "new"
        live.startedAt = "2099-01-01T00:00:00Z"
        live.workflowName = nil
        s.upsert(live, select: true)
        #expect(s.runs.first?.runId == "new")
        #expect(s.runs.first?.workflowName == "Flaky integration")
        #expect(s.selectedRunId == "new")
    }

    @Test func mergeKeepsLiveRunsTheListingLacks() async throws {
        let (s, _) = try await store()
        var live = try #require(s.runs.first)
        live.runId = "live"
        live.status = .running
        s.upsert(live)
        var settled = live
        settled.runId = "gone"
        settled.status = .success
        s.upsert(settled)
        s.mergePersisted(Array(s.runs.filter { $0.runId != "live" && $0.runId != "gone" }))
        #expect(s.runs.contains { $0.runId == "live" })
        #expect(!s.runs.contains { $0.runId == "gone" })
    }

    @Test func runNowWithoutATriggerToasts() async throws {
        let (s, backend) = try await store()
        var wf = try #require(s.workflow("wf-lint"))
        wf.nodes.removeAll { $0.type == .trigger }
        s.runNow(wf)
        #expect(s.toast == "\"Lint the repo\" has no trigger — add one in the editor first")
        #expect(await backend.writes.isEmpty)
    }

    @Test func togglingAScheduleIsOptimistic() async throws {
        let (s, backend) = try await store()
        let wf = try #require(s.workflow("wf-nightly"))
        s.setEnabled(wf, false)
        #expect(s.workflow("wf-nightly")?.enabled == false)
        try await Task.sleep(for: .milliseconds(50))
        #expect(await backend.writes.contains { $0.hasPrefix("setEnabled") })
    }

    @Test func answeringAGateGoesToTheBackend() async throws {
        let (s, backend) = try await store()
        let r = try #require(s.runs.first { $0.waitingStep != nil })
        let ok = await s.resolveGate(r, nodeId: r.waitingStep!.nodeId, decision: .approve)
        #expect(ok)
        #expect(await backend.writes.contains { $0.hasPrefix("resolveGate") })
    }

    @Test func openingAWorkflowLoadsItsRuns() async throws {
        let (s, _) = try await store()
        await s.openWorkflow("wf-lint").value
        #expect(s.editingWorkflow?.id == "wf-lint")
        #expect(s.workflowRuns.count == 2)
        s.showAllRuns()
        #expect(s.editingWorkflowId == nil)
        #expect(s.workflowRuns.isEmpty)
    }
}
