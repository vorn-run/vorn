import Foundation
import Testing
import VornCore
@testable import VornSettings

/// The Plugins page against a throwaway vornd: the directory loads and a connection is added and removed.
@MainActor
struct PluginsLiveTests {
    @Test func directoryLoadsAndConnectionsComeAndGo() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let client = VornClient(dataDir: server.dataDir)
        #expect(await liveUntil { (try? await client.connect()) != nil })
        defer { Task { await client.close() } }

        let plugins = PluginsModel(client: client)
        await plugins.load()
        #expect(plugins.builtIns.contains { $0.id == "http" })
        #expect(plugins.connections.isEmpty)
        #expect(plugins.tab == .browse)
        let http = try #require(plugins.listings.first { $0.id == "http" })
        #expect(http.source == .builtin)
        #expect(plugins.canAdd(http))

        let api = ConnectorsAPI(client: client)
        let created = try #require(try await api.createConnection(
            connectorId: "http", name: "Status page",
            filters: ["profileName": .string("Status page"), "baseUrl": .string("https://status.example.com/api")],
            executionProject: nil))
        await plugins.reload()
        let groups = ConnectorBrowse.groups(plugins.connections, listings: plugins.listings)
        #expect(groups.map(\.connectorId) == ["http"])
        #expect(groups.first?.connections.map(\.name) == ["Status page"])

        await plugins.deleteConnection(created.id)
        #expect(plugins.connections.isEmpty)
        #expect(!plugins.isBusy(.delete, created.id))
    }
}
