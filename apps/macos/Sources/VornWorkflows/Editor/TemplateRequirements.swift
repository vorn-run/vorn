import Foundation
import VornCore

/// workflow-portability.ts PortableRequirement: a connection or profile a step still needs.
struct PortableRequirement: Hashable, Sendable {
    enum Kind: String, Sendable { case connection, httpProfile }
    var kind: Kind
    var nodeId: String
    var connectorId: String = ""
    var name: String = ""
    var key: String?

    /// workflow-files.ts describeRequirement.
    var description: String {
        if kind == .httpProfile { return name.isEmpty ? "an HTTP profile" : "an HTTP profile like \"\(name)\"" }
        let connector = connectorId.isEmpty ? "connector" : connectorId
        if key == "secretsFrom" {
            return "a \(connector) key for its Secrets from" + (name.isEmpty ? "" : " (it used \"\(name)\")")
        }
        return name.isEmpty ? "a \(connector) connection" : "a \(connector) connection like \"\(name)\""
    }
}

enum TemplateRequirements {
    /// workflow-portability.ts boundConnectionKey: the config key that binds a step to a connection.
    static func boundConnectionKey(_ node: WorkflowNode) -> String? {
        switch node.type {
        case .trigger: return node.config.str("triggerType") == "connectorPoll" ? "connectionId" : nil
        case .callConnectorAction: return "connectionId"
        case .httpRequest: return "profileConnectionId"
        case .script: return "secretsFrom"
        default: return nil
        }
    }

    /// template-requirements.ts requirementsOfDefinition: what the canvas cannot run yet.
    static func requirementsOfDefinition(_ nodes: [WorkflowNode]) -> [PortableRequirement] {
        nodes.compactMap { node in
            guard let key = boundConnectionKey(node) else { return nil }
            if case .string(let bound)? = node.config[key], !bound.isEmpty { return nil }
            if key == "profileConnectionId" {
                guard node.config[key] != nil else { return nil }
                return PortableRequirement(kind: .httpProfile, nodeId: node.id)
            }
            let connectorId = node.config.str("connectorId") ?? ""
            guard !connectorId.isEmpty else { return nil }
            return PortableRequirement(kind: .connection, nodeId: node.id, connectorId: connectorId)
        }
    }
}
