import Foundation

// Connector and extension data as vornd sends it (packages/shared/src/types.ts), read leniently from JSON.

extension JSONValue {
    func text(_ key: String) -> String? { self[key]?.stringValue }
    func flag(_ key: String) -> Bool? { self[key]?.boolValue }
    func number(_ key: String) -> Double? { self[key]?.numberValue }
    func list(_ key: String) -> [JSONValue] { self[key]?.arrayValue ?? [] }
    func strings(_ key: String) -> [String] { list(key).compactMap(\.stringValue) }
}

/// SdkConnectorIcon: a package's glyph, path data only.
public struct ConnectorIconData: Hashable, Sendable {
    public var viewBox: String
    public var paths: [String]

    public init(viewBox: String = "0 0 24 24", paths: [String]) {
        self.viewBox = viewBox
        self.paths = paths
    }

    public init?(json: JSONValue?) {
        guard let json, case .object = json else { return nil }
        let raw = json.list("paths")
        let paths = raw.compactMap(\.stringValue)
        guard !paths.isEmpty, paths.count == raw.count, paths.allSatisfy({ !$0.isEmpty }) else { return nil }
        self.init(viewBox: json.text("viewBox") ?? "0 0 24 24", paths: paths)
    }

    /// connection-icon.ts connectionIcon: the glyph a connection's `sdkIcon` filter holds as JSON.
    public init?(filter raw: String?) {
        guard let raw, !raw.isEmpty, let data = raw.data(using: .utf8),
              let json = try? JSONDecoder().decode(JSONValue.self, from: data) else { return nil }
        self.init(json: json)
    }
}

public enum ConnectorKind: String, Hashable, Sendable, CaseIterable {
    case connector, `extension`
}

public enum ConnectorAuthRung: String, Hashable, Sendable, CaseIterable {
    case none, cli, key, browser, oauth
}

public enum ExtensionPermission: String, Hashable, Sendable, CaseIterable {
    case gitRead = "git.read"
    case terminalRead = "terminal.read"
    case terminalSelection = "terminal.selection"
    case agentUsage = "agent.usage"
    case terminalSend = "terminal.send"
    case cardRename = "card.rename"
}

/// ConnectorConfigField.
public struct ConnectorField: Hashable, Sendable, Identifiable {
    public struct Option: Hashable, Sendable {
        public var value: String
        public var label: String
        public init(value: String, label: String) {
            self.value = value
            self.label = label
        }
    }
    public var key: String
    public var label: String
    public var type: String
    public var required: Bool
    public var placeholder: String?
    public var description: String?
    public var options: [Option]
    public var id: String { key }

    public init(key: String, label: String, type: String = "text", required: Bool = false,
                placeholder: String? = nil, description: String? = nil, options: [Option] = []) {
        self.key = key
        self.label = label
        self.type = type
        self.required = required
        self.placeholder = placeholder
        self.description = description
        self.options = options
    }

    init?(json: JSONValue) {
        guard let key = json.text("key") else { return nil }
        self.init(key: key, label: json.text("label") ?? key, type: json.text("type") ?? "text",
                  required: json.flag("required") ?? false, placeholder: json.text("placeholder"),
                  description: json.text("description"),
                  options: json.list("options").compactMap { o in
                      o.text("value").map { Option(value: $0, label: o.text("label") ?? $0) }
                  })
    }
}

/// A trigger or action as a listing describes it.
public struct ConnectorCapability: Hashable, Sendable, Identifiable {
    public var type: String
    public var label: String
    public var description: String?
    public var id: String { type }

    public init(type: String, label: String, description: String? = nil) {
        self.type = type
        self.label = label
        self.description = description
    }

    init?(json: JSONValue) {
        guard let type = json.text("type") else { return nil }
        self.init(type: type, label: json.text("label") ?? type, description: json.text("description"))
    }
}

