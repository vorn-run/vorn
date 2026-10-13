import Foundation
import Observation
import VornCore

/// ConnectorSettings.tsx's state with use-connections, use-connector-catalog, use-extensions and use-pack-install.
@MainActor
@Observable
public final class PluginsModel {
    public enum Tab: String, CaseIterable, Sendable { case installed, connections, browse }

    /// A pack staged for install and waiting on the person (use-pack-install.ts Pending).
    public struct Pending: Hashable, Sendable {
        public var preview: PackPreview
        public var rowId: String
        public var rowKey: String?
    }

    public struct Project: Hashable, Sendable {
        public var name: String
        public var path: String
    }

    public var tab: Tab = .browse
    public var selectedKey: String?
    public var adding: ConnectorListing?

    public private(set) var loaded = false
    public private(set) var builtIns: [BuiltInConnector] = []
    public private(set) var connections: [SourceConnection] = []
    public private(set) var statuses: [String: ConnectorAuthStatus] = [:]
    public private(set) var packs: [InstalledPack] = []
    public private(set) var catalog: CatalogSnapshot?
    public private(set) var catalogLoading = true
    public private(set) var catalogRefreshing = false
    public private(set) var workflows: [SeededWorkflow] = []
    public private(set) var projects: [Project] = []

    public var progress: [String: InstallProgress] = [:]
    public var pending: Pending?
    public var installError: String?
    public private(set) var installing = false

    /// use-row-action.ts: what each row is doing, and why the last attempt failed.
    public private(set) var busy: [String: String] = [:]
    public private(set) var failed: [String: String] = [:]
    public private(set) var backfills: [String: ConnectorsAPI.BackfillResult] = [:]
    public private(set) var tests: [String: ConnectorsAPI.PreflightResult] = [:]
    public private(set) var testing: Set<String> = []

    /// use-extensions.ts: open cards an extension draws on, by id; nil until counted.
    public private(set) var activeCards: [String: Int]?
    public private(set) var openCards = 0
    /// The open sessions' ids, which the shell provides.
    @ObservationIgnored public var sessionIds: @MainActor () -> [String] = { [] }
    /// Opens the workflow editor for a connector that needs no connection; the shell provides it.
    @ObservationIgnored public var openWorkflowEditor: (@MainActor (String?) -> Void)?

    @ObservationIgnored let api: ConnectorsAPI
    /// What `probeSdk` answers in renders and tests that cannot start a package.
    @ObservationIgnored var probeFixture: SdkManifest?
    @ObservationIgnored private var watching: [Task<Void, Never>] = []
    @ObservationIgnored private var tabChosen = false

    public init(client: VornClient) {
        api = ConnectorsAPI(client: client)
    }

    // MARK: Derived

    public var listings: [ConnectorListing] {
        ConnectorBrowse.listings(builtIns: builtIns, catalog: catalog?.items ?? [], connections: connections, packs: packs,
                                 mcpServers: catalog?.mcpServers ?? [])
    }

    public func state(_ listing: ConnectorListing) -> PackState {
        PackState(installed: listing.pack, catalogItem: listing.catalogItem, progress: progress[listing.id])
    }

    public func canAdd(_ listing: ConnectorListing) -> Bool {
        PackStatusView.canAddConnection(state(listing), source: listing.source, hasLegacyLaunch: listing.catalogItem?.packageName.isEmpty == false,
                                        kind: listing.kind)
    }

    public func rowState(_ id: String, _ actions: [RowAction]) -> RowState {
        RowState(busy: busy, failed: failed, id: id, actions: actions)
    }

    public func isBusy(_ action: RowAction, _ id: String) -> Bool { busy[action.key(id)] != nil }

    public var mcpConnector: BuiltInConnector? { builtIns.first { $0.id == "mcp" } }

    // MARK: Loading

    /// Loads what every tab reads, then the slower sign-in checks and card counts behind it.
    public func load() async {
        await reload()
        await loadCatalog()
        if !tabChosen {
            tab = packs.isEmpty ? .browse : .installed
            tabChosen = true
        }
        loaded = true
        Task { await loadStatuses() }
        await loadActivity()
    }

    /// Re-reads connections, packs, built-ins and the config's workflows and projects.
    public func reload() async {
        async let b = try? api.builtIns()
        async let c = try? api.connections()
        async let p = try? api.packs()
        async let config = try? api.client.call("config:load")
        if let b = await b { builtIns = b }
        if let c = await c { connections = c }
        if let p = await p { packs = p }
        if let config = await config {
            workflows = (config["workflows"]?.arrayValue ?? []).compactMap(SeededWorkflow.init(json:))
            projects = (config["projects"]?.arrayValue ?? []).compactMap { p in
                p["name"]?.stringValue.map { Project(name: $0, path: p["path"]?.stringValue ?? "") }
            }
        }
    }

