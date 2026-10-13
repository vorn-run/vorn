import Foundation

/// vornd's Remote Access calls (`native/reach.rs`), as the renderer's `window.api` makes them.
extension VornClient {
    public func tailscaleStatus() async throws -> TailscaleStatus {
        try await call("tailscale:status", as: TailscaleStatus.self)
    }

    public func reachableUrls() async throws -> ReachableUrls {
        try await call("server:reachableUrls", as: ReachableUrls.self)
    }

    public func listDeviceTokens() async throws -> [DeviceToken] {
        try await call("token:list", as: [DeviceToken].self)
    }

    public func createDeviceToken(name: String) async throws -> MintedDeviceToken {
        try await call("token:create", .object(["name": .string(name)]), as: MintedDeviceToken.self)
    }

    /// Answers whether a live token was revoked; vornd closes the sockets holding it.
    @discardableResult
    public func revokeDeviceToken(id: String) async throws -> Bool {
        try await call("token:revoke", .string(id))["revoked"]?.boolValue == true
    }

    public func startPairing() async throws -> PairingCode {
        try await call("pairing:start", as: PairingCode.self)
    }

    public func pendingPairings() async throws -> [PairingRequest] {
        try await call("pairing:pending", as: [PairingRequest].self)
    }

    @discardableResult
    public func approvePairing(requestId: String) async throws -> Bool {
        try await call("pairing:approve", .object(["requestId": .string(requestId)]))["ok"]?.boolValue == true
    }

    @discardableResult
    public func denyPairing(requestId: String) async throws -> Bool {
        try await call("pairing:deny", .object(["requestId": .string(requestId)]))["ok"]?.boolValue == true
    }

    /// Retires the shown code, so a photo of it is worth nothing.
    public func cancelPairing() async throws {
        try await call("pairing:cancel")
    }
}

extension RemoteAccess {
    public static let pairingRequested = "pairing:requested"
    public static let pairingCollected = "pairing:collected"
}
