import Foundation
import Observation
import ServiceManagement
import VornCore
import VornUI

/// The settings page's categories, in the order the nav lists them.
public enum SettingsCategory: String, CaseIterable, Sendable {
    case appearance, general, updates, notifications, worktrees
    case agents, ssh, mcp, connectors, keys, network, about
}

public enum SettingsToastKind: Sendable { case success, error, info }

/// Where the app's own updates come from; the Mac build has none yet, so it reports `.unsupported`.
@MainActor
public protocol AppUpdater: AnyObject {
    var status: UpdateStatus { get }
    func check()
    func download()
    func install()
    func setChannel(_ channel: String)
    func setAutoDownload(_ on: Bool)
}

/// The settings page's state: the config as vornd stores it, and what each page reads beside it.
@MainActor
@Observable
public final class SettingsModel {
    public var category: SettingsCategory = .appearance
    public private(set) var config: [String: JSONValue]?
    public private(set) var installedAgents: [String: Bool]?
    public private(set) var detectingAgents = false
    public private(set) var shells: [InstalledShell]?
    public private(set) var sshKeys: [SSHKeyMeta] = []
    /// Whether vornd can keep secrets in the OS keychain (`credential:safeStorageAvailable`).
    public private(set) var safeStorage = true
    public private(set) var connectorKeys: [ConnectorKey] = []
    public private(set) var connectorKeysLoaded = false
    public private(set) var core: CoreStatus?
    public private(set) var holders: SessionHolders?
    public var inventory: WorktreeInventory?
    public var inventoryLoading = true
    /// The worktrees page's selection, kept here so the floating bar outside the scroll view can read it.
    public var wtSelected: Set<String> = []
    public var wtBranches: Set<String> = []
    public var wtConfirming: String?
    public var wtBusy = false
    /// Fixed for renders; otherwise read from the updater.
    public var updateStatus: UpdateStatus = .unsupported
    public var appVersion: String
    /// Sessions a restart would touch, and whether one of them is mid-turn (update-cost.ts).
    public var restartFacing: (count: Int, running: Bool) = (0, false)
    /// States an offscreen render opens straight into (`host:ID`, `paste`).
    public var renderHints: Set<String> = []
    @ObservationIgnored public weak var toasts: ToastCenter?
    public var onToast: (String, SettingsToastKind) -> Void = { _, _ in }
    /// Called after a write so the shell can pick up changes (font size, layout).
    public var onDefaultsChanged: ([String: JSONValue]) -> Void = { _ in }
    @ObservationIgnored public weak var updater: AppUpdater?

    public let client: VornClient

    public init(client: VornClient, appVersion: String = SettingsModel.bundleVersion) {
        self.client = client
        self.appVersion = appVersion
    }

