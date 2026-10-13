import Foundation
import Testing
@testable import VornCore

/// Remote Access's pure rules: address-kind.ts, DeviceTokenList.tsx and connect-window.ts.
struct RemoteAccessTests {
    @Test func tailnetIsExactlyTheCGNATRange() {
        #expect(RemoteAccess.addressKind("http://100.64.0.1:61601/app/") == .tailnet)
        #expect(RemoteAccess.addressKind("http://100.101.7.42:61601/app/") == .tailnet)
        #expect(RemoteAccess.addressKind("http://100.127.255.254/") == .tailnet)
        #expect(RemoteAccess.addressKind("http://100.63.0.1/") == .lan)
        #expect(RemoteAccess.addressKind("http://100.128.0.1/") == .lan)
        #expect(RemoteAccess.addressKind("http://192.168.1.24:61601/app/") == .lan)
        #expect(RemoteAccess.addressKind("http://studio.tail1234.ts.net/") == .lan)
        #expect(RemoteAccess.addressKind("not a url") == .lan)
        #expect(RemoteAccess.addressKind("") == .lan)
    }

    @Test func addressHostIsTheHostnameOrTheInput() {
        #expect(RemoteAccess.addressHost("http://192.168.1.24:61601/app/") == "192.168.1.24")
        #expect(RemoteAccess.addressHost("http://Studio.Local:61601/") == "studio.local")
        #expect(RemoteAccess.addressHost("http://[fe80::1]:61601/") == "[fe80::1]")
        #expect(RemoteAccess.addressHost("192.168.1.24:61601") == "192.168.1.24:61601")
        #expect(RemoteAccess.addressHost("nope") == "nope")
    }

    @Test func remainingReadsMinutesAndPaddedSeconds() {
        #expect(RemoteAccess.formatRemaining(300_000) == "5:00")
        #expect(RemoteAccess.formatRemaining(272_000) == "4:32")
        #expect(RemoteAccess.formatRemaining(61_001) == "1:02")
        #expect(RemoteAccess.formatRemaining(9_000) == "0:09")
        #expect(RemoteAccess.formatRemaining(1) == "0:01")
        #expect(RemoteAccess.formatRemaining(0) == "0:00")
    }

    @Test func pairingLinkEncodesTheUrlAsEncodeURIComponent() {
        #expect(RemoteAccess.pairingLink(url: "http://100.101.7.42:61601/app/", code: "K7Q-4MX")
            == "vorn://pair?url=http%3A%2F%2F100.101.7.42%3A61601%2Fapp%2F&code=K7Q-4MX")
        #expect(RemoteAccess.encodeURIComponent("a b&c=d/é!~*'()") == "a%20b%26c%3Dd%2F%C3%A9!~*'()")
    }

    @Test func hostUrlsAreNormalisedToTheBridgeSocket() {
        #expect(RemoteAccess.normaliseHostUrl("192.168.0.4:61601") == "ws://192.168.0.4:61601/ws")
        #expect(RemoteAccess.normaliseHostUrl("  http://192.168.0.4:61601/app/  ") == "ws://192.168.0.4:61601/ws")
        #expect(RemoteAccess.normaliseHostUrl("https://vorn.example.com/app/") == "wss://vorn.example.com/ws")
        #expect(RemoteAccess.normaliseHostUrl("wss://vorn.example.com:8443/ws") == "wss://vorn.example.com:8443/ws")
        #expect(RemoteAccess.normaliseHostUrl("http://studio.local:80") == "ws://studio.local/ws")
        #expect(RemoteAccess.normaliseHostUrl("HTTP://Studio.Local:61601") == "ws://studio.local:61601/ws")
        #expect(RemoteAccess.normaliseHostUrl("[::1]:61601") == "ws://[::1]:61601/ws")
        #expect(RemoteAccess.normaliseHostUrl(" not a host ") == "not a host")
    }

    @Test func connectingNeedsBothFields() {
        #expect(throws: RemoteAccess.HostInputError.self) { try RemoteAccess.prepareHost(url: " ", token: "vorn_x").get() }
        #expect(throws: RemoteAccess.HostInputError.self) { try RemoteAccess.prepareHost(url: "box:1", token: "").get() }
        let ok = try? RemoteAccess.prepareHost(url: " box:61601 ", token: " vorn_abc ").get()
        #expect(ok?.url == "ws://box:61601/ws")
        #expect(ok?.token == "vorn_abc")
        #expect(RemoteAccess.HostInputError().message == "Both an address and a token are needed.")
    }

    @Test func lastSeenReadsAsEnUSLocaleString() {
        let utc = TimeZone(identifier: "UTC")!
        #expect(RemoteAccess.localeString("2026-10-12T21:04:31.000Z", timeZone: utc) == "10/12/2026, 9:04:31 PM")
        #expect(RemoteAccess.localeString("2026-01-02T03:04:05Z", timeZone: utc) == "1/2/2026, 3:04:05 AM")
        #expect(RemoteAccess.localeString("yesterday", timeZone: utc) == "Invalid Date")
    }

    @Test func revokedTokensAreNotListed() {
        let tokens = [
            DeviceToken(id: "a", name: "A", createdAt: "2026-01-01T00:00:00Z"),
            DeviceToken(id: "b", name: "B", createdAt: "2026-01-01T00:00:00Z", revokedAt: "2026-02-01T00:00:00Z"),
        ]
        #expect(RemoteAccess.active(tokens).map(\.id) == ["a"])
    }

    @Test func modelsDecodeWhatVorndSends() throws {
        let tokens = try JSONValue.array([[
            "id": "t1", "userId": "u", "name": "Phone", "createdAt": "2026-10-01T00:00:00.000Z",
            "lastSeenAt": .null, "revokedAt": .null,
        ]]).decode([DeviceToken].self)
        #expect(tokens.first?.lastSeenAt == nil)
        let request = try JSONValue.object([
            "requestId": "r", "deviceName": "iPhone", "address": "192.168.1.31", "askedAt": 1_760_000_000_000, "status": "pending",
        ]).decode(PairingRequest.self)
        #expect(request.deviceName == "iPhone")
        let status = try JSONValue.object([
            "installed": true, "running": false, "backendState": "Stopped", "selfIP": "", "selfDNSName": "", "peers": [],
        ]).decode(TailscaleStatus.self)
        #expect(status.installed && !status.running)
    }

    @Test func qrMatrixHasItsFinderPatternsTheRightWayUp() throws {
        let m = try #require(QRMatrix.make("vorn://pair?url=http%3A%2F%2F192.168.1.24%3A61601%2Fapp%2F&code=K7Q-4MX"))
        #expect(m.size >= 21 && (m.size - 17) % 4 == 0)
        func finder(_ x0: Int, _ y0: Int) -> Bool {
            (0..<7).allSatisfy { i in m[x0 + i, y0] && m[x0 + i, y0 + 6] && m[x0, y0 + i] && m[x0 + 6, y0 + i] }
                && !m[x0 + 1, y0 + 1] && m[x0 + 3, y0 + 3]
        }
        #expect(finder(0, 0))
        #expect(finder(m.size - 7, 0))
        #expect(finder(0, m.size - 7))
        #expect(!finder(m.size - 7, m.size - 7))
    }
}
