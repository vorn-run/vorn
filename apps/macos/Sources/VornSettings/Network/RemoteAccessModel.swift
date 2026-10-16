import Foundation
import Observation
import VornCore

/// NetworkSettings.tsx and DeviceTokenList.tsx's state: the network status, the devices, and a pairing in progress.
@MainActor
@Observable
final class RemoteAccessModel {
    enum Adding { case scan, token }

    var status: TailscaleStatus?
    var reachable: ReachableUrls?
    var loading = true
    /// Where the app points; nil hides the This/Another machine switch, as the web build does.
    var connect: HostConnectSettings?
    var viewing: ServerMode = .local
    /// ShareThisMachine.tsx: set while the toggle is on its way up without a tailnet.
    var confirming = false
    var showAllAddresses = false

    var tokens: [DeviceToken]?
    var adding: Adding?
    var name = ""
    var code: String?
    var expiresAt: Double?
    var asking: PairingRequest?
    var chosenUrl: String?
    /// Shown once, then unrecoverable: only its hash is stored.
    var minted: (name: String, plaintext: String)?
    var confirmRevokeId: String?
    var error: String?
    /// Epoch ms, ticked once a second while a code is shown.
    var now = Date().timeIntervalSince1970 * 1000

    /// UseAnotherMachine.tsx's form.
    var hostUrl = ""
    var hostToken = ""
    var hostError: String?
    var hostEditing = true

    /// Set by renders: the page shows these fixtures and asks vornd for nothing.
    var staged = false
    var stagedEnabled: Bool?

    @ObservationIgnored let client: VornClient
    @ObservationIgnored var hostLink: RemoteHostLink?
    @ObservationIgnored private var ticker: Task<Void, Never>?

    init(client: VornClient, hostLink: RemoteHostLink?) {
        self.client = client
        self.hostLink = hostLink
    }

    // MARK: NetworkSettings.tsx

    func fetchStatus() async {
        guard !staged else { return }
        loading = true
        defer { loading = false }
        // Independent questions: a missing Tailscale must not stop the panel from showing an address.
        async let s = client.tailscaleStatus()
        async let r = client.reachableUrls()
        do {
            let (result, urls) = try await (s, r)
            status = result
            reachable = urls
        } catch {
            // The addresses go with the status: one it cannot confirm reads as "still reachable".
            reachable = nil
            status = .unreadable
        }
    }

    func loadConnect() async {
        guard !staged, let s = await hostLink?.connectSettings() else { return }
        applyConnect(s)
    }

    func applyConnect(_ s: HostConnectSettings) {
        connect = s
        viewing = s.mode
        hostUrl = s.url
        hostEditing = s.mode != .host
    }

    // MARK: DeviceTokenList.tsx

    var remaining: Double { expiresAt.map { max(0, $0 - now) } ?? 0 }
    /// Derived: a code past its countdown is one the server has already forgotten.
    var showingCode: Bool { code != nil && remaining > 0 }
    /// A phone cannot reach a server bound to loopback.
    var canScan: Bool { reachable?.remote == true && !(reachable?.urls.isEmpty ?? true) }
    var urls: [String] { reachable?.urls ?? [] }
    var fingerprint: String? { reachable?.fingerprint.flatMap { $0.isEmpty ? nil : $0 } }
    var pairUrl: String? {
        if let chosenUrl, urls.contains(chosenUrl) { return chosenUrl }
        return urls.first
    }
    var active: [DeviceToken] { RemoteAccess.active(tokens ?? []) }

    func load() async {
        guard !staged else { return }
        do {
            tokens = try await client.listDeviceTokens()
        } catch {
            self.error = "Could not read the device list."
        }
    }

    /// Pairing requests and collections, for as long as the page is open.
    func watch() async {
        guard !staged else { return }
        let requested = await client.notifications(RemoteAccess.pairingRequested)
        let collected = await client.notifications(RemoteAccess.pairingCollected)
        await withTaskGroup(of: Void.self) { group in
            group.addTask { @MainActor [weak self] in
                for await params in requested {
                    if let r = try? params.decode(PairingRequest.self) { self?.asking = r }
                }
            }
            // Approving does not create the token; the phone collecting it does, a moment later.
            group.addTask { @MainActor [weak self] in
                for await _ in collected { await self?.load() }
            }
        }
    }

    func addDevice() async {
        if canScan { await startScan() } else { adding = .token }
    }

    func create() async {
        error = nil
        let label = name.trimmingCharacters(in: .whitespacesAndNewlines)
        do {
            let result = try await client.createDeviceToken(name: label.isEmpty ? "Device" : label)
            minted = (result.token.name, result.plaintext)
            name = ""
            adding = nil
            await load()
        } catch {
            self.error = "Could not create a token."
        }
    }

    func startScan() async {
        error = nil
        do {
            let started = try await client.startPairing()
            code = started.code
            expiresAt = started.expiresAt
            adding = .scan
            startTicking()
        } catch {
            self.error = "Could not start pairing."
        }
    }

    private func startTicking() {
        now = Date().timeIntervalSince1970 * 1000
        ticker?.cancel()
        ticker = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(1))
                guard let self, self.expiresAt != nil else { return }
                self.now = Date().timeIntervalSince1970 * 1000
            }
        }
    }

    /// Retired, not merely hidden: a photographed code must stop working once put away.
    func closeAdding() {
        adding = nil
        code = nil
        expiresAt = nil
        asking = nil
        name = ""
        chosenUrl = nil
        ticker?.cancel()
        cancelPairing()
    }

    func pairManually() {
        cancelPairing()
        code = nil
        expiresAt = nil
        adding = .token
    }

    func cancelPairing() {
        guard !staged else { return }
        Task { [client] in try? await client.cancelPairing() }
    }

    func decide(_ approve: Bool) async {
        guard let request = asking else { return }
        asking = nil
        do {
            if approve {
                try await client.approvePairing(requestId: request.requestId)
            } else {
                try await client.denyPairing(requestId: request.requestId)
            }
        } catch {
            self.error = "Could not answer that request."
        }
        closeAdding()
        await load()
    }

    func revoke(_ id: String) async {
        confirmRevokeId = nil
        do {
            try await client.revokeDeviceToken(id: id)
            await load()
        } catch {
            self.error = "Could not revoke that token."
        }
    }

    // MARK: UseAnotherMachine.tsx

    func applyHost() async {
        hostError = nil
        switch RemoteAccess.prepareHost(url: hostUrl, token: hostToken) {
        case .failure(let e): hostError = e.message
        case .success(let v):
            guard let hostLink else { hostError = "Could not save those details."; return }
            if let message = await hostLink.save(url: v.url, token: v.token) { hostError = message }
        }
    }

    func useLocal() async { await hostLink?.useLocal() }
}