/// ConnectorManifest, what a built-in connector describes itself as.
public struct ConnectorManifest: Hashable, Sendable {
    public struct DefaultWorkflow: Hashable, Sendable {
        public var name: String
        public var event: String
        public init(name: String, event: String) {
            self.name = name
            self.event = event
        }
    }
    public struct StatusOption: Hashable, Sendable {
        public var upstream: String
        public var local: String
    }
    public var auth: [ConnectorField]
    public var detectRepo: Bool
    public var taskFilters: [ConnectorField]
    public var statusMapping: [StatusOption]
    public var triggers: [ConnectorCapability]
    public var actions: [ConnectorCapability]
    public var defaultWorkflows: [DefaultWorkflow]

    public init(auth: [ConnectorField] = [], detectRepo: Bool = false, taskFilters: [ConnectorField] = [],
                statusMapping: [StatusOption] = [], triggers: [ConnectorCapability] = [], actions: [ConnectorCapability] = [],
                defaultWorkflows: [DefaultWorkflow] = []) {
        self.auth = auth
        self.detectRepo = detectRepo
        self.taskFilters = taskFilters
        self.statusMapping = statusMapping
        self.triggers = triggers
        self.actions = actions
        self.defaultWorkflows = defaultWorkflows
    }

    init(json: JSONValue) {
        self.init(
            auth: json.list("auth").compactMap(ConnectorField.init(json:)),
            detectRepo: json.flag("detectRepo") ?? false,
            taskFilters: json.list("taskFilters").compactMap(ConnectorField.init(json:)),
            statusMapping: json.list("statusMapping").compactMap { s in
                guard let u = s.text("upstream"), let l = s.text("suggestedLocal") else { return nil }
                return StatusOption(upstream: u, local: l)
            },
            triggers: json.list("triggers").compactMap(ConnectorCapability.init(json:)),
            actions: json.list("actions").compactMap(ConnectorCapability.init(json:)),
            defaultWorkflows: json.list("defaultWorkflows").compactMap { w in
                guard let event = w.text("event") else { return nil }
                return DefaultWorkflow(name: w.text("name") ?? event, event: event)
            })
    }
}

/// connector:list: a connector compiled into vornd.
public struct BuiltInConnector: Hashable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var capabilities: [String]
    public var addable: Bool?
    public var manifest: ConnectorManifest?

    public init(id: String, name: String, capabilities: [String] = [], addable: Bool? = nil, manifest: ConnectorManifest? = nil) {
        self.id = id
        self.name = name
        self.capabilities = capabilities
        self.addable = addable
        self.manifest = manifest
    }

    public init?(json: JSONValue) {
        guard let id = json.text("id") else { return nil }
        self.init(id: id, name: json.text("name") ?? id, capabilities: json.strings("capabilities"),
                  addable: json.flag("addable"), manifest: json["manifest"].map(ConnectorManifest.init(json:)))
    }
}

/// The filters a packaged connector records itself under (SDK_FILTER_KEYS).
public enum SdkFilterKey {
    public static let connectorId = "sdkConnectorId"
    public static let version = "sdkVersion"
    public static let icon = "sdkIcon"
    public static let implicit = "implicit"
    public static let trigger = "sdkTrigger"
    /// SDK_CONNECTOR_ID: the connector every package connection belongs to.
    public static let sdkConnector = "sdk"
}

/// SourceConnection: a saved, configured link to a connector.
public struct SourceConnection: Hashable, Sendable, Identifiable {
    public var id: String
    public var connectorId: String
    public var name: String
    public var filters: [String: JSONValue]
    public var syncIntervalMinutes: Double
    public var executionProject: String?
    public var lastSyncAt: String?
    public var lastSyncError: String?
    public var createdAt: String?
    public var signedInAs: String?
    public var signedInAt: String?

    public init(id: String, connectorId: String, name: String, filters: [String: JSONValue] = [:],
                syncIntervalMinutes: Double = 5, executionProject: String? = nil, lastSyncAt: String? = nil,
                lastSyncError: String? = nil, createdAt: String? = nil, signedInAs: String? = nil, signedInAt: String? = nil) {
        self.id = id
        self.connectorId = connectorId
        self.name = name
        self.filters = filters
        self.syncIntervalMinutes = syncIntervalMinutes
        self.executionProject = executionProject
        self.lastSyncAt = lastSyncAt
        self.lastSyncError = lastSyncError
        self.createdAt = createdAt
        self.signedInAs = signedInAs
        self.signedInAt = signedInAt
    }