    public func loadCatalog() async {
        catalogLoading = true
        if let snapshot = try? await api.catalog() { catalog = snapshot }
        catalogLoading = false
    }

    /// `connector:status` asks every tool whether it is signed in, so it is read after the page draws.
    public func loadStatuses() async {
        guard let list = try? await api.statuses() else { return }
        statuses = Dictionary(list.map { ($0.connectorId, $0) }, uniquingKeysWith: { a, _ in a })
    }

    public func loadActivity() async {
        let ids = sessionIds()
        var counts: [String: Int] = [:]
        for id in ids {
            for state in (try? await api.activation(sessionId: id)) ?? [] where state.active && state.draws {
                counts[state.extensionId, default: 0] += 1
            }
        }
        openCards = ids.count
        activeCards = counts
    }

    public func refreshCatalog() async {
        catalogRefreshing = true
        if let snapshot = try? await api.refreshCatalog() { catalog = snapshot }
        catalogRefreshing = false
    }

    /// Follows install progress and catalog changes vornd pushes, until `stop`.
    public func start() {
        guard watching.isEmpty else { return }
        let api = api
        watching.append(Task { [weak self] in
            for await p in await api.installProgress() { self?.progress[p.id] = p }
        })
        watching.append(Task { [weak self] in
            for await snapshot in await api.catalogChanges() { self?.catalog = snapshot }
        })
    }

    public func stop() {
        watching.forEach { $0.cancel() }
        watching = []
    }

    /// Fixture data for renders and tests that have no server to ask.
    public func stage(builtIns: [BuiltInConnector]? = nil, connections: [SourceConnection]? = nil, packs: [InstalledPack]? = nil,
                      catalog: CatalogSnapshot? = nil, statuses: [ConnectorAuthStatus]? = nil, workflows: [SeededWorkflow]? = nil,
                      projects: [Project]? = nil, activeCards: [String: Int]? = nil, openCards: Int? = nil) {
        if let builtIns { self.builtIns = builtIns }
        if let connections { self.connections = connections }
        if let packs { self.packs = packs }
        if let catalog { self.catalog = catalog; catalogLoading = false }
        if let statuses { self.statuses = Dictionary(statuses.map { ($0.connectorId, $0) }, uniquingKeysWith: { a, _ in a }) }
        if let workflows { self.workflows = workflows }
        if let projects { self.projects = projects }
        if let activeCards { self.activeCards = activeCards }
        if let openCards { self.openCards = openCards }
        loaded = true
    }

    func probe(_ launch: LaunchSpec) async -> ConnectorsAPI.Outcome<SdkManifest> {
        if let probeFixture { return .ok(probeFixture) }
        return await api.probeSdk(launch)
    }

    // MARK: Navigation

    /// Loads, then opens the state named by `--settings-hints`: `plugins-installed|connections|browse`,
    /// `plugins-detail:<id>`, `plugins-add:<id>` and `plugins-confirm:<id>`.
    public func render(hints: Set<String>) async {
        await load()
        for tab in Tab.allCases where hints.contains("plugins-\(tab.rawValue)") { show(tab) }
        func listing(_ prefix: String) -> ConnectorListing? {
            hints.first { $0.hasPrefix(prefix) }.flatMap { hint in
                let id = String(hint.dropFirst(prefix.count))
                return listings.first { $0.id == id }
            }
        }
        if let found = listing("plugins-detail:") { selectedKey = found.key }
        if let found = listing("plugins-add:") {
            adding = found
            // A render client is read-only, so the add form reads the installed pack's manifest instead of starting it.
            if let pack = found.pack {
                probeFixture = SdkManifest(id: pack.id, name: pack.name, version: pack.version, description: pack.description, icon: pack.icon,
                                           auth: pack.auth, triggers: pack.triggers, actions: pack.actions, env: pack.env)
            }
        }
        if let found = listing("plugins-confirm:") {
            pending = Pending(preview: Self.preview(of: found), rowId: found.id, rowKey: found.key)
        }
    }

    /// What staging this listing's pack would describe, without fetching it.
    static func preview(of listing: ConnectorListing) -> PackPreview {
        if let pack = listing.pack {
            return PackPreview(id: pack.id, name: pack.name, version: pack.version, kind: pack.kind, description: pack.description,
                               icon: pack.icon, auth: pack.auth, triggers: pack.triggers, actions: pack.actions, env: pack.env,
                               contributes: pack.contributes, permissions: pack.permissions, activates: pack.activates)
        }
        let item = listing.catalogItem
        return PackPreview(id: listing.id, name: listing.name, version: item?.verified?.version ?? item?.version ?? "1.0.0",
                           kind: listing.kind, description: listing.description, icon: listing.icon,
                           actions: item?.actions ?? [], env: item?.env ?? [], contributes: listing.contributes,
                           permissions: listing.permissions, activates: listing.activates)
    }

