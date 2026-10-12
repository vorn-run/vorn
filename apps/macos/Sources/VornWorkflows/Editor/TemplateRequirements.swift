import Foundation
import VornCore

/// types.ts WorkflowTemplate: a workflow someone can start from, carried as a portable file.
public struct WorkflowTemplate: Sendable, Decodable, Identifiable {
    public var id: String
    public var name: String
    public var description: String
    /// The chain as a person reads it, e.g. ["Webhook", "Condition", "HTTP request"].
    public var steps: [String]
    public var category: String?
    public var portable: JSONValue

    var portableObject: OObject { OJSON(unordered: portable).objectValue ?? OObject() }

    var nodeCount: Int { portable["nodes"]?.arrayValue?.count ?? 0 }
}

/// connector:catalog's snapshot, as much of it as the Start from list reads.
public struct ConnectorCatalog: Sendable, Decodable {
    public var templates: [WorkflowTemplate]

    public init(templates: [WorkflowTemplate]) { self.templates = templates }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        templates = (try? c.decode([WorkflowTemplate].self, forKey: .templates)) ?? []
    }

    private enum CodingKeys: String, CodingKey { case templates }
}

/// A requirement, and the connection that answers it here when there is one.
public struct TemplateRequirement: Sendable, Hashable {
    public var requirement: PortableRequirement
    public var connectionId: String?
}

/// A workflow a connection's connector ships, offered beside the templates.
public struct ConnectorSuggestion: Sendable, Hashable, Identifiable {
    public var key: String
    public var connectionId: String
    public var connectionName: String
    public var event: String
    public var name: String
    public var id: String { key }
}

/// connector:list rows: the connector id and the workflows its manifest ships.
public struct ConnectorListing: Sendable, Decodable {
    public struct Seeded: Sendable, Decodable {
        public var name: String
        public var event: String
    }

    public var id: String
    public var defaultWorkflows: [Seeded]

    public init(id: String, defaultWorkflows: [Seeded]) {
        self.id = id
        self.defaultWorkflows = defaultWorkflows
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        let manifest = try? c.nestedContainer(keyedBy: CodingKeys.self, forKey: .manifest)
        defaultWorkflows = (try? manifest?.decode([Seeded].self, forKey: .defaultWorkflows)) ?? []
    }

    private enum CodingKeys: String, CodingKey { case id, manifest, defaultWorkflows }
}

/// template-requirements.ts: what a template needs, what it puts on the canvas, what the canvas still misses.
public enum TemplateRequirements {
    public static func connectorSuggestions(
        connections: [PortableConnection], connectors: [ConnectorListing]
    ) -> [ConnectorSuggestion] {
        let manifests = Dictionary(connectors.map { ($0.id, $0.defaultWorkflows) }, uniquingKeysWith: { $1 })
        return connections.flatMap { connection in
            (manifests[connection.connector] ?? []).map { seeded in
                ConnectorSuggestion(
                    key: "\(connection.id):\(seeded.event)", connectionId: connection.id,
                    connectionName: connection.name, event: seeded.event, name: seeded.name)
            }
        }
    }

    /// What the workflow on the canvas is still missing.
    public static func requirementsOfDefinition(_ nodes: [WorkflowNode]) -> [PortableRequirement] {
        var requirements: [PortableRequirement] = []
        for node in nodes {
            let config = OJSON(unordered: node.config).objectValue ?? OObject()
            guard let key = WorkflowPortability.boundConnectionKey(type: node.type.rawValue, config: config) else { continue }
            if let bound = config[key]?.textValue, !bound.isEmpty { continue }
            if key == "profileConnectionId" {
                // A request with no profile field at all is a request to a public URL.
                if !config.has(key) { continue }
                requirements.append(.httpProfile(nodeId: node.id, name: ""))
                continue
            }
            let connectorId = config["connectorId"]?.textValue ?? ""
            if connectorId.isEmpty { continue }
            requirements.append(PortableRequirement(nodeId: node.id, connectorId: connectorId))
        }
        return requirements
    }