    public init?(json: JSONValue) {
        guard let id = json.text("id"), let connectorId = json.text("connectorId") else { return nil }
        self.init(id: id, connectorId: connectorId, name: json.text("name") ?? id,
                  filters: json["filters"]?.objectValue ?? [:], syncIntervalMinutes: json.number("syncIntervalMinutes") ?? 5,
                  executionProject: json.text("executionProject"), lastSyncAt: json.text("lastSyncAt"),
                  lastSyncError: json.text("lastSyncError"), createdAt: json.text("createdAt"),
                  signedInAs: json.text("signedInAs"), signedInAt: json.text("signedInAt"))
    }

    /// connectionConnectorId: for a package's `sdk` connection, the package it runs.
    public var realConnectorId: String {
        if let packaged = filters[SdkFilterKey.connectorId]?.stringValue, !packaged.isEmpty { return packaged }
        return connectorId
    }

    /// isImplicitConnection: made by the app for a connector that asks for nothing.
    public var isImplicit: Bool { filters[SdkFilterKey.implicit] == .bool(true) }

    /// connectionIcon: the glyph the connection stored.
    public var icon: ConnectorIconData? { ConnectorIconData(filter: filters[SdkFilterKey.icon]?.stringValue) }
}

/// SdkEnvVar.
public struct ConnectorEnvVar: Hashable, Sendable, Identifiable {
    public var name: String
    public var required: Bool
    public var secret: Bool
    public var description: String?
    public var id: String { name }

    public init(name: String, required: Bool = false, secret: Bool = false, description: String? = nil) {
        self.name = name
        self.required = required
        self.secret = secret
        self.description = description
    }

    init?(json: JSONValue) {
        guard let name = json.text("name") else { return nil }
        self.init(name: name, required: json.flag("required") ?? false, secret: json.flag("secret") ?? false,
                  description: json.text("description"))
    }
}

/// SdkTrigger.
public struct SdkTrigger: Hashable, Sendable, Identifiable {
    public var type: String
    public var label: String
    public var description: String?
    public var statusMapping: [String: String]
    /// Passed back unchanged as the connection's `seedWorkflow`.
    public var defaultWorkflow: JSONValue?
    public var id: String { type }

    public init(type: String, label: String, description: String? = nil, statusMapping: [String: String] = [:], defaultWorkflow: JSONValue? = nil) {
        self.type = type
        self.label = label
        self.description = description
        self.statusMapping = statusMapping
        self.defaultWorkflow = defaultWorkflow
    }

    init?(json: JSONValue) {
        guard let type = json.text("type") else { return nil }
        var mapping: [String: String] = [:]
        for s in json.list("statusMapping") {
            if let u = s.text("upstream"), let l = s.text("suggestedLocal") { mapping[u] = l }
        }
        self.init(type: type, label: json.text("label") ?? type, description: json.text("description"),
                  statusMapping: mapping, defaultWorkflow: json["defaultWorkflow"])
    }

    public var capability: ConnectorCapability { ConnectorCapability(type: type, label: label, description: description) }
}

/// SdkConnectorAuth.
public struct SdkAuth: Hashable, Sendable {
    public var rung: ConnectorAuthRung?
    public var probeCommand: String?
    public var probeArgs: [String]
    public var borrowEnv: [String]
    public var browserSignInUrl: String?
    public var browserOrigins: [String]

    public init(rung: ConnectorAuthRung? = nil, probeCommand: String? = nil, probeArgs: [String] = [], borrowEnv: [String] = [],
                browserSignInUrl: String? = nil, browserOrigins: [String] = []) {
        self.rung = rung
        self.probeCommand = probeCommand
        self.probeArgs = probeArgs
        self.borrowEnv = borrowEnv
        self.browserSignInUrl = browserSignInUrl
        self.browserOrigins = browserOrigins
    }

