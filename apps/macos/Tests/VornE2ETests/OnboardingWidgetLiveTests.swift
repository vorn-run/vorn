import Foundation
import Testing
@testable import VornApp
import VornCore

/// OnboardingModal.tsx's first-launch rule and Widget.tsx's rows, against a throwaway vornd.
@MainActor
struct OnboardingWidgetLiveTests {
    private func connect(_ server: TestVornd) async throws -> VornStore {
        let store = VornStore(dataDir: server.dataDir, readOnly: false)
        store.start()
        #expect(await liveUntil(timeout: 15) { store.phase == .connected })
        try #require(store.phase == .connected)
        return store
    }

    private func savedFlag(_ client: VornClient) async -> JSONValue? {
        (try? await client.call("config:load"))?["defaults"]?["hasSeenOnboarding"]
    }

    @Test func guideOpensOnAFreshDataDirAndRemembersItWasSeen() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let store = try await connect(server)
        defer { store.stop() }

        let model = AppModel()
        model.persistsSidebar = false
        OnboardingGate.bind(model: model, store: store)
        #expect(await savedFlag(store.client) == nil)
        #expect(await OnboardingGate.openIfUnseen(model: model, store: store, timeout: .seconds(10)))
        #expect(model.overlay == .onboarding)
        #expect(model.onboardingStep == 0)

        // Paging does not save; closing does, as App.tsx's onClose.
        model.onboardingStep = Onboarding.next(model.onboardingStep)
        #expect(await savedFlag(store.client) == nil)
        model.escape()
        #expect(model.overlay == nil)
        #expect(await liveUntil { await savedFlag(store.client)?.numberValue == Double(Onboarding.version) })
        #expect(await liveUntil { store.defaults?.hasSeenOnboarding?.numberValue == Double(Onboarding.version) })

        // The next launch keeps it closed.
        let again = AppModel()
        again.persistsSidebar = false
        #expect(await OnboardingGate.openIfUnseen(model: again, store: store, timeout: .seconds(10)) == false)
        #expect(again.overlay == nil)

        // A guide newer than the one seen opens again.
        try await store.setDefault("hasSeenOnboarding", 1)
        #expect(await OnboardingGate.openIfUnseen(model: again, store: store, timeout: .seconds(10)))
        #expect(again.overlay == .onboarding)
    }

    @Test func widgetListsTheDaemonsSessionsWithTheirStatuses() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let store = try await connect(server)
        defer { store.stop() }

        let feed = WidgetFeed(store: store)
        #expect(feed.agents.isEmpty)
        let cwd = server.root.appendingPathComponent("w")
        try FileManager.default.createDirectory(at: cwd, withIntermediateDirectories: true)
        let a = try await store.createShell(cwd: cwd.path)
        let b = try await store.createShell(cwd: cwd.path)
        #expect(await liveUntil { feed.agents.count == 2 })

        // The rows are vornd's own list, in its order, with its statuses.
        let listed = try await store.client.call("terminal:listActive")
        let daemon = (listed.arrayValue ?? []).compactMap { s -> (String, String)? in
            guard let id = s["id"]?.stringValue, let status = s["status"]?.stringValue else { return nil }
            return (id, status)
        }
        #expect(Set(daemon.map(\.0)) == [a.id, b.id])
        #expect(await liveUntil {
            let rows = feed.agents
            let now = (try? await store.client.call("terminal:listActive"))?.arrayValue ?? []
            return rows.map(\.id) == now.compactMap { $0["id"]?.stringValue }
                && rows.map(\.status.rawValue) == now.compactMap { $0["status"]?.stringValue }
        })
        #expect(feed.agents.allSatisfy { $0.agentType == .shell && !$0.label.isEmpty })
        #expect(WidgetCounts(feed.agents).total == 2)

        // The widget's setting lives in the config's defaults.
        #expect(store.defaults?.isWidgetEnabled == false)
        try await store.setDefault("widgetEnabled", true)
        #expect(await liveUntil { store.defaults?.isWidgetEnabled == true })

        // A request vornd broadcasts lands under its row until it is cancelled.
        feed.apply("widget:permission-request", ["requestId": "r1", "terminalId": .string(a.id), "toolName": "Bash",
                                                 "toolInput": ["command": "ls"]])
        #expect(feed.queue.forTerminal(a.id).map(\.requestId) == ["r1"])
        feed.apply("widget:permission-cancelled", "r1")
        #expect(feed.queue.requests.isEmpty)

        try await store.close(a.id)
        #expect(await liveUntil { feed.agents.map(\.id) == [b.id] })
    }
}
