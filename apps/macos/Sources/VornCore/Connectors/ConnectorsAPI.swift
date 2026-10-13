import Foundation

/// The vornd calls behind the Plugins page (use-connections.ts, use-connector-catalog.ts, use-extensions.ts, use-pack-install.ts).
public struct ConnectorsAPI: Sendable {
    public let client: VornClient

    public init(client: VornClient) {
        self.client = client
    }

    /// A pack to read or install: a file on disk, a published URL, or one already staged.
    public enum PackSource: Hashable, Sendable {
        case file(path: String)
        case url(String, sha256: String?)
        case staged(token: String)

        var json: JSONValue {
            switch self {
            case .file(let path): ["kind": "file", "path": .string(path)]
            case .url(let url, let sha):
                sha.map { ["kind": "url", "url": .string(url), "sha256": .string($0)] } ?? ["kind": "url", "url": .string(url)]
            case .staged(let token): ["kind": "staged", "token": .string(token)]
            }
        }
    }

    /// An answer that is either a value or the reason there is none.
    public enum Outcome<Value: Sendable>: Sendable {
        case ok(Value)
        case failed(String)
    }

    public struct BackfillResult: Hashable, Sendable {
        public var imported: Int
        public var updated: Int
        public var error: String?

        public init(imported: Int, updated: Int, error: String? = nil) {
            self.imported = imported
            self.updated = updated
            self.error = error
        }
    }

    public struct PreflightResult: Hashable, Sendable {
        public var ok: Bool?
        public var message: String?

        public init(ok: Bool?, message: String?) {
            self.ok = ok
            self.message = message
        }
    }

    public struct Repo: Hashable, Sendable {
        public var owner: String
        public var repo: String
    }

    // MARK: Reads

    public func builtIns() async throws -> [BuiltInConnector] {
        try await client.call("connector:list").arrayValue?.compactMap(BuiltInConnector.init(json:)) ?? []
    }

    public func connections() async throws -> [SourceConnection] {
        try await client.call("connection:list", .object([:])).arrayValue?.compactMap(SourceConnection.init(json:)) ?? []
    }

    public func packs() async throws -> [InstalledPack] {
        try await client.call("connector:listPacks").arrayValue?.compactMap(InstalledPack.init(json:)) ?? []
    }

    public func extensions() async throws -> [InstalledPack] {
        try await client.call("extension:list").arrayValue?.compactMap(InstalledPack.init(json:)) ?? []
    }

    public func catalog() async throws -> CatalogSnapshot {
        CatalogSnapshot(json: try await client.call("connector:catalog"))
    }

    /// Fetches the published catalog again; this goes to the network.
    public func refreshCatalog() async throws -> CatalogSnapshot {
        CatalogSnapshot(json: try await client.call("connector:catalogRefresh"))
    }

    /// Whether each built-in connector's tool is signed in; this asks every tool, so it can be slow.
    public func statuses() async throws -> [ConnectorAuthStatus] {
        try await client.call("connector:status").arrayValue?.compactMap(ConnectorAuthStatus.init(json:)) ?? []
    }

    public func activation(sessionId: String) async throws -> [ExtensionActivationState] {
        try await client.call("extension:activation", ["sessionId": .string(sessionId)]).arrayValue?
            .compactMap(ExtensionActivationState.init(json:)) ?? []
    }

    // MARK: Packs

    public func inspectPack(_ source: PackSource) async -> Outcome<PackPreview> {
        do {
            let answer = try await client.call("connector:inspectPack", source.json)
            if answer.flag("ok") == true, let preview = answer["preview"].flatMap(PackPreview.init(json:)) { return .ok(preview) }
            return .failed(answer.text("error") ?? "The pack could not be checked")
        } catch {
            return .failed(Self.describe(error, fallback: "The pack could not be checked"))
        }
    }

    public func installPack(_ source: PackSource) async -> Outcome<InstalledPack> {
        do {
            let answer = try await client.call("connector:installPack", source.json)
            if answer.flag("ok") == true, let pack = answer["pack"].flatMap(InstalledPack.init(json:)) { return .ok(pack) }
            return .failed(answer.text("error") ?? "The pack could not be installed")
        } catch {
            return .failed(Self.describe(error, fallback: "The pack could not be installed"))
        }
    }

    /// Removes a pack's files; the count is the connections that stop working with it.
    public func removePack(_ id: String) async -> Outcome<Int> {
        do {
            let answer = try await client.call("connector:removePack", .string(id))
            if answer.flag("ok") == true { return .ok(Int(answer.number("connections") ?? 0)) }
            return .failed(answer.text("error") ?? "It did not say why.")
        } catch {
            return .failed(Self.describe(error, fallback: "It did not say why."))
        }
    }

    public func rollbackPack(_ id: String) async -> Outcome<InstalledPack?> {
        do {
            let answer = try await client.call("connector:rollbackPack", .string(id))
            if answer.flag("ok") == true { return .ok(answer["pack"].flatMap(InstalledPack.init(json:))) }
            return .failed(answer.text("error") ?? "It did not say why.")
        } catch {
            return .failed(Self.describe(error, fallback: "It did not say why."))
        }
    }

    // MARK: Connections

