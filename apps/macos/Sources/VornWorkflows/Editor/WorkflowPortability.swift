import Foundation
import VornCore

/// workflow-portability.ts: a workflow's machine-specific paths and connections, made portable and put back.
/// Works on key-ordered JSON so an export is byte-for-byte what `JSON.stringify` writes.
public enum WorkflowPortability {
    /// Stands in for the importing project's directory.
    public static let projectPathToken = "{{project.path}}"
    /// Stands in for the importing project's name.
    public static let projectNameToken = "{{project.name}}"
    public static let formatVersion = 1
    /// The built-in connector whose connections are HTTP auth profiles.
    public static let httpProfileConnector = "http"

    /// Keys a step may simply not have: unset is a finished step, not a question.
    static let optionalConnectionKeys: Set<String> = ["profileConnectionId", "secretsFrom"]

    public static func importedWorkflowId(bundle: String, slug: String) -> String { "import:\(bundle):\(slug)" }

    /// The id an import should take; a derived id held by a workflow of another name steps aside.
    public static func importedWorkflowIdFor(
        bundle: String, slug: String, name: String, existing: [(id: String, name: String)]
    ) -> String {
        var attempt = 1
        while true {
            let id = importedWorkflowId(bundle: bundle, slug: attempt == 1 ? slug : "\(slug)-\(attempt)")
            if let held = existing.first(where: { $0.id == id }), held.name != name {
                attempt += 1
                continue
            }
            return id
        }
    }

    public static func parseImportedWorkflowId(_ id: String) -> (bundle: String, slug: String)? {
        guard id.hasPrefix("import:") else { return nil }
        let rest = id.dropFirst("import:".count)
        guard let colon = rest.firstIndex(of: ":") else { return nil }
        return (String(rest[..<colon]), String(rest[rest.index(after: colon)...]))
    }

    public static func slugify(_ name: String) -> String {
        var out = ""
        var pendingDash = false
        for scalar in name.lowercased().unicodeScalars {
            if ("a"..."z").contains(scalar) || ("0"..."9").contains(scalar) {
                if pendingDash, !out.isEmpty { out += "-" }
                pendingDash = false
                out.unicodeScalars.append(scalar)
            } else {
                pendingDash = true
            }
        }
        let slug = String(out.prefix(60))
        return slug.isEmpty ? "workflow" : slug
    }

    /// The config key that binds this node to a connection, whether it runs against one or borrows its key.
    static func boundConnectionKey(type: String?, config: OObject) -> String? {
        switch type {
        case "trigger": return config["triggerType"] == .string("connectorPoll") ? "connectionId" : nil
        case "callConnectorAction": return "connectionId"
        case "httpRequest": return "profileConnectionId"
        case "script": return "secretsFrom"
        default: return nil
        }
    }

    /// The connection a requirement should bind to here, if this machine has one answer.
    public static func resolveRequirement(
        _ requirement: PortableRequirement, connections: [PortableConnection]
    ) -> String? {
        let candidates = connections.filter { connection in
            requirement.kind == .httpProfile
                ? connection.connector == httpProfileConnector
                : requirement.connectorId != "" && connection.connector == requirement.connectorId
        }
        if candidates.isEmpty { return nil }
        if requirement.name != "" {
            let named = candidates.filter { $0.name == requirement.name }
            if named.count == 1 { return named[0].id }
        }
        return candidates.count == 1 ? candidates[0].id : nil
    }

    /// A file may name the key a script wants; handing one over is a person's choice, made in the step.
    public static func bindsOnlyByHand(_ requirement: PortableRequirement) -> Bool {
        requirement.kind == .connection && requirement.key == "secretsFrom"
    }

