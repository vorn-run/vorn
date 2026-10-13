import SwiftUI
import VornCore
import VornUI

/// NetworkSettings.tsx: which Vorn this window talks to, and who else may reach it.
struct NetworkSettings: View {
    @Bindable var model: SettingsModel
    @State private var net: RemoteAccessModel

    init(model: SettingsModel) {
        self.model = model
        let net = RemoteAccessModel(client: model.client, hostLink: model.remoteHost)
        if !model.renderHints.isEmpty { net.stage(hints: model.renderHints) }
        _net = State(initialValue: net)
    }

    private var enabled: Bool { net.stagedEnabled ?? model.bool("networkAccessEnabled", default: false) }

    private func setEnabled(_ value: Bool) {
        if net.staged { net.stagedEnabled = value; return }
        model.updateDefaults(["networkAccessEnabled": .bool(value)])
        // The server rebinds on the config change; re-ask once it has settled.
        Task {
            try? await Task.sleep(for: .milliseconds(500))
            await net.fetchStatus()
        }
    }

    var body: some View {
        if model.config != nil {
            VStack(alignment: .leading, spacing: 0) {
                SettingsPageHeader(title: "Remote Access", description: "Reach this Vorn from your phone or another computer")
                if net.loading && net.status == nil {
                    HStack(spacing: 12) {
                        CheckingSpinner()
                        Text("Checking network status...").font(.ui(14)).foregroundStyle(Theme.gray500).lineHeight(20, size: 14)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 32)
                }
                if net.connect != nil {
                    SegmentedControl(options: [
                        SegmentOption(value: ServerMode.local.rawValue, label: "This machine"),
                        SegmentOption(value: ServerMode.host.rawValue, label: "Another machine"),
                    ], value: net.viewing.rawValue) { net.viewing = ServerMode(rawValue: $0) ?? .local }
                    .padding(.bottom, 20)
                }
                if net.viewing == .host, let connect = net.connect {
                    UseAnotherMachine(net: net, mode: connect.mode, currentUrl: connect.url)
                } else {
                    ShareThisMachine(net: net, enabled: enabled, onChange: setEnabled)
                }
            }
            .task { await net.fetchStatus() }
            .task { await net.loadConnect() }
        }
    }
}

/// NetworkSettings.tsx's 20 px rotating arrow while the status loads.
private struct CheckingSpinner: View {
    @State private var turn = false
    var body: some View {
        LucideIcon(.rotateCw, size: 20, strokeWidth: 2)
            .foregroundStyle(Theme.gray500)
            .rotationEffect(.degrees(turn ? 360 : 0))
            .animation(.linear(duration: 1).repeatForever(autoreverses: false), value: turn)
            .onAppear { turn = true }
    }
}

/// A host link with fixed answers, for renders.
@MainActor
final class StagedHostLink: RemoteHostLink {
    let settings: HostConnectSettings
    init(_ settings: HostConnectSettings) { self.settings = settings }
    func connectSettings() async -> HostConnectSettings? { settings }
    func save(url: String, token: String) async -> String? { nil }
    func useLocal() async {}
}

extension RemoteAccessModel {
    /// Opens straight into the states renders name: confirm, sharing, lan, all, devices, revoke, code, expired,
    /// token, minted, asking, error, host, host-connected, host-error.
    func stage(hints: Set<String>) {
        let known: Set<String> = ["confirm", "sharing", "lan", "all", "devices", "revoke", "code", "expired", "token",
                                  "minted", "asking", "error", "host", "host-connected", "host-error"]
        guard !hints.isDisjoint(with: known) else { return }
        staged = true
        loading = false
        let lan = hints.contains("lan")
        status = lan ? TailscaleStatus(installed: false, running: false, backendState: "NoState")
            : TailscaleStatus(installed: true, running: true, backendState: "Running", selfIP: "100.101.7.42", selfDNSName: "studio.tail1234.ts.net")
        reachable = lan
            ? ReachableUrls(urls: ["http://192.168.1.24:61601/app/", "http://10.0.0.8:61601/app/"], port: 61601, remote: true)
            : ReachableUrls(urls: ["http://100.101.7.42:61601/app/", "http://192.168.1.24:61601/app/"], port: 61601, remote: true)
        tokens = []
        if hints.contains("confirm") {
            status = TailscaleStatus(installed: false, running: false, backendState: "NoState")
            confirming = true
            stagedEnabled = false
            return
        }
        stagedEnabled = true
        showAllAddresses = hints.contains("all")
        let devices = [
            DeviceToken(id: "t1", name: "iPhone 16", createdAt: "2026-10-01T09:12:00.000Z", lastSeenAt: "2026-10-12T21:04:31.000Z"),
            DeviceToken(id: "t2", name: "iPad", createdAt: "2026-10-10T18:00:00.000Z"),
            DeviceToken(id: "t3", name: "Old phone", createdAt: "2026-01-01T00:00:00.000Z", revokedAt: "2026-02-01T00:00:00.000Z"),
        ]
        if hints.contains("devices") || hints.contains("revoke") { tokens = devices }
        if hints.contains("revoke") { confirmRevokeId = "t1" }
        if hints.contains("code") || hints.contains("asking") || hints.contains("expired") {
            adding = .scan
            code = "K7Q-4MX"
            now = Date().timeIntervalSince1970 * 1000
            expiresAt = hints.contains("expired") ? now - 1000 : now + 272_000
        }
        if hints.contains("asking") {
            asking = PairingRequest(requestId: "r1", deviceName: "iPhone 16", address: "192.168.1.31", askedAt: now)
        }
        if hints.contains("token") { adding = .token; name = "" }
        if hints.contains("minted") {
            minted = ("iPhone 16", "vorn_2kQ9fXr7Lw3tNbVd8sYpHc1mZe6uGaJo4Ri0TlEy")
            tokens = Array(devices.prefix(2))
        }
        if hints.contains("error") { error = "Could not read the device list." }
        if hints.contains("host") || hints.contains("host-error") {
            let s = HostConnectSettings(mode: .local, url: "", hasToken: false)
            hostLink = StagedHostLink(s)
            applyConnect(s)
            viewing = .host
            if hints.contains("host-error") {
                hostUrl = "studio.local:61601"
                hostError = "Both an address and a token are needed."
            }
        }
        if hints.contains("host-connected") {
            let s = HostConnectSettings(mode: .host, url: "ws://100.101.7.42:61601/ws", hasToken: true)
            hostLink = StagedHostLink(s)
            applyConnect(s)
        }
    }
}