    public func show(_ next: Tab) {
        tab = next
        tabChosen = true
        selectedKey = nil
        adding = nil
    }

    public func finishAdding() async {
        adding = nil
        tab = .connections
        await reload()
    }

    // MARK: Packs (use-pack-install.ts)

    /// Stages a listing's published pack; installs straight away when it is exactly what the directory described.
    public func inspect(_ listing: ConnectorListing, direct: Bool) async {
        guard let url = listing.catalogItem?.packUrl, !url.isEmpty else {
            progress[listing.id] = nil
            return
        }
        installError = nil
        progress[listing.id] = InstallProgress(id: listing.id, phase: .checking)
        switch await api.inspectPack(.url(url, sha256: listing.catalogItem?.sha256)) {
        case .failed(let error):
            progress[listing.id] = InstallProgress(id: listing.id, phase: .failed, error: error)
        case .ok(let preview):
            let pending = Pending(preview: preview, rowId: listing.id, rowKey: listing.key)
            if direct && ConnectorBrowse.matches(preview, listing) {
                await keep(pending)
            } else {
                progress[listing.id] = nil
                self.pending = pending
            }
        }
    }

    /// Stages a pack file someone picked.
    public func inspectFile(_ path: String) async {
        installError = nil
        switch await api.inspectPack(.file(path: path)) {
        case .failed(let error): installError = error
        case .ok(let preview): pending = Pending(preview: preview, rowId: preview.id, rowKey: nil)
        }
    }

    public func confirm() async {
        guard let pending else { return }
        await keep(pending)
    }

    public func cancel() {
        pending = nil
        installError = nil
    }

    private func keep(_ staged: Pending) async {
        installing = true
        defer { installing = false }
        let id = staged.rowId
        progress[id] = InstallProgress(id: id, phase: .installing)
        switch await api.installPack(.staged(token: staged.preview.token)) {
        case .ok:
            pending = nil
            await reload()
            progress[id] = nil
        case .failed(let error):
            pending = nil
            progress[id] = InstallProgress(id: id, phase: .failed, error: error)
            installError = error
        }
    }

    public func removePack(_ id: String) async {
        await act(.remove, id) {
            switch await self.api.removePack(id) {
            case .failed(let error): return error
            case .ok(let count):
                await self.reload()
                return count > 0
                    ? "Removed the files. \(ExtensionCopy.count(count, "connection")) will stop working until the connector is installed again."
                    : nil
            }
        }
    }

    public func rollback(_ id: String) async {
        await act(.rollback, id) {
            switch await self.api.rollbackPack(id) {
            case .failed(let error): return error
            case .ok:
                await self.reload()
                return nil
            }
        }
    }

    // MARK: Connections

    public func deleteConnection(_ id: String) async {
        await act(.delete, id) {
            try await self.api.deleteConnection(id)
            await self.reload()
            return nil
        }
    }

    public func backfill(_ id: String) async {
        backfills[id] = nil
        await act(.backfill, id) {
            self.backfills[id] = try await self.api.backfill(id)
            await self.reload()
            return nil
        }
    }

    public func test(_ id: String) async {
        testing.insert(id)
        tests[id] = nil
        do {
            tests[id] = try await api.preflight(id)
        } catch {
            tests[id] = .init(ok: false, message: ConnectorsAPI.describe(error, fallback: "Failed"))
        }
        testing.remove(id)
    }

    /// Runs a seeded workflow now, then waits up to 15s for the connection to sync.
    public func run(_ workflowId: String, connectionId: String) async {
        await act(.run, workflowId) {
            let since = Date()
            try await self.api.runWorkflow(workflowId)
            for _ in 0..<30 {
                try? await Task.sleep(for: .milliseconds(500))
                await self.reload()
                if let at = self.connections.first(where: { $0.id == connectionId })?.lastSyncAt.flatMap(Self.date), at > since {
                    break
                }
            }
            return nil
        }
    }

    public func resetWorkflow(connectionId: String, event: String) async {
        do {
            try await api.seedWorkflow(connectionId: connectionId, event: event)
            await reload()
        } catch {
            failed[RowAction.run.key(connectionId)] = ConnectorsAPI.describe(error, fallback: "The workflow could not be reset")
        }
    }

    static func date(_ text: String) -> Date? {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let d = f.date(from: text) { return d }
        f.formatOptions = [.withInternetDateTime]
        return f.date(from: text)
    }

    /// use-row-action.ts run: marks the row busy, then keeps the failure it reports or throws.
    private func act(_ action: RowAction, _ id: String, _ body: @MainActor () async throws -> String?) async {
        let key = action.key(id)
        busy[key] = action.phrase
        failed[key] = nil
        do {
            if let error = try await body() { failed[key] = error }
        } catch {
            failed[key] = ConnectorsAPI.describe(error, fallback: "It did not say why.")
        }
        busy[key] = nil
    }
}
