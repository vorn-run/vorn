import Foundation
import Testing
@testable import VornApp
import VornCore
@testable import VornWorkflows

/// Global approvals and notifications against a throwaway vornd: a gate shows in the dock from any
/// view and is answered there, and a bell, a program's notice and the run's end reach the notifier.
@MainActor
struct ApprovalsLiveTests {
    @Test func dockAnswersGatesAndTheNotifierHearsEverything() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let store = VornStore(dataDir: server.dataDir, readOnly: false)
        store.start()
        defer { store.stop() }
        #expect(await liveUntil(timeout: 15) { store.phase == .connected })
        try #require(store.phase == .connected)

        let model = AppModel()
        model.persistsSidebar = false
        let features = FeatureStores(client: store.client)
        features.bind(store: store, model: model)
        var shown: [(title: String, body: String, target: AppNotice.Target)] = []
        var tones: [NotificationReason] = []
        let notifier = features.notifier
        notifier.permitted = { true }
        notifier.hasFocus = { false }
        notifier.show = { shown.append(($0, $1, $2)) }
        notifier.playTone = { reason, _ in tones.append(reason) }
        try await store.setDefault("notifications", ["enabled": true, "soundEnabled": true, "soundVolume": 0.4])
        #expect(store.defaults?.notifications?.wantsBell == true)

        await features.workflows.start()
        defer { features.workflows.stop() }
        try await gate(store.client, features: features, model: model, shown: { shown })
        #expect(tones.contains(.waiting))

        // A bell in a shell, and a program's own notice, both open that session when clicked.
        let dir = server.root.appendingPathComponent("alpha")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let shell = try await store.createShell(cwd: dir.path, project: nil, worktree: nil)
        try await Task.sleep(for: .seconds(1))
        try await store.write(shell.id, "printf '\\a'\r")
        #expect(await liveUntil(timeout: 15) { shown.contains { $0.title == "Shell notification" } })
        try await store.write(shell.id, "printf '\\033]777;notify;Build;All green\\a'\r")
        #expect(await liveUntil(timeout: 15) { shown.contains { $0.title == "Build" && $0.body == "All green" } },
                "shown: \(shown.map(\.title))")
        #expect(tones.contains(.bell))

        model.mainViewMode = .workflows
        notifier.open(.session(shell.id))
        #expect(model.mainViewMode == .sessions)
        #expect(model.selectedSessionID == shell.id)
    }

    private func gate(_ client: VornClient, features: FeatureStores, model: AppModel,
                      shown: () -> [(title: String, body: String, target: AppNotice.Target)]) async throws {
        let store = features.workflows
        let id = "wf-gate-\(UUID().uuidString.prefix(8))"
        try await client.createWorkflow(WorkflowDefinition(
            id: id, name: "Ship",
            nodes: [
                WorkflowNode(id: "trigger", type: .trigger, label: "Trigger", config: ["triggerType": "manual"]),
                WorkflowNode(id: "ok", type: .approval, label: "Sign off", slug: "ok",
                             config: ["message": "Ship it?"], position: .init(x: 0, y: 120)),
            ],
            edges: [WorkflowEdge(id: "e0", source: "trigger", target: "ok")],
            workspaceId: "personal"))
        await store.reload()
        _ = try await client.runWorkflow(id: id)

        // The dock lists the gate from the sessions view, with what it asks.
        #expect(model.mainViewMode == .sessions)
        #expect(await liveUntil(timeout: 20) { store.waitingApprovals.contains { $0.workflow?.id == id } })
        let waiting = try #require(store.waitingApprovals.first { $0.workflow?.id == id })
        #expect(waiting.message == "Ship it?")
        #expect(await liveUntil(timeout: 10) { shown().contains { $0.title == "Ship · awaiting approval" } })
        #expect(shown().first { $0.title == "Ship · awaiting approval" }?.body == "Ship it?")

        // Clicking the pill opens the run; Approve answers the gate and the run finishes.
        features.showRun(waiting.run.runId, model: model)
        #expect(model.mainViewMode == .workflows)
        #expect(store.selectedRunId == waiting.run.runId)
        model.mainViewMode = .sessions
        #expect(await store.resolveGate(waiting.run, nodeId: waiting.state.nodeId, decision: .approve))
        #expect(await liveUntil(timeout: 20) { !store.waitingApprovals.contains { $0.workflow?.id == id } })
        #expect(await liveUntil(timeout: 20) {
            shown().contains { $0.title == "Vorn" && $0.body == "Workflow \"Ship\" completed — 2 steps" }
        }, "shown: \(shown().map(\.body))")
    }
}
