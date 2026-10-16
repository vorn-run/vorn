import Foundation

/// A device allowed to connect (`token:list`); only its hash is stored.
public struct DeviceToken: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var userId: String
    public var name: String
    public var createdAt: String
    public var lastSeenAt: String?
    public var revokedAt: String?
    /// Where it last connected from.
    public var lastAddress: String?

    public init(id: String, userId: String = "owner", name: String, createdAt: String,
                lastSeenAt: String? = nil, revokedAt: String? = nil, lastAddress: String? = nil) {
        self.id = id
        self.userId = userId
        self.name = name
        self.createdAt = createdAt
        self.lastSeenAt = lastSeenAt
        self.revokedAt = revokedAt
        self.lastAddress = lastAddress
    }

    /// The line under its name.
    public var seen: String {
        guard let at = lastSeenAt else { return "Never connected" }
        let when = "Last seen \(RemoteAccess.localeString(at))"
        guard let from = lastAddress, !from.isEmpty else { return when }
        return "\(when) from \(from)"
    }
}

/// `token:create`'s answer: the row, and the plaintext shown this once.
public struct MintedDeviceToken: Codable, Hashable, Sendable {
    public var token: DeviceToken
    public var plaintext: String
}

/// A code a phone can redeem (`pairing:start`), with its deadline in epoch ms.
public struct PairingCode: Codable, Hashable, Sendable {
    public var code: String
    public var expiresAt: Double

    public init(code: String, expiresAt: Double) {
        self.code = code
        self.expiresAt = expiresAt
    }
}

/// A phone that offered a valid code and waits to be approved (`pairing:requested`).
public struct PairingRequest: Codable, Hashable, Sendable, Identifiable {
    public var requestId: String
    public var deviceName: String
    public var address: String
    public var askedAt: Double
    public var status: String
    public var id: String { requestId }

    public init(requestId: String, deviceName: String, address: String, askedAt: Double, status: String = "pending") {
        self.requestId = requestId
        self.deviceName = deviceName
        self.address = address
        self.askedAt = askedAt
        self.status = status
    }
}

/// Where clients reach this vornd (`server:reachableUrls`), tailnet first.
public struct ReachableUrls: Codable, Hashable, Sendable {
    public var urls: [String]
    public var port: Int
    public var remote: Bool
    /// SHA-256 of the certificate the urls are served with, which a phone pins.
    public var fingerprint: String?

    public init(urls: [String], port: Int, remote: Bool, fingerprint: String? = nil) {
        self.urls = urls
        self.port = port
        self.remote = remote
        self.fingerprint = fingerprint
    }
}

/// What `tailscale:status` reports, without the peers this page never shows.
public struct TailscaleStatus: Codable, Hashable, Sendable {
    public var installed: Bool
    public var running: Bool
    public var backendState: String
    public var selfIP: String
    public var selfDNSName: String

    public init(installed: Bool, running: Bool, backendState: String, selfIP: String = "", selfDNSName: String = "") {
        self.installed = installed
        self.running = running
        self.backendState = backendState
        self.selfIP = selfIP
        self.selfDNSName = selfDNSName
    }

    /// What NetworkSettings.tsx assumes when the status cannot be read.
    public static let unreadable = TailscaleStatus(installed: false, running: false, backendState: "Error")
}

/// Whether this app runs its own server or talks to one elsewhere (host-store.ts).
public enum ServerMode: String, Codable, Sendable { case local, host }

/// `connect:get`: where the app points, without the stored token.
public struct HostConnectSettings: Hashable, Sendable {
    public var mode: ServerMode
    public var url: String
    public var hasToken: Bool

    public init(mode: ServerMode, url: String, hasToken: Bool) {
        self.mode = mode
        self.url = url
        self.hasToken = hasToken
    }
}

/// The shell's half of "Another machine": reads and stores the host, and restarts onto it.
@MainActor
public protocol RemoteHostLink: AnyObject {
    func connectSettings() async -> HostConnectSettings?
    /// Stores `url` (already normalised) and `token`, then restarts; an error message when it cannot.
    func save(url: String, token: String) async -> String?
    func useLocal() async
}

/// address-kind.ts: which network an address reaches this machine over.
public enum AddressKind: String, Sendable { case tailnet, lan }

public enum RemoteAccess {
    /// address-kind.ts `addressKind`: 100.64.0.0/10 is Tailscale's, and nothing else uses it.
    public static func addressKind(_ url: String) -> AddressKind {
        guard let host = hostname(url) else { return .lan }
        let parts = host.split(separator: ".", omittingEmptySubsequences: false).map { Int($0) }
        guard parts.count >= 2, let a = parts[0], let b = parts[1] else { return .lan }
        return a == 100 && b >= 64 && b <= 127 ? .tailnet : .lan
    }