    init?(json: JSONValue?) {
        guard let json, case .object = json else { return nil }
        self.init(rung: json.text("rung").flatMap(ConnectorAuthRung.init(rawValue:)),
                  probeCommand: json["probe"]?.text("command"), probeArgs: json["probe"]?.strings("args") ?? [],
                  borrowEnv: json["borrow"]?.strings("env") ?? [],
                  browserSignInUrl: json["browser"]?.text("signInUrl"), browserOrigins: json["browser"]?.strings("origins") ?? [])
    }

    public var signsInThroughWindow: Bool { browserSignInUrl != nil }
}

/// ExtensionActivation.
public struct ExtensionActivation: Hashable, Sendable {
    public var workspaceContains: [String]
    public var remoteHost: [String]
    public var agent: [String]
    public var platform: [String]

    public init(workspaceContains: [String] = [], remoteHost: [String] = [], agent: [String] = [], platform: [String] = []) {
        self.workspaceContains = workspaceContains
        self.remoteHost = remoteHost
        self.agent = agent
        self.platform = platform
    }

    init?(json: JSONValue?) {
        guard let json, case .object = json else { return nil }
        self.init(workspaceContains: json.strings("workspaceContains"), remoteHost: json.strings("remoteHost"),
                  agent: json.strings("agent"), platform: json.strings("platform"))
    }
}

/// One pane, footer or link handler an extension contributes.
public struct ExtensionContribution: Hashable, Sendable, Identifiable {
    public var id: String
    public var title: String
    public var description: String?
    public var when: ExtensionActivation?
    public var icon: ConnectorIconData?
    public var web: String?
    public var command: [String]
    public var every: Double
    public var pattern: String?

    public init(id: String, title: String, description: String? = nil, when: ExtensionActivation? = nil, icon: ConnectorIconData? = nil,
                web: String? = nil, command: [String] = [], every: Double = 0, pattern: String? = nil) {
        self.id = id
        self.title = title
        self.description = description
        self.when = when
        self.icon = icon
        self.web = web
        self.command = command
        self.every = every
        self.pattern = pattern
    }

    init?(json: JSONValue) {
        guard let id = json.text("id") else { return nil }
        self.init(id: id, title: json.text("title") ?? id, description: json.text("description"),
                  when: ExtensionActivation(json: json["when"]), icon: ConnectorIconData(json: json["icon"]),
                  web: json.text("web"), command: json.strings("command"), every: json.number("every") ?? 0,
                  pattern: json.text("pattern"))
    }
}

/// ExtensionContributions.
public struct ExtensionContributions: Hashable, Sendable {
    public var panes: [ExtensionContribution]
    public var footers: [ExtensionContribution]
    public var linkHandlers: [ExtensionContribution]

    public init(panes: [ExtensionContribution] = [], footers: [ExtensionContribution] = [], linkHandlers: [ExtensionContribution] = []) {
        self.panes = panes
        self.footers = footers
        self.linkHandlers = linkHandlers
    }

    init?(json: JSONValue?) {
        guard let json, case .object = json else { return nil }
        self.init(panes: json.list("panes").compactMap(ExtensionContribution.init(json:)),
                  footers: json.list("footers").compactMap(ExtensionContribution.init(json:)),
                  linkHandlers: json.list("linkHandlers").compactMap(ExtensionContribution.init(json:)))
    }

    public var all: [ExtensionContribution] { panes + footers + linkHandlers }
}

func readPermissions(_ json: JSONValue?) -> [ExtensionPermission]? {
    json?.arrayValue?.compactMap { $0.stringValue.flatMap(ExtensionPermission.init(rawValue:)) }
}