    /// Replace this machine's paths with placeholders and unbind its connections.
    static func toPortable(_ workflow: OObject, projectPath: String, connections: [PortableConnection] = []) -> OObject {
        let name = jsString(workflow["name"])
        var requires: [OJSON] = []

        let nodes = (workflow["nodes"]?.arrayValue ?? []).map { raw -> OJSON in
            guard var node = raw.objectValue else { return raw }
            let type = node["type"]?.textValue
            var config = node["config"]?.objectValue ?? OObject()

            // The token guards this install's hook route; it does not belong in a shared file.
            if type == "trigger", config["triggerType"] == .string("webhook") { config["token"] = .string("") }

            if type == "launchAgent" || type == "script" {
                for key in ["projectPath", "cwd", "existingWorktreePath"] {
                    if let value = config[key]?.textValue, !value.isEmpty {
                        config[key] = .string(replacePath(value, projectPath: projectPath))
                    }
                }
                if let value = config["projectName"]?.textValue, !value.isEmpty {
                    config["projectName"] = .string(projectNameToken)
                }
                config["remoteHostId"] = nil
            }

            if let key = boundConnectionKey(type: type, config: config) {
                let bound = config[key]
                let unbound = !optionalConnectionKeys.contains(key) && bound == .string("")
                if let boundId = bound?.textValue, boundId != "" || unbound {
                    let source = connections.first { $0.id == boundId }
                    if key == "profileConnectionId" {
                        requires.append(PortableRequirement.httpProfile(nodeId: nodeId(node), name: source?.name ?? "").portableJSON)
                    } else {
                        let event = config["event"]?.textValue
                        requires.append(
                            PortableRequirement(
                                kind: .connection, nodeId: nodeId(node),
                                connectorId: source?.connector ?? config["connectorId"]?.textValue ?? "",
                                name: source?.name ?? "",
                                event: event?.isEmpty == false ? event : nil,
                                key: key == "secretsFrom" ? key : nil
                            ).portableJSON)
                    }
                    if optionalConnectionKeys.contains(key) { config[key] = nil } else { config[key] = .string("") }
                }
            }

            node["config"] = .object(config)
            return .object(node)
        }

        var out = OObject()
        out["version"] = .number(Double(formatVersion))
        out["slug"] = .string(slugify(name))
        out["name"] = workflow["name"] ?? .null
        if let icon = workflow["icon"], truthy(icon) { out["icon"] = icon }
        if let color = workflow["iconColor"], truthy(color) { out["iconColor"] = color }
        if let stagger = workflow["staggerDelayMs"] { out["staggerDelayMs"] = stagger }
        if !requires.isEmpty { out["requires"] = .array(requires) }
        out["nodes"] = .array(nodes)
        if let edges = workflow["edges"] { out["edges"] = edges }
        return out
    }

    /// Compare paths without caring which separator the machine writes.
    static func normalizeForCompare(_ p: String) -> String {
        var s = p.replacingOccurrences(of: "\\", with: "/")
        while s.hasSuffix("/") { s.removeLast() }
        return s
    }

    static func replacePath(_ value: String, projectPath: String) -> String {
        let v = normalizeForCompare(value)
        let root = normalizeForCompare(projectPath)
        if root.isEmpty { return value }
        if v == root { return projectPathToken }
        if v.hasPrefix("\(root)/") { return "\(projectPathToken)/\(v.dropFirst(root.count + 1))" }
        return value
    }

    /// Which of this file's requirements this machine cannot answer on its own.
    static func unresolvedRequirements(_ portable: OObject, connections: [PortableConnection]) -> [PortableRequirement] {
        let present = Set((portable["nodes"]?.arrayValue ?? []).compactMap { $0.objectValue?["id"]?.textValue })
        return requirements(of: portable).filter { requirement in
            present.contains(requirement.nodeId)
                && (bindsOnlyByHand(requirement) || resolveRequirement(requirement, connections: connections) == nil)
        }
    }

    static func requirements(of portable: OObject) -> [PortableRequirement] {
        (portable["requires"]?.arrayValue ?? []).compactMap { $0.objectValue.map(PortableRequirement.init) }
    }