    public nonisolated static var bundleVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.8.0-beta.7"
    }

    // MARK: Config

    public var defaults: [String: JSONValue] { config?["defaults"]?.objectValue ?? [:] }

    public func bool(_ key: String, default value: Bool) -> Bool { defaults[key]?.boolValue ?? value }

    public func load() async {
        if case .object(let c)? = try? await client.call("config:load") { config = c }
    }

    /// Fixture config for renders and tests that have no server to ask.
    public func stage(config: [String: JSONValue]? = nil, installedAgents: [String: Bool]? = nil,
                      shells: [InstalledShell]? = nil, sshKeys: [SSHKeyMeta]? = nil) {
        if let config { self.config = config }
        if let installedAgents { self.installedAgents = installedAgents }
        if let shells { self.shells = shells }
        if let sshKeys { self.sshKeys = sshKeys }
    }

    /// Merges `patch` into `defaults` and saves the whole config, as `updateDefaults` does.
    public func updateDefaults(_ patch: [String: JSONValue]) {
        guard var c = config else { return }
        var d = c["defaults"]?.objectValue ?? [:]
        for (k, v) in patch { d[k] = v }
        c["defaults"] = .object(d)
        save(c)
        onDefaultsChanged(patch)
    }

    /// Replaces top-level config keys and saves.
    public func updateConfig(_ patch: [String: JSONValue]) {
        guard var c = config else { return }
        for (k, v) in patch { c[k] = v }
        save(c)
    }

    @ObservationIgnored private var saving: Task<Void, Never>?

    private func save(_ c: [String: JSONValue]) {
        config = c
        let previous = saving
        saving = Task { [client, weak self] in
            await previous?.value
            do {
                try await client.call("config:save", .object(c))
            } catch {
                self?.onToast(error.localizedDescription, .error)
            }
        }
    }

    /// Waits for writes in flight, for tests.
    public func flush() async { await saving?.value }

    // MARK: Pages' data

    public func detectAgents() async {
        detectingAgents = true
        defer { detectingAgents = false }
        if let r = try? await client.call("agent:detectInstalled"), case .object(let o) = r {
            installedAgents = o.compactMapValues(\.boolValue)
        }
    }

    public func isInstalled(_ agent: AgentType) -> Bool { installedAgents?[agent.rawValue] ?? true }

    public func loadShells() async {
        shells = (try? await client.call("shell:listInstalled", as: [InstalledShell].self)) ?? []
    }

    public func loadSSHKeys() async {
        if let k = try? await client.call("credential:listKeys", as: [SSHKeyMeta].self) { sshKeys = k }
        if let ok = try? await client.call("credential:safeStorageAvailable").boolValue { safeStorage = ok }
    }

    public func storeSSHKey(label: String, privateKey: String, publicKey: String?, certificate: String?) async {
        var p: [String: JSONValue] = ["label": .string(label), "privateKey": .string(privateKey)]
        if let publicKey { p["publicKey"] = .string(publicKey) }
        if let certificate { p["certificate"] = .string(certificate) }
        do { try await client.call("credential:storeKey", .object(p)) } catch { onToast(error.localizedDescription, .error) }
        await loadSSHKeys()
    }

    public func importSSHKeyFile(_ path: String) async {
        do { try await client.call("credential:importKeyFile", .object(["filePath": .string(path)])) } catch {
            onToast(error.localizedDescription, .error)
        }
        await loadSSHKeys()
    }

    public func deleteSSHKey(_ id: String) async {
        try? await client.call("credential:deleteKey", .string(id))
        await loadSSHKeys()
    }

    public func testSSH(_ host: JSONValue) async -> (ok: Bool, message: String) {
        do {
            let r = try await client.call("ssh:testConnection", host)
            return (r["success"]?.boolValue == true, r["message"]?.stringValue ?? "")
        } catch {
            return (false, error.localizedDescription)
        }
    }

    public func loadConnectorKeys() async {
        connectorKeys = (try? await client.call("connection:listKeys", as: [ConnectorKey].self)) ?? []
        connectorKeysLoaded = true
    }

    public func preflight(_ connectionId: String) async -> (ok: Bool?, message: String?) {
        do {
            let r = try await client.call("connection:preflight", .string(connectionId))
            return (r["ok"]?.boolValue, r["message"]?.stringValue)
        } catch {
            return (false, error.localizedDescription)
        }
    }

    public func rotateSecret(connectionId: String, field: String, plaintext: String) async -> String? {
        do {
            let r = try await client.call("connection:rotateSecret", .object([
                "connectionId": .string(connectionId), "field": .string(field), "plaintext": .string(plaintext),
            ]))
            if r["ok"]?.boolValue == false { return r["error"]?.stringValue ?? "The key could not be replaced" }
            await loadConnectorKeys()
            return nil
        } catch {
            return error.localizedDescription
        }
    }

    public func loadCore() async {
        core = try? await client.call("core:status", as: CoreStatus.self)
        await loadHolders()
    }

    /// vornd's `/vornd/health`, which reports its session holders.
    public func loadHolders() async {
        guard let url = await healthURL(),
              let (data, _) = try? await URLSession.shared.data(from: url),
              let body = try? JSONDecoder().decode(JSONValue.self, from: data) else { return }
        holders = SessionHolders.parse(body["sessiond"])
    }

    private func healthURL() async -> URL? {
        guard let ep = await client.endpoint else { return nil }
        return URL(string: "http://127.0.0.1:\(ep.port)/vornd/health")
    }

    /// Ends an older holder, but only while its announcement still names the same process (session-holder.ts).
    public func endHolder(_ h: SessionHolder) async -> String? {
        guard holders?.older.contains(where: { $0.instance == h.instance }) == true else {
            return "vornd does not report that session holder as older"
        }
        guard h.instance.range(of: "^[0-9a-f]{1,32}$", options: .regularExpression) != nil else {
            return "that session holder is no longer running"
        }
        let file = client.dataDir.appendingPathComponent("run/sessiond-\(h.instance).info")
        guard let text = try? String(contentsOf: file, encoding: .utf8) else { return "that session holder is no longer running" }
        let fields = Dictionary(text.split(separator: "\n").compactMap { line -> (String, String)? in
            let kv = line.split(separator: "=", maxSplits: 1).map(String.init)
            return kv.count == 2 ? (kv[0], kv[1]) : nil
        }, uniquingKeysWith: { a, _ in a })
        guard let pid = fields["pid"].flatMap(Int32.init), pid > 0, pid == Int32(h.pid), fields["instance"] == h.instance else {
            return "that session holder is no longer running"
        }
        if kill(pid, SIGKILL) != 0 { return String(cString: strerror(errno)) }
        try? FileManager.default.removeItem(at: file)
        await loadHolders()
        return nil
    }

    // MARK: Worktrees

    public func loadInventory(refresh: Bool) async {
        do {
            inventory = try await client.call("worktree:inventory", .object(["refresh": .bool(refresh)]), as: WorktreeInventory.self)
        } catch {
            onToast(error.localizedDescription, .error)
        }
        inventoryLoading = false
    }

    public func reclaim(_ paths: [String]) async throws -> WorktreeActionResult {
        try await client.call("worktree:reclaimArtifacts", .object(["paths": .array(paths.map { .string($0) })]),
                              as: WorktreeActionResult.self)
    }

    public func removeWorktrees(_ entries: [WorktreeInventoryEntry]) async throws -> WorktreeActionResult {
        let items: [JSONValue] = entries.map { e in
            .object(["projectPath": .string(e.projectPath), "worktreePath": .string(e.path),
                     "force": .bool(e.isDirty), "deleteBranch": .bool(e.isMerged)])
        }
        return try await client.call("worktree:removeMany", .object(["items": .array(items)]), as: WorktreeActionResult.self)
    }

    public func pruneOrphans(_ paths: [String]) async throws -> WorktreeActionResult {
        try await client.call("worktree:pruneOrphans", .object(["paths": .array(paths.map { .string($0) })]),
                              as: WorktreeActionResult.self)
    }

    public func deleteBranches(projectPath: String, branches: [String]) async throws -> WorktreeActionResult {
        let r = try await client.call("git:deleteBranches", .object([
            "projectPath": .string(projectPath), "branches": .array(branches.map { .string($0) }), "force": .bool(false),
        ]))
        let deleted = (r["deleted"]?.arrayValue ?? []).compactMap(\.stringValue)
        let failed = (r["failed"]?.arrayValue ?? []).map {
            WorktreeActionResult.Failure(path: $0["branch"]?.stringValue ?? "", error: $0["error"]?.stringValue ?? "")
        }
        return WorktreeActionResult(succeeded: deleted, failed: failed, freedBytes: 0, deletedBranches: deleted)
    }

    // MARK: Login item

    /// Registers the app to open at sign-in; only a bundled app can (login-item.ts leaves dev builds alone).
    public static func applyLoginItem(_ on: Bool) {
        guard Bundle.main.bundleURL.pathExtension == "app" else { return }
        if on { try? SMAppService.mainApp.register() } else { try? SMAppService.mainApp.unregister() }
    }
}