/// InstalledConnectorPack (connector:listPacks, extension:list).
public struct InstalledPack: Hashable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var version: String
    public var kind: ConnectorKind?
    public var description: String?
    public var icon: ConnectorIconData?
    public var auth: SdkAuth?
    public var path: String
    public var previousVersion: String?
    public var installedAt: Double?
    public var bytes: Double
    public var triggers: [SdkTrigger]
    public var actions: [ConnectorCapability]
    public var env: [ConnectorEnvVar]
    public var contributes: ExtensionContributions?
    public var permissions: [ExtensionPermission]?
    public var activates: ExtensionActivation?

    public init(id: String, name: String, version: String, kind: ConnectorKind? = nil, description: String? = nil,
                icon: ConnectorIconData? = nil, auth: SdkAuth? = nil, path: String = "", previousVersion: String? = nil,
                installedAt: Double? = nil, bytes: Double = 0, triggers: [SdkTrigger] = [], actions: [ConnectorCapability] = [],
                env: [ConnectorEnvVar] = [], contributes: ExtensionContributions? = nil, permissions: [ExtensionPermission]? = nil,
                activates: ExtensionActivation? = nil) {
        self.id = id
        self.name = name
        self.version = version
        self.kind = kind
        self.description = description
        self.icon = icon
        self.auth = auth
        self.path = path
        self.previousVersion = previousVersion
        self.installedAt = installedAt
        self.bytes = bytes
        self.triggers = triggers
        self.actions = actions
        self.env = env
        self.contributes = contributes
        self.permissions = permissions
        self.activates = activates
    }

    public init?(json: JSONValue) {
        guard let id = json.text("id") else { return nil }
        self.init(id: id, name: json.text("name") ?? id, version: json.text("version") ?? "",
                  kind: json.text("kind").flatMap(ConnectorKind.init(rawValue:)), description: json.text("description"),
                  icon: ConnectorIconData(json: json["icon"]), auth: SdkAuth(json: json["auth"]), path: json.text("path") ?? "",
                  previousVersion: json.text("previousVersion"), installedAt: json.number("installedAt"),
                  bytes: json.number("bytes") ?? 0, triggers: json.list("triggers").compactMap(SdkTrigger.init(json:)),
                  actions: json.list("actions").compactMap(ConnectorCapability.init(json:)),
                  env: json.list("env").compactMap(ConnectorEnvVar.init(json:)),
                  contributes: ExtensionContributions(json: json["contributes"]), permissions: readPermissions(json["permissions"]),
                  activates: ExtensionActivation(json: json["activates"]))
    }

    /// pack-status.ts packLaunch: what a pack's child runs.
    public var launch: LaunchSpec { LaunchSpec(command: "node", args: ["\(path)/index.js"]) }
}

/// A command and its arguments, as a connector is started.
public struct LaunchSpec: Hashable, Sendable {
    public var command: String
    public var args: [String]

    public init(command: String, args: [String]) {
        self.command = command
        self.args = args
    }

    /// parse-launch-spec.ts: a bare package name runs through npx; anything else is a command line.
    public init(parsing input: String) {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        let parts = trimmed.split(whereSeparator: \.isWhitespace).map(String.init)
        let local = trimmed.hasPrefix(".") || trimmed.hasPrefix("/") || trimmed.hasPrefix("~")
            || trimmed.range(of: "^[a-zA-Z]:\\\\", options: .regularExpression) != nil
        if parts.count == 1 && !local {
            self.init(command: "npx", args: ["-y", trimmed])
        } else {
            self.init(command: parts.first ?? "", args: Array(parts.dropFirst()))
        }
    }

    public var json: JSONValue { .object(["command": .string(command), "args": .array(args.map { .string($0) })]) }
}

