import Foundation
import VornCore

/// A connection as the forms read it (`SourceConnection` in types.ts).
public struct FormConnection: Decodable, Hashable, Sendable, Identifiable {
    public var id: String
    public var connectorId: String
    public var name: String
    public var filters: JSONValue?

    public init(id: String, connectorId: String, name: String, filters: JSONValue? = nil) {
        self.id = id
        self.connectorId = connectorId
        self.name = name
        self.filters = filters
    }

    /// connectionConnectorId: the real connector behind an `sdk` connection.
    public var realConnectorId: String {
        if connectorId == "sdk", let id = filters?.str("sdkConnectorId"), !id.isEmpty { return id }
        return connectorId
    }
}

/// One input of a connector action (`ConnectorConfigField`).
public struct ConnectorConfigField: Decodable, Hashable, Sendable {
    public struct Option: Decodable, Hashable, Sendable {
        public var value: String
        public var label: String

        public init(value: String, label: String) {
            self.value = value
            self.label = label
        }

        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            value = try c.decode(String.self, forKey: .value)
            label = (try? c.decode(String.self, forKey: .label)) ?? value
        }

        private enum CodingKeys: String, CodingKey { case value, label }
    }

    public var key: String
    public var label: String
    public var type: String
    public var required: Bool?
    public var placeholder: String?
    public var description: String?
    public var options: [Option]?

    public init(
        key: String, label: String, type: String = "text", required: Bool? = nil, placeholder: String? = nil,
        description: String? = nil, options: [Option]? = nil
    ) {
        self.key = key
        self.label = label
        self.type = type
        self.required = required
        self.placeholder = placeholder
        self.description = description
        self.options = options
    }
}

/// An action a connection can run (`ConnectorActionDef`).
public struct ConnectorActionDef: Decodable, Hashable, Sendable {
    public var type: String
    public var label: String
    public var description: String?
    public var configFields: [ConnectorConfigField]
    public var outputSchema: JSONValue?

    public init(
        type: String, label: String, description: String? = nil, configFields: [ConnectorConfigField] = [],
        outputSchema: JSONValue? = nil
    ) {
        self.type = type
        self.label = label
        self.description = description
        self.configFields = configFields
        self.outputSchema = outputSchema
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        type = try c.decode(String.self, forKey: .type)
        label = (try? c.decode(String.self, forKey: .label)) ?? type
        description = try? c.decode(String.self, forKey: .description)
        configFields = (try? c.decode([ConnectorConfigField].self, forKey: .configFields)) ?? []
        outputSchema = try? c.decode(JSONValue.self, forKey: .outputSchema)
    }

    private enum CodingKeys: String, CodingKey { case type, label, description, configFields, outputSchema }
}

/// A connector and the poll events it offers (`connector:list`).
public struct ConnectorInfo: Decodable, Hashable, Sendable {
    public struct Trigger: Decodable, Hashable, Sendable {
        public var type: String
        public var label: String

        public init(type: String, label: String) {
            self.type = type
            self.label = label
        }
    }

    public var id: String
    public var name: String
    public var triggers: [Trigger]

    public init(id: String, name: String, triggers: [Trigger]) {
        self.id = id
        self.name = name
        self.triggers = triggers
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = (try? c.decode(String.self, forKey: .name)) ?? id
        let manifest = try? c.decode(JSONValue.self, forKey: .manifest)
        triggers = (manifest?["triggers"]?.arrayValue ?? []).compactMap { t in
            guard let type = t.str("type") else { return nil }
            return Trigger(type: type, label: t.str("label") ?? type)
        }
    }

    private enum CodingKeys: String, CodingKey { case id, name, manifest }
}

/// A connection holding secrets (`ConnectorKey`).
public struct ConnectorKeyInfo: Decodable, Hashable, Sendable {
    public var connectionId: String
    public var name: String

    public init(connectionId: String, name: String) {
        self.connectionId = connectionId
        self.name = name
    }
}

/// The models an agent offers (`AgentModelCatalog`).
public struct AgentModelCatalog: Decodable, Hashable, Sendable {
    public struct Choice: Decodable, Hashable, Sendable {
        public var id: String
        public var label: String
        public var description: String?

        public init(id: String, label: String, description: String? = nil) {
            self.id = id
            self.label = label
            self.description = description
        }
    }

    public var choices: [Choice]
    public var status: String
    public var fetchedAt: Double?
    public var error: String?

    public init(choices: [Choice], status: String = "fresh", fetchedAt: Double? = nil, error: String? = nil) {
        self.choices = choices
        self.status = status
        self.fetchedAt = fetchedAt
        self.error = error
    }
}

/// A todo task a prompt can come from (`task:list`).
public struct FormTask: Decodable, Hashable, Sendable {
    public var id: String
    public var title: String

    public init(id: String, title: String) {
        self.id = id
        self.title = title
    }
}

/// What `config.defaults` holds for the step forms.
public struct FormDefaults: Hashable, Sendable {
    /// workflow-graph.ts DEFAULT_STEP_TIMEOUT_MINUTES.
    public static let stepTimeoutMinutes: Double = 60