    /// Resolve placeholders against the project this workflow is being imported into.
    static func fromPortable(
        _ portable: OObject, bundle: String, project: (name: String, path: String),
        connections: [PortableConnection] = [], mintToken: () -> String = { UUID().uuidString.lowercased() }
    ) -> OObject {
        var bindings: [String: [PortableRequirement]] = [:]
        for requirement in requirements(of: portable) { bindings[requirement.nodeId, default: []].append(requirement) }
        var projectPath = project.path
        while projectPath.hasSuffix("/") || projectPath.hasSuffix("\\") { projectPath.removeLast() }

        let nodes = (portable["nodes"]?.arrayValue ?? []).map { raw -> OJSON in
            guard var node = raw.objectValue else { return raw }
            let type = node["type"]?.textValue
            var config = node["config"]?.objectValue ?? OObject()

            // A carried id names a row in the writer's table: dropped rather than bound to whatever holds it here.
            let key = boundConnectionKey(type: type, config: config)
            if let key, optionalConnectionKeys.contains(key) { config[key] = nil }

            for (k, value) in config.entries {
                guard case .string(let s) = value else { continue }
                config[k] = .string(
                    s.replacingOccurrences(of: projectPathToken, with: projectPath)
                        .replacingOccurrences(of: projectNameToken, with: project.name))
            }

            for requirement in bindings[jsString(node["id"])] ?? [] {
                guard !bindsOnlyByHand(requirement), let key else { continue }
                if let resolved = resolveRequirement(requirement, connections: connections) {
                    config[key] = .string(resolved)
                }
            }

            if type == "trigger", config["triggerType"] == .string("webhook"), !truthy(config["token"]) {
                config["token"] = .string(mintToken())
            }

            node["config"] = .object(config)
            return .object(node)
        }

        var out = OObject()
        out["id"] = .string(importedWorkflowId(bundle: bundle, slug: jsString(portable["slug"])))
        out["name"] = portable["name"] ?? .null
        out["icon"] = nonNull(portable["icon"]) ?? .string("Zap")
        out["iconColor"] = nonNull(portable["iconColor"]) ?? .string("#6366f1")
        out["enabled"] = .bool(false)
        if let stagger = portable["staggerDelayMs"] { out["staggerDelayMs"] = stagger }
        out["nodes"] = .array(nodes)
        if let edges = portable["edges"] { out["edges"] = edges }
        return out
    }

    /// What is still machine-specific after export. Empty is the goal.
    static func residualAbsolutePaths(_ portable: OObject) -> [String] {
        var found: [String] = []
        for node in portable["nodes"]?.arrayValue ?? [] {
            guard let node = node.objectValue else { continue }
            for (key, value) in node["config"]?.objectValue?.entries ?? [] {
                if case .string(let s) = value, isMachinePath(s) { found.append("\(jsString(node["id"])).\(key)") }
            }
        }
        return found
    }

    nonisolated(unsafe) private static let machinePath = try! NSRegularExpression(
        pattern: #"(^|["'\s])(/(Users|home)/|[A-Za-z]:[\\/]|\\\\[^\\/\s]+[\\/])"#)

    static func isMachinePath(_ s: String) -> Bool {
        machinePath.firstMatch(in: s, range: NSRange(s.startIndex..., in: s)) != nil
    }

    private static func nodeId(_ node: OObject) -> String { jsString(node["id"]) }

    private static func nonNull(_ v: OJSON?) -> OJSON? {
        guard let v, v != .null else { return nil }
        return v
    }

    /// JavaScript truthiness.
    static func truthy(_ v: OJSON?) -> Bool {
        switch v {
        case .none, .null: return false
        case .bool(let b): return b
        case .number(let n): return n != 0 && !n.isNaN
        case .string(let s): return !s.isEmpty
        case .array, .object: return true
        }
    }

    /// A value as a template literal prints it.
    static func jsString(_ v: OJSON?) -> String {
        switch v {
        case .none: return "undefined"
        case .null: return "null"
        case .bool(let b): return b ? "true" : "false"
        case .number(let n): return OJSON.format(n)
        case .string(let s): return s
        case .array(let a): return a.map { $0 == .null ? "" : jsString($0) }.joined(separator: ",")
        case .object: return "[object Object]"
        }
    }
}

/// What a step needs bound before it can run, named so any machine can match it.
public struct PortableRequirement: Sendable, Hashable {
    public enum Kind: String, Sendable { case connection, httpProfile }

    public var kind: Kind
    public var nodeId: String
    /// Empty when the exporting machine could not name the connector; unused for an HTTP profile.
    public var connectorId: String
    public var name: String
    public var event: String?
    /// A script's key (`secretsFrom`): never bound by an import, only by a person in the step.
    public var key: String?

    public init(
        kind: Kind = .connection, nodeId: String, connectorId: String = "", name: String = "",
        event: String? = nil, key: String? = nil
    ) {
        self.kind = kind
        self.nodeId = nodeId
        self.connectorId = connectorId
        self.name = name
        self.event = event
        self.key = key
    }

    public static func httpProfile(nodeId: String, name: String) -> PortableRequirement {
        PortableRequirement(kind: .httpProfile, nodeId: nodeId, name: name)
    }

    init(_ o: OObject) {
        kind = o["kind"] == .string("httpProfile") ? .httpProfile : .connection
        nodeId = o["nodeId"]?.textValue ?? ""
        connectorId = o["connectorId"]?.textValue ?? ""
        name = o["name"]?.textValue ?? ""
        event = o["event"]?.textValue
        key = o["key"]?.textValue
    }