/// ConnectorPackSummary: a staged pack, shown before it is kept.
public struct PackPreview: Hashable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var version: String
    public var kind: ConnectorKind?
    public var description: String?
    public var icon: ConnectorIconData?
    public var auth: SdkAuth?
    public var triggers: [SdkTrigger]
    public var actions: [ConnectorCapability]
    public var env: [ConnectorEnvVar]
    public var contributes: ExtensionContributions?
    public var permissions: [ExtensionPermission]?
    public var activates: ExtensionActivation?
    public var installedVersion: String?
    public var token: String

    public init(id: String, name: String, version: String, kind: ConnectorKind? = nil, description: String? = nil,
                icon: ConnectorIconData? = nil, auth: SdkAuth? = nil, triggers: [SdkTrigger] = [], actions: [ConnectorCapability] = [],
                env: [ConnectorEnvVar] = [], contributes: ExtensionContributions? = nil, permissions: [ExtensionPermission]? = nil,
                activates: ExtensionActivation? = nil, installedVersion: String? = nil, token: String = "") {
        self.id = id
        self.name = name
        self.version = version
        self.kind = kind
        self.description = description
        self.icon = icon
        self.auth = auth
        self.triggers = triggers
        self.actions = actions
        self.env = env
        self.contributes = contributes
        self.permissions = permissions
        self.activates = activates
        self.installedVersion = installedVersion
        self.token = token
    }

    public init?(json: JSONValue) {
        guard let id = json.text("id") else { return nil }
        self.init(id: id, name: json.text("name") ?? id, version: json.text("version") ?? "",
                  kind: json.text("kind").flatMap(ConnectorKind.init(rawValue:)), description: json.text("description"),
                  icon: ConnectorIconData(json: json["icon"]), auth: SdkAuth(json: json["auth"]),
                  triggers: json.list("triggers").compactMap(SdkTrigger.init(json:)),
                  actions: json.list("actions").compactMap(ConnectorCapability.init(json:)),
                  env: json.list("env").compactMap(ConnectorEnvVar.init(json:)),
                  contributes: ExtensionContributions(json: json["contributes"]), permissions: readPermissions(json["permissions"]),
                  activates: ExtensionActivation(json: json["activates"]), installedVersion: json.text("installedVersion"),
                  token: json.text("token") ?? "")
    }
}

/// ConnectorCatalogVerification.
public struct CatalogVerification: Hashable, Sendable {
    public var version: String
    public var checkedAt: String
    public var checks: [String]

    public init(version: String, checkedAt: String, checks: [String]) {
        self.version = version
        self.checkedAt = checkedAt
        self.checks = checks
    }

    init?(json: JSONValue?) {
        // Only the receipt format this build knows: a later one may vouch for something else.
        guard let json, json.number("schema") == 1, let version = json.text("version") else { return nil }
        self.init(version: version, checkedAt: json.text("checkedAt") ?? "", checks: json.strings("checks"))
    }
}