    public var defaultAgent: String
    public var headlessStepTimeoutMinutes: Double

    public init(defaultAgent: String = "claude", headlessStepTimeoutMinutes: Double = stepTimeoutMinutes) {
        self.defaultAgent = defaultAgent
        self.headlessStepTimeoutMinutes = headlessStepTimeoutMinutes
    }
}

extension ConnectorConfigField {
    /// types.ts actionInputField: a select keeps its choices, json becomes a textarea, the rest text.
    static func catalogInput(_ input: JSONValue) -> ConnectorConfigField? {
        guard let key = input.str("key") else { return nil }
        let label = input.str("label") ?? key
        let required = input.bool("required") ?? false
        let description = input.str("description").flatMap { $0.isEmpty ? nil : $0 }
        let options = (input["options"]?.arrayValue ?? []).compactMap { o in
            o.str("value").map { Option(value: $0, label: o.str("label") ?? $0) }
        }
        switch input.str("type") {
        case "select" where !options.isEmpty:
            return .init(key: key, label: label, type: "select", required: required, description: description, options: options)
        case "json":
            return .init(key: key, label: label, type: "textarea", required: required, placeholder: "{} or []", description: description)
        default:
            return .init(key: key, label: label, type: "text", required: required, description: description)
        }
    }
}

/// Typed vornd calls the step forms make.
extension WorkflowsBackend {
    public func listConnections() async throws -> [FormConnection] {
        try await rpc("connection:list", .object([:]), as: [FormConnection].self)
    }

    public func listConnectionActions(_ connectionId: String) async throws -> [ConnectorActionDef] {
        try await rpc("connection:listActions", .string(connectionId), as: [ConnectorActionDef].self)
    }

    public func listConnectors() async throws -> [ConnectorInfo] {
        try await rpc("connector:list", nil, as: [ConnectorInfo].self)
    }

    public func listConnectorKeys() async throws -> [ConnectorKeyInfo] {
        try await rpc("connection:listKeys", nil, as: [ConnectorKeyInfo].self)
    }

    public func webhookBaseURL() async throws -> String? {
        try await rpc("webhook:info", nil).str("baseUrl")
    }

    public func isGitRepo(_ path: String) async throws -> Bool {
        try await rpc("git:isGitRepo", .string(path)).boolValue ?? false
    }

    public func listWorktrees(_ path: String) async throws -> [Worktree] {
        try await rpc("git:listWorktrees", .string(path), as: [Worktree].self)
    }

    public func detectInstalledAgents() async throws -> [String: Bool] {
        let raw = try await rpc("agent:detectInstalled", nil).objectValue ?? [:]
        return raw.compactMapValues(\.boolValue)
    }

    public func listModels(agentType: String, projectPath: String, refresh: Bool = false) async throws
        -> AgentModelCatalog
    {
        var params: JSONValue = ["agentType": .string(agentType), "projectPath": .string(projectPath)]
        if refresh { params.set("refresh", bool: true) }
        return try await rpc("agent:listModels", params, as: AgentModelCatalog.self)
    }

    public func listTodoTasks(projectName: String) async throws -> [FormTask] {
        try await rpc(
            "task:list", ["projectName": .string(projectName), "status": "todo"], as: [FormTask].self)
    }

    /// The `defaults` the step forms read from `config:load`: the default agent and the headless timeout.
    public func formDefaults() async throws -> FormDefaults {
        let defaults = try await rpc("config:load", nil)["defaults"]
        return FormDefaults(
            defaultAgent: defaults?.str("defaultAgent") ?? "claude",
            headlessStepTimeoutMinutes: defaults?.num("headlessStepTimeoutMinutes") ?? FormDefaults.stepTimeoutMinutes)
    }

    /// use-connector-catalog.ts: each published connector's actions, as argument fields (`actionInputField`).
    public func connectorCatalogActions() async throws -> [String: [ConnectorActionDef]] {
        let items = try await rpc("connector:catalog", nil)["items"]?.arrayValue ?? []
        var out: [String: [ConnectorActionDef]] = [:]
        for item in items {
            guard let id = item.str("id") else { continue }
            out[id] = (item["actions"]?.arrayValue ?? []).compactMap { a in
                guard let type = a.str("type") else { return nil }
                let fields = (a["inputs"]?.arrayValue ?? []).compactMap(ConnectorConfigField.catalogInput)
                return ConnectorActionDef(
                    type: type, label: a.str("label") ?? type, description: a.str("description"), configFields: fields)
            }
        }
        return out
    }

    public func getWorkflow(_ id: String) async throws -> WorkflowDefinition? {
        let raw = try await rpc("workflow:get", ["id": .string(id)])
        return raw.isNull ? nil : try raw.decode(WorkflowDefinition.self)
    }

    /// Writes a draft's nodes and edges through `workflow:update`.
    public func saveGraph(workflowId: String, nodes: [WorkflowNode], edges: [WorkflowEdge]) async throws {
        let updates: JSONValue = ["nodes": try JSONValue.from(nodes), "edges": try JSONValue.from(edges)]
        _ = try await rpc("workflow:update", ["id": .string(workflowId), "updates": updates])
    }
}