    /// address-kind.ts `addressHost`: the host on its own, or the input when it does not parse.
    public static func addressHost(_ url: String) -> String { hostname(url) ?? url }

    /// `new URL(url).hostname`, or nil where the URL constructor throws.
    static func hostname(_ url: String) -> String? {
        guard let c = parsed(url) else { return nil }
        let host = (c.host ?? "").lowercased()
        return host.contains(":") && !host.hasPrefix("[") ? "[\(host)]" : host
    }

    private static func parsed(_ url: String) -> URLComponents? {
        guard let c = URLComponents(string: url), let scheme = c.scheme?.lowercased(),
              scheme.first?.isLetter == true else { return nil }
        if ["http", "https", "ws", "wss", "ftp"].contains(scheme), (c.host ?? "").isEmpty { return nil }
        return c
    }

    /// DeviceTokenList.tsx `formatRemaining`: `m:ss`, rounding up.
    public static func formatRemaining(_ ms: Double) -> String {
        let total = Int((ms / 1000).rounded(.up))
        return "\(total / 60):\(String(format: "%02d", total % 60))"
    }

    /// The QR's payload: `vorn://pair?url=…&code=…`, the URL as `encodeURIComponent` writes it.
    public static func pairingLink(url: String, code: String, fingerprint: String? = nil) -> String {
        let link = "vorn://pair?url=\(encodeURIComponent(url))&code=\(code)"
        guard let fingerprint, !fingerprint.isEmpty else { return link }
        return "\(link)&fp=\(fingerprint)"
    }

    /// A fingerprint in groups of four, to compare by eye or type.
    public static func grouped(_ fingerprint: String) -> String {
        stride(from: 0, to: fingerprint.count, by: 4).map { i -> String in
            let start = fingerprint.index(fingerprint.startIndex, offsetBy: i)
            let end = fingerprint.index(start, offsetBy: 4, limitedBy: fingerprint.endIndex) ?? fingerprint.endIndex
            return String(fingerprint[start..<end])
        }.joined(separator: " ")
    }

    private static let uriUnreserved = CharacterSet(charactersIn:
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.!~*'()")

    static func encodeURIComponent(_ s: String) -> String {
        s.addingPercentEncoding(withAllowedCharacters: uriUnreserved) ?? s
    }

    /// connect-window.ts `normaliseHostUrl`: what a person pastes, as the `ws(s)://host:port/ws` the bridge needs.
    public static func normaliseHostUrl(_ input: String) -> String {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        var value = trimmed
        if value.range(of: "^[a-z]+://", options: [.regularExpression, .caseInsensitive]) == nil { value = "http://\(value)" }
        guard let c = parsed(value), let scheme = c.scheme?.lowercased(), let host = hostname(value), !host.isEmpty else {
            return trimmed
        }
        let secure = scheme == "https" || scheme == "wss"
        let defaultPort = ["http": 80, "ws": 80, "https": 443, "wss": 443][scheme]
        let port = c.port.flatMap { $0 == defaultPort ? nil : ":\($0)" } ?? ""
        return "\(secure ? "wss" : "ws")://\(host)\(port)/ws"
    }

    /// connect-window.ts `connect:save`: both fields are needed; answers the normalised address and token.
    public static func prepareHost(url: String, token: String) -> Result<(url: String, token: String), HostInputError> {
        let u = url.trimmingCharacters(in: .whitespacesAndNewlines)
        let t = token.trimmingCharacters(in: .whitespacesAndNewlines)
        if u.isEmpty || t.isEmpty { return .failure(HostInputError()) }
        return .success((normaliseHostUrl(u), t))
    }

    public struct HostInputError: Error, Equatable {
        public var message: String { "Both an address and a token are needed." }
    }

    private static let isoFractional: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()

    private static let isoPlain = ISO8601DateFormatter()

    /// `new Date(iso).toLocaleString()` as en-US writes it: `10/12/2026, 10:40:53 PM`.
    public static func localeString(_ iso: String, timeZone: TimeZone = .current) -> String {
        guard let date = isoFractional.date(from: iso) ?? isoPlain.date(from: iso) else { return "Invalid Date" }
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = timeZone
        f.dateFormat = "M/d/yyyy, h:mm:ss a"
        return f.string(from: date)
    }

    /// The tokens still allowed in.
    public static func active(_ tokens: [DeviceToken]) -> [DeviceToken] { tokens.filter { $0.revokedAt == nil } }
}
