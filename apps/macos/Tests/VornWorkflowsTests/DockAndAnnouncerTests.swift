import Foundation
import Testing
@testable import VornCore
@testable import VornWorkflows

@MainActor @Suite struct DockAndAnnouncerTests {
    func store() async throws -> WorkflowsStore {
        let backend = try FixtureWorkflowsBackend(workflowsJSON: fixture("workflows"), runsJSON: fixture("runs"))
        let store = WorkflowsStore(backend: backend)
        await store.reload()
        return store
    }

    @Test func waitingApprovalsListEachGateWithItsWorkflow() async throws {
        let s = try await store()
        let list = s.waitingApprovals
        #expect(list.count == s.waitingCount)
        let first = try #require(list.first)
        #expect(first.workflow?.id == "wf-release")
        #expect(first.id == "\(first.run.runId):\(first.state.nodeId)")
    }

    @Test func aSignInWaitIsNotAnApproval() async throws {
        let s = try await store()
        var r = try #require(s.runs.first { $0.waitingStep != nil })
        let i = try #require(r.nodeStates.firstIndex { $0.status == .waiting })
        r.nodeStates[i].waitingFor = "signIn"
        s.upsert(r)
        #expect(s.waitingApprovals.isEmpty)
    }

    @Test func anotherWorkspacesGatesStayOut() async throws {
        let s = try await store()
        s.workspaceId = "elsewhere"
        #expect(!s.waitingApprovals.contains { $0.workflow?.id == "wf-release" })
    }

    @Test func showRunOpensTheRunOnTheRunsLanding() async throws {
        let s = try await store()
        s.openWorkflow("wf-lint")
        s.tab = .review
        s.runFilter = .error
        s.showRun("r-x")
        #expect(!s.isEditing)
        #expect(s.tab == .runs)
        #expect(s.runFilter == .all)
        #expect(s.selectedRunId == "r-x")
    }

    @Test func liveUpdatesReachTheAppHook() async throws {
        let s = try await store()
        var seen: [String] = []
        s.onRunUpdate = { seen.append($0.runId) }
        let r = try #require(s.runs.first)
        s.apply(.runUpdated(r))
        s.apply(.runCompleted(r))
        #expect(seen == [r.runId, r.runId])
    }

    // MARK: Announcer

    let workflow: WorkflowDefinition = {
        var wf = try! JSONDecoder().decode([WorkflowDefinition].self, from: fixture("workflows"))[0]
        wf.id = "w"
        wf.name = "Release"
        wf.nodes = nodes
        wf.edges = []
        return wf
    }()

    @Test func aGateIsAnnouncedOnceWhenItStartsWaiting() throws {
        var a = RunAnnouncer()
        let running = try run("running", #"[{"nodeId":"b","status":"running"}]"#)
        #expect(a.announce(running, workflow: workflow).isEmpty)
        let gate = try run("running", #"[{"nodeId":"b","status":"success"},{"nodeId":"g","status":"waiting"}]"#)
        #expect(a.announce(gate, workflow: workflow) == [.wait(AppNotice(
            reason: .waiting, key: "workflow-gate:w:g", title: "Release · awaiting approval", body: "Ship it?",
            target: .run("r1")))])
        #expect(a.announce(gate, workflow: workflow).isEmpty)
    }

    @Test func aSignInWaitSaysSo() throws {
        var a = RunAnnouncer()
        let wait = try run("running", #"[{"nodeId":"b","status":"waiting","waitingFor":"signIn"}]"#)
        guard case .wait(let n) = try #require(a.announce(wait, workflow: workflow).first) else {
            Issue.record("expected a wait")
            return
        }
        #expect(n.title == "Release · awaiting sign-in")
        #expect(n.body == "Build is waiting for you to sign in again")
        #expect(n.key == "workflow-sign-in:w:b")
    }

    @Test func theEndIsAnnouncedOnlyAfterRunning() throws {
        var a = RunAnnouncer()
        let done = try run("success", #"[{"nodeId":"t","status":"success"},{"nodeId":"b","status":"success"},{"nodeId":"g","status":"pending"}]"#)
        #expect(a.announce(done, workflow: workflow).isEmpty)
        _ = a.announce(try run("running"), workflow: workflow)
        #expect(a.announce(done, workflow: workflow) == [.ended(runId: "r1", body: "Workflow \"Release\" completed — 2 steps")])
        _ = a.announce(try run("running"), workflow: workflow)
        let failed = try run("error", #"[{"nodeId":"b","status":"error"}]"#)
        #expect(a.announce(failed, workflow: workflow) == [.ended(runId: "r1", body: "Workflow \"Release\" failed — 1 step")])
    }

    @Test func aRunOfAnUnknownWorkflowStaysQuiet() throws {
        var a = RunAnnouncer()
        let gate = try run("running", #"[{"nodeId":"g","status":"waiting"}]"#)
        #expect(a.announce(gate, workflow: nil).isEmpty)
    }
}
