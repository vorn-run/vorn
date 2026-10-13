import Foundation
import Testing
import VornCore
@testable import VornSettings

/// Remote Access against a throwaway vornd on loopback: device tokens, and a phone pairing by code.
@MainActor
struct RemoteAccessLiveTests {
    private func connect(_ server: TestVornd) async throws -> VornClient {
        let client = VornClient(dataDir: server.dataDir)
        #expect(await liveUntil { (try? await client.connect()) != nil })
        return client
    }

    /// What a phone sends: `POST /api/pair/…` with a JSON body, over loopback.
    private func phone(_ client: VornClient, _ path: String, _ body: [String: String]) async throws -> (Int, JSONValue) {
        let port = try #require(await client.endpoint?.port)
        var request = URLRequest(url: URL(string: "http://127.0.0.1:\(port)\(path)")!)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONEncoder().encode(body)
        let (data, response) = try await URLSession.shared.data(for: request)
        return ((response as? HTTPURLResponse)?.statusCode ?? 0, try JSONDecoder().decode(JSONValue.self, from: data))
    }

    @Test func tokensAreCreatedListedAndRevoked() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let client = try await connect(server)
        defer { Task { await client.close() } }

        let net = RemoteAccessModel(client: client, hostLink: nil)
        await net.load()
        #expect(net.tokens == [])
        #expect(net.error == nil)

        // Loopback cannot be scanned from a phone, so Add device goes straight to the form.
        await net.addDevice()
        #expect(net.adding == .token)
        net.name = "  Laptop  "
        await net.create()
        let minted = try #require(net.minted)
        #expect(minted.name == "Laptop")
        #expect(minted.plaintext.hasPrefix("vorn_"))
        #expect(net.adding == nil)
        #expect(net.active.map(\.name) == ["Laptop"])
        #expect(net.active.first?.lastSeenAt == nil)

        // An empty name is "Device", as the renderer sends it.
        net.adding = .token
        await net.create()
        #expect(Set(net.active.map(\.name)) == ["Laptop", "Device"])

        let laptop = try #require(net.active.first { $0.name == "Laptop" })
        net.confirmRevokeId = laptop.id
        await net.revoke(laptop.id)
        #expect(net.confirmRevokeId == nil)
        #expect(net.active.map(\.name) == ["Device"])
        let stored = try await client.listDeviceTokens()
        #expect(stored.first { $0.id == laptop.id }?.revokedAt != nil)
        #expect(try await client.revokeDeviceToken(id: laptop.id) == false)

        // The status is asked for and settles; a loopback vornd reaches nobody else.
        await net.fetchStatus()
        #expect(net.status != nil)
        #expect(!net.loading)
        #expect(net.reachable?.remote != true)
        #expect(!net.canScan)
    }

    @Test func aPhonePairsByCodeOnceApproved() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let client = try await connect(server)
        defer { Task { await client.close() } }

        let net = RemoteAccessModel(client: client, hostLink: nil)
        await net.load()
        let watching = Task { await net.watch() }
        defer { watching.cancel() }

        await net.startScan()
        let code = try #require(net.code)
        #expect(net.adding == .scan)
        #expect(net.showingCode)
        #expect(net.remaining > 60_000 && net.remaining <= 300_000)

        // The phone offers the code; the page asks by name.
        let (status, redeemed) = try await phone(client, "/api/pair/redeem", ["code": code, "deviceName": "Test phone"])
        #expect(status == 200)
        let requestId = try #require(redeemed["requestId"]?.stringValue)
        #expect(await liveUntil(timeout: 10) { net.asking?.requestId == requestId })
        #expect(net.asking?.deviceName == "Test phone")
        #expect(net.asking?.address == "127.0.0.1")
        #expect(try await client.pendingPairings().map(\.requestId).contains(requestId))

        await net.decide(true)
        #expect(net.asking == nil)
        #expect(net.adding == nil)
        #expect(net.code == nil)

        // Collecting mints the token, and the list picks it up without reopening.
        let (_, polled) = try await phone(client, "/api/pair/poll", ["requestId": requestId])
        #expect(polled["status"]?.stringValue == "approved")
        #expect(polled["token"]?.stringValue?.hasPrefix("vorn_") == true)
        #expect(await liveUntil(timeout: 10) { net.active.contains { $0.name == "Test phone" } })

        // Denied requests never become devices.
        await net.startScan()
        let third = try #require(net.code)
        let (_, asked) = try await phone(client, "/api/pair/redeem", ["code": third, "deviceName": "Stranger"])
        let deniedId = try #require(asked["requestId"]?.stringValue)
        #expect(await liveUntil(timeout: 10) { net.asking?.requestId == deniedId })
        await net.decide(false)
        let (_, denied) = try await phone(client, "/api/pair/poll", ["requestId": deniedId])
        #expect(denied["status"]?.stringValue == "denied")
        #expect(!net.active.contains { $0.name == "Stranger" })
        // A code put away is retired: the phone that photographed it gets nothing.
        await net.startScan()
        let second = try #require(net.code)
        net.closeAdding()
        #expect(net.code == nil)
        #expect(await liveUntil(timeout: 5) {
            let r = try? await phone(client, "/api/pair/redeem", ["code": second, "deviceName": "Late phone"])
            return r?.0 == 400
        })

    }

    @Test func anotherMachineValidatesThenHandsTheHostToTheShell() async throws {
        final class Link: RemoteHostLink {
            var saved: (String, String)?
            var local = false
            func connectSettings() async -> HostConnectSettings? { HostConnectSettings(mode: .host, url: "ws://10.0.0.2:61601/ws", hasToken: true) }
            func save(url: String, token: String) async -> String? { saved = (url, token); return nil }
            func useLocal() async { local = true }
        }
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let client = try await connect(server)
        defer { Task { await client.close() } }

        let link = Link()
        let net = RemoteAccessModel(client: client, hostLink: link)
        await net.loadConnect()
        #expect(net.connect?.mode == .host)
        #expect(net.viewing == .host)
        #expect(!net.hostEditing)

        net.hostEditing = true
        net.hostUrl = "192.168.0.4:61601"
        net.hostToken = ""
        await net.applyHost()
        #expect(net.hostError == "Both an address and a token are needed.")
        #expect(link.saved == nil)

        net.hostToken = " vorn_abc "
        await net.applyHost()
        #expect(net.hostError == nil)
        #expect(link.saved?.0 == "ws://192.168.0.4:61601/ws")
        #expect(link.saved?.1 == "vorn_abc")

        await net.useLocal()
        #expect(link.local)
    }
}