/// ConnectorCatalogItem: one published connector or extension.
public struct CatalogItem: Hashable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var description: String
    public var kind: ConnectorKind?
    public var packageName: String
    public var version: String?
    public var packUrl: String?
    public var sha256: String?
    public var capabilities: [String]
    public var auth: String?
    public var authRung: ConnectorAuthRung?
    public var verified: CatalogVerification?
    public var icon: ConnectorIconData?
    public var category: String?
    public var keywords: [String]
    public var triggers: [ConnectorCapability]?
    public var actions: [ConnectorCapability]?
    public var env: [ConnectorEnvVar]?
    public var contributes: ExtensionContributions?
    public var permissions: [ExtensionPermission]?
    public var activates: ExtensionActivation?
    public var launch: LaunchSpec?

    public init(id: String, name: String, description: String = "", kind: ConnectorKind? = nil, packageName: String = "",
                version: String? = nil, packUrl: String? = nil, sha256: String? = nil, capabilities: [String] = [],
                auth: String? = nil, authRung: ConnectorAuthRung? = nil, verified: CatalogVerification? = nil,
                icon: ConnectorIconData? = nil, category: String? = nil, keywords: [String] = [],
                triggers: [ConnectorCapability]? = nil, actions: [ConnectorCapability]? = nil, env: [ConnectorEnvVar]? = nil,
                contributes: ExtensionContributions? = nil, permissions: [ExtensionPermission]? = nil,
                activates: ExtensionActivation? = nil, launch: LaunchSpec? = nil) {
        self.id = id
        self.name = name
        self.description = description
        self.kind = kind
        self.packageName = packageName
        self.version = version
        self.packUrl = packUrl
        self.sha256 = sha256
        self.capabilities = capabilities
        self.auth = auth
        self.authRung = authRung
        self.verified = verified
        self.icon = icon
        self.category = category
        self.keywords = keywords
        self.triggers = triggers
        self.actions = actions
        self.env = env
        self.contributes = contributes
        self.permissions = permissions
        self.activates = activates
        self.launch = launch
    }

    public init?(json: JSONValue) {
        guard let id = json.text("id") else { return nil }
        self.init(id: id, name: json.text("name") ?? id, description: json.text("description") ?? "",
                  kind: json.text("kind").flatMap(ConnectorKind.init(rawValue:)), packageName: json.text("packageName") ?? "",
                  version: json.text("version"), packUrl: json.text("packUrl"), sha256: json.text("sha256"),
                  capabilities: json.strings("capabilities"), auth: json.text("auth"),
                  authRung: json.text("authRung").flatMap(ConnectorAuthRung.init(rawValue:)),
                  verified: CatalogVerification(json: json["verified"]), icon: ConnectorIconData(json: json["icon"]),
                  category: json.text("category"), keywords: json.strings("keywords"),
                  triggers: json["triggers"]?.arrayValue?.compactMap(ConnectorCapability.init(json:)),
                  actions: json["actions"]?.arrayValue?.compactMap(ConnectorCapability.init(json:)),
                  env: json["env"]?.arrayValue?.compactMap(ConnectorEnvVar.init(json:)),
                  contributes: ExtensionContributions(json: json["contributes"]), permissions: readPermissions(json["permissions"]),
                  activates: ExtensionActivation(json: json["activates"]),
                  launch: json["launch"]?.text("command").map { LaunchSpec(command: $0, args: json["launch"]?.strings("args") ?? []) })
    }
}

/// McpServerCatalogEntry: a plain MCP server the catalog lists.
public struct McpServerEntry: Hashable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var description: String?
    public var command: String
    public var args: [String]
    public var category: String?
    public var keywords: [String]

    public init(id: String, name: String, description: String? = nil, command: String, args: [String] = [],
                category: String? = nil, keywords: [String] = []) {
        self.id = id
        self.name = name
        self.description = description
        self.command = command
        self.args = args
        self.category = category
        self.keywords = keywords
    }

    public init?(json: JSONValue) {
        guard let id = json.text("id"), let command = json.text("command") else { return nil }
        self.init(id: id, name: json.text("name") ?? id, description: json.text("description"), command: command,
                  args: json.strings("args"), category: json.text("category"), keywords: json.strings("keywords"))
    }
}

/// use-connector-catalog.ts CatalogSnapshot.
public struct CatalogSnapshot: Hashable, Sendable {
    public var items: [CatalogItem]
    public var mcpServers: [McpServerEntry]
    public var fetchedAt: Double?

    public init(items: [CatalogItem] = [], mcpServers: [McpServerEntry] = [], fetchedAt: Double? = nil) {
        self.items = items
        self.mcpServers = mcpServers
        self.fetchedAt = fetchedAt
    }

    public init(json: JSONValue) {
        self.init(items: json.list("items").compactMap(CatalogItem.init(json:)),
                  mcpServers: json.list("mcpServers").compactMap(McpServerEntry.init(json:)), fetchedAt: json.number("fetchedAt"))
    }
}

/// ConnectorInstallProgress (the `connector:installProgress` broadcast).
public struct InstallProgress: Hashable, Sendable {
    public enum Phase: String, Hashable, Sendable {
        case checking, downloading, verifying, installing, installed, failed
    }
    public var id: String
    public var phase: Phase
    public var percent: Double?
    public var version: String?
    public var error: String?

    public init(id: String, phase: Phase, percent: Double? = nil, version: String? = nil, error: String? = nil) {
        self.id = id
        self.phase = phase
        self.percent = percent
        self.version = version
        self.error = error
    }