    @discardableResult
    public func createConnection(connectorId: String, name: String, filters: [String: JSONValue], statusMapping: [String: String] = [:],
                                 executionProject: String?, syncIntervalMinutes: Double = 5,
                                 seedWorkflow: JSONValue? = nil) async throws -> SourceConnection? {
        var params: [String: JSONValue] = [
            "connectorId": .string(connectorId), "name": .string(name), "filters": .object(filters),
            "syncIntervalMinutes": .number(syncIntervalMinutes),
            "statusMapping": .object(statusMapping.mapValues { .string($0) }),
        ]
        if let executionProject, !executionProject.isEmpty { params["executionProject"] = .string(executionProject) }
        if let seedWorkflow, seedWorkflow != .null { params["seedWorkflow"] = seedWorkflow }
        return SourceConnection(json: try await client.call("connection:create", .object(params)))
    }

    public func deleteConnection(_ id: String) async throws {
        _ = try await client.call("connection:delete", .string(id))
    }

    public func backfill(_ connectionId: String) async throws -> BackfillResult {
        let answer = try await client.call("connection:backfill", ["connectionId": .string(connectionId)])
        return BackfillResult(imported: Int(answer.number("imported") ?? 0), updated: Int(answer.number("updated") ?? 0),
                              error: answer.text("error"))
    }

    public func preflight(_ connectionId: String) async throws -> PreflightResult {
        let answer = try await client.call("connection:preflight", .string(connectionId))
        return PreflightResult(ok: answer.flag("ok"), message: answer.text("message"))
    }

    public func seedWorkflow(connectionId: String, event: String) async throws {
        _ = try await client.call("connector:seedWorkflow", ["connectionId": .string(connectionId), "event": .string(event)])
    }

    public func runWorkflow(_ workflowId: String) async throws {
        _ = try await client.call("workflow:runManual", ["workflowId": .string(workflowId)])
    }

    public func detectRepo(_ projectPath: String) async throws -> Repo? {
        let answer = try await client.call("connector:detectRepo", .string(projectPath))
        guard let owner = answer.text("owner"), let repo = answer.text("repo") else { return nil }
        return Repo(owner: owner, repo: repo)
    }

    /// Asks a package for its manifest by starting it once.
    public func probeSdk(_ launch: LaunchSpec, env: [String: String] = [:]) async -> Outcome<SdkManifest> {
        var params: [String: JSONValue] = ["command": .string(launch.command), "args": .array(launch.args.map { .string($0) })]
        if !env.isEmpty { params["env"] = .object(env.mapValues { .string($0) }) }
        do {
            let answer = try await client.call("connector:probeSdk", .object(params))
            if answer.flag("ok") == true, let manifest = answer["manifest"].flatMap(SdkManifest.init(json:)) { return .ok(manifest) }
            return .failed(answer.text("error") ?? "The package did not describe itself.")
        } catch {
            return .failed(Self.describe(error, fallback: "The package did not describe itself."))
        }
    }

    /// Asks the tool a connector borrows whether it is signed in, and as whom.
    public func probeAuth(_ connectorId: String) async -> AuthProbeReport {
        (try? await client.call("connector:probeAuth", .string(connectorId))).map(AuthProbeReport.init(json:)) ?? AuthProbeReport()
    }

    // MARK: Pushes

    public func installProgress() async -> AsyncStream<InstallProgress> {
        let raw = await client.notifications("connector:installProgress")
        return AsyncStream { continuation in
            let task = Task {
                for await value in raw { if let p = InstallProgress(json: value) { continuation.yield(p) } }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    public func catalogChanges() async -> AsyncStream<CatalogSnapshot> {
        let raw = await client.notifications("connector:catalogChanged")
        return AsyncStream { continuation in
            let task = Task {
                for await value in raw { continuation.yield(CatalogSnapshot(json: value)) }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    public static func describe(_ error: Error, fallback: String) -> String {
        let text = (error as? LocalizedError)?.errorDescription ?? "\(error)"
        return text.isEmpty ? fallback : text
    }
}

/// A workflow seeded for a connection, which polls it on a schedule (ConnectionGroups.tsx seededFor).
public struct SeededWorkflow: Hashable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var cron: String

    public init(id: String, name: String, cron: String) {
        self.id = id
        self.name = name
        self.cron = cron
    }

    public init?(json: JSONValue) {
        guard let id = json.text("id") else { return nil }
        let trigger = json.list("nodes").first { $0.text("type") == "trigger" }
        self.init(id: id, name: json.text("name") ?? id, cron: trigger?["config"]?.text("cron") ?? "")
    }

    public static func seeded(_ workflows: [SeededWorkflow], for connectionId: String) -> [SeededWorkflow] {
        workflows.filter { $0.id.hasPrefix("connector:\(connectionId):") }
    }

    /// missingFor: events the connector polls but has no workflow for, so polling silently stopped.
    public static func missing(_ workflows: [SeededWorkflow], for connectionId: String,
                               manifest: ConnectorManifest?) -> [ConnectorManifest.DefaultWorkflow] {
        let ids = Set(seeded(workflows, for: connectionId).map(\.id))
        return (manifest?.defaultWorkflows ?? []).filter { !ids.contains("connector:\(connectionId):\($0.event)") }
    }
}