    /// One list of gaps from the canvas and the file.
    public static func mergeRequirements(
        _ derived: [PortableRequirement], _ imported: [PortableRequirement]
    ) -> [PortableRequirement] {
        var byNode: [String: PortableRequirement] = [:]
        var order: [String] = []
        for requirement in imported {
            if byNode[requirement.nodeId] == nil { order.append(requirement.nodeId) }
            byNode[requirement.nodeId] = requirement
        }
        let merged = derived.map { fromCanvas -> PortableRequirement in
            guard var fromFile = byNode.removeValue(forKey: fromCanvas.nodeId) else { return fromCanvas }
            if fromFile.kind != .connection || fromCanvas.kind != .connection { return fromFile }
            if fromFile.connectorId.isEmpty { fromFile.connectorId = fromCanvas.connectorId }
            if fromFile.name.isEmpty { fromFile.name = fromCanvas.name }
            return fromFile
        }
        return merged + order.compactMap { byNode[$0] }
    }

    public static func requirementsWithBindings(
        _ requirements: [PortableRequirement], connections: [PortableConnection]
    ) -> [TemplateRequirement] {
        requirements.map { requirement in
            // A key a person has to hand over stays open here, whatever this machine holds.
            TemplateRequirement(
                requirement: requirement,
                connectionId: WorkflowPortability.bindsOnlyByHand(requirement)
                    ? nil : WorkflowPortability.resolveRequirement(requirement, connections: connections))
        }
    }

    public static func templateRequirements(
        _ template: WorkflowTemplate, connections: [PortableConnection]
    ) -> [TemplateRequirement] {
        requirementsWithBindings(WorkflowPortability.requirements(of: template.portableObject), connections: connections)
    }

    public static func templateIsReady(_ template: WorkflowTemplate, connections: [PortableConnection]) -> Bool {
        WorkflowPortability.unresolvedRequirements(template.portableObject, connections: connections).isEmpty
    }

    /// The canvas a template starts: this project's paths, this machine's connections, a webhook token of its own.
    public static func templateSeed(
        _ template: WorkflowTemplate, project: (name: String, path: String)?, connections: [PortableConnection],
        mintToken: () -> String = { UUID().uuidString.lowercased() }
    ) -> WorkflowSeed {
        let portable = template.portableObject
        var definition = WorkflowPortability.fromPortable(
            portable, bundle: "template", project: project ?? ("", ""), connections: connections)
        let nodes = (definition["nodes"]?.arrayValue ?? []).map { raw -> OJSON in
            guard var node = raw.objectValue, var config = node["config"]?.objectValue,
                  config["triggerType"] == .string("webhook")
            else { return raw }
            // Always this machine's own token: one the catalog carried would be shared with every install.
            config["token"] = .string(mintToken())
            node["config"] = .object(config)
            return .object(node)
        }
        definition["nodes"] = .array(nodes)
        definition["name"] = .string(template.name)
        let decoded = WorkflowDefinition(portableJSON: definition)
        return WorkflowSeed(
            name: template.name,
            icon: definition["icon"]?.textValue.flatMap { $0.isEmpty ? nil : $0 },
            iconColor: definition["iconColor"]?.textValue.flatMap { $0.isEmpty ? nil : $0 },
            nodes: decoded?.nodes ?? [],
            edges: decoded?.edges ?? [],
            unresolved: WorkflowPortability.unresolvedRequirements(portable, connections: connections)
                .map(WorkflowFiles.describeRequirement))
    }
}

extension WorkflowsStore {
    /// connector:catalog's templates, empty when vornd has none to offer.
    /// Waits up to `seconds` for vornd's background catalog refresh, as a person opening the list would.
    public func awaitFreshCatalog(seconds: Double = 4) async {
        let deadline = Date().addingTimeInterval(seconds)
        while Date() < deadline {
            if (try? await backend.rpc("connector:catalog", nil))?["fetchedAt"]?.isNull == false { return }
            try? await Task.sleep(for: .milliseconds(200))
        }
    }

    func workflowTemplates() async -> [WorkflowTemplate] {
        (try? await backend.rpc("connector:catalog", nil, as: ConnectorCatalog.self))?.templates ?? []
    }

    /// The workflows this machine's connections already know how to build.
    func connectorListings() async -> [ConnectorListing] {
        (try? await backend.rpc("connector:list", nil, as: [ConnectorListing].self)) ?? []
    }
}