    public init(json: JSONValue) {
        self.init(OJSON(unordered: json).objectValue ?? OObject())
    }

    /// The requirement as the file writes it, in the TS object's key order.
    var portableJSON: OJSON {
        var o = OObject()
        o["kind"] = .string(kind.rawValue)
        o["nodeId"] = .string(nodeId)
        if kind == .connection { o["connectorId"] = .string(connectorId) }
        o["name"] = .string(name)
        if kind == .connection {
            if let event { o["event"] = .string(event) }
            if let key { o["key"] = .string(key) }
        }
        return .object(o)
    }
}

/// Enough of a connection to match one; `connection:list` rows decode into it.
public struct PortableConnection: Sendable, Hashable, Decodable {
    public var id: String
    public var name: String
    public var connectorId: String
    public var filters: [String: JSONValue]?

    public init(id: String, name: String, connectorId: String, filters: [String: JSONValue]? = nil) {
        self.id = id
        self.name = name
        self.connectorId = connectorId
        self.filters = filters
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = (try? c.decode(String.self, forKey: .name)) ?? ""
        connectorId = (try? c.decode(String.self, forKey: .connectorId)) ?? ""
        filters = try? c.decode([String: JSONValue].self, forKey: .filters)
    }

    private enum CodingKeys: String, CodingKey { case id, name, connectorId, filters }

    /// The connector a connection belongs to, packaged connectors included (connectionConnectorId).
    public var connector: String {
        if let packaged = filters?["sdkConnectorId"]?.stringValue, !packaged.isEmpty { return packaged }
        return connectorId
    }
}

extension OJSON {
    /// An unordered value, its object keys sorted: the order is already lost, so pick one that is stable.
    init(unordered value: JSONValue) {
        switch value {
        case .null: self = .null
        case .bool(let b): self = .bool(b)
        case .number(let n): self = .number(n)
        case .string(let s): self = .string(s)
        case .array(let a): self = .array(a.map { OJSON(unordered: $0) })
        case .object(let o): self = .object(OObject(o.keys.sorted().map { ($0, OJSON(unordered: o[$0]!)) }))
        }
    }

    var unordered: JSONValue {
        switch self {
        case .null: return .null
        case .bool(let b): return .bool(b)
        case .number(let n): return .number(n)
        case .string(let s): return .string(s)
        case .array(let a): return .array(a.map(\.unordered))
        case .object(let o): return .object(Dictionary(o.entries.map { ($0.key, $0.value.unordered) }, uniquingKeysWith: { $1 }))
        }
    }

    var textValue: String? {
        if case .string(let s) = self { return s }
        return nil
    }
}

extension WorkflowDefinition {
    /// The definition as ordered JSON: fields in types.ts order, config keys sorted.
    var portableJSON: OObject {
        var o = OObject()
        o["id"] = .string(id)
        o["name"] = .string(name)
        o["icon"] = .string(icon)
        o["iconColor"] = .string(iconColor)
        o["enabled"] = .bool(enabled)
        if let staggerDelayMs { o["staggerDelayMs"] = .number(staggerDelayMs) }
        if let autoCleanupWorktrees { o["autoCleanupWorktrees"] = .bool(autoCleanupWorktrees) }
        if let workspaceId { o["workspaceId"] = .string(workspaceId) }
        o["nodes"] = .array(nodes.map { .object($0.portableJSON) })
        o["edges"] = .array(edges.map { .object($0.portableJSON) })
        return o
    }

    /// Reads a definition back from ordered JSON, as vornd's would decode.
    init?(portableJSON ordered: OObject) {
        guard let value = try? JSONDecoder().decode(WorkflowDefinition.self, from: Data(OJSON.object(ordered).stringify().utf8))
        else { return nil }
        self = value
    }
}

extension WorkflowNode {
    var portableJSON: OObject {
        var o = OObject()
        o["id"] = .string(id)
        o["type"] = .string(type.rawValue)
        o["label"] = .string(label)
        if let slug { o["slug"] = .string(slug) }
        o["config"] = OJSON(unordered: config)
        o["position"] = .object(OObject([("x", .number(position.x)), ("y", .number(position.y))]))
        if let onError { o["onError"] = .string(onError) }
        return o
    }
}

extension WorkflowEdge {
    var portableJSON: OObject {
        var o = OObject()
        o["id"] = .string(id)
        o["source"] = .string(source)
        o["target"] = .string(target)
        if let conditionBranch { o["conditionBranch"] = .string(conditionBranch) }
        return o
    }
}