    public init?(json: JSONValue) {
        guard let id = json.text("id"), let phase = json.text("phase").flatMap(Phase.init(rawValue:)) else { return nil }
        self.init(id: id, phase: phase, percent: json.number("percent"), version: json.text("version"), error: json.text("error"))
    }
}

/// connector:status: whether a built-in connector's tool is signed in.
public struct ConnectorAuthStatus: Hashable, Sendable {
    public var connectorId: String
    public var authed: Bool
    public var message: String?

    public init(connectorId: String, authed: Bool, message: String? = nil) {
        self.connectorId = connectorId
        self.authed = authed
        self.message = message
    }

    public init?(json: JSONValue) {
        guard let id = json.text("connectorId") else { return nil }
        self.init(connectorId: id, authed: json.flag("authed") ?? false, message: json.text("message"))
    }
}

/// AuthProbeReport: what asking a borrowed tool answered; `ok == nil` is "nothing to ask".
public struct AuthProbeReport: Hashable, Sendable {
    public var ok: Bool?
    public var identity: String?
    public var message: String?
    public var installHint: String?

    public init(ok: Bool? = nil, identity: String? = nil, message: String? = nil, installHint: String? = nil) {
        self.ok = ok
        self.identity = identity
        self.message = message
        self.installHint = installHint
    }

    public init(json: JSONValue) {
        self.init(ok: json.flag("ok"), identity: json.text("identity"), message: json.text("message"), installHint: json.text("installHint"))
    }
}

/// SdkConnectorManifest, as connector:probeSdk reads it from a package.
public struct SdkManifest: Hashable, Sendable {
    public var id: String
    public var name: String
    public var version: String
    public var description: String?
    public var icon: ConnectorIconData?
    public var auth: SdkAuth?
    public var triggers: [SdkTrigger]
    public var actions: [ConnectorCapability]
    public var env: [ConnectorEnvVar]

    public init(id: String, name: String, version: String, description: String? = nil, icon: ConnectorIconData? = nil,
                auth: SdkAuth? = nil, triggers: [SdkTrigger] = [], actions: [ConnectorCapability] = [], env: [ConnectorEnvVar] = []) {
        self.id = id
        self.name = name
        self.version = version
        self.description = description
        self.icon = icon
        self.auth = auth
        self.triggers = triggers
        self.actions = actions
        self.env = env
    }

    public init?(json: JSONValue) {
        guard let id = json.text("id") else { return nil }
        self.init(id: id, name: json.text("name") ?? id, version: json.text("version") ?? "", description: json.text("description"),
                  icon: ConnectorIconData(json: json["icon"]), auth: SdkAuth(json: json["auth"]),
                  triggers: json.list("triggers").compactMap(SdkTrigger.init(json:)),
                  actions: json.list("actions").compactMap(ConnectorCapability.init(json:)),
                  env: json.list("env").compactMap(ConnectorEnvVar.init(json:)))
    }
}

/// ExtensionActivationState (extension:activation for one session).
public struct ExtensionActivationState: Hashable, Sendable {
    public var extensionId: String
    public var extensionName: String
    public var active: Bool
    public var panes: [String]
    public var footers: [String]
    public var linkHandlers: [String]

    public init(extensionId: String, extensionName: String = "", active: Bool, panes: [String] = [], footers: [String] = [], linkHandlers: [String] = []) {
        self.extensionId = extensionId
        self.extensionName = extensionName
        self.active = active
        self.panes = panes
        self.footers = footers
        self.linkHandlers = linkHandlers
    }

    public init?(json: JSONValue) {
        guard let id = json.text("extensionId") else { return nil }
        self.init(extensionId: id, extensionName: json.text("extensionName") ?? id, active: json.flag("active") ?? false,
                  panes: json.strings("panes"), footers: json.strings("footers"), linkHandlers: json.strings("linkHandlers"))
    }

    /// Whether it draws anything on the card.
    public var draws: Bool { !(panes.isEmpty && footers.isEmpty && linkHandlers.isEmpty) }
}
