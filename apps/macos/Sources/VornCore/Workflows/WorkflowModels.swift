import Foundation

// Wire shapes of packages/shared/src/types.ts, decoded tolerantly: unknown
// fields are ignored and missing optional fields stay nil.

public enum WorkflowNodeType: String, Sendable, Codable, CaseIterable {
    case trigger, launchAgent, script, condition, approval, createTaskFromItem
    case callConnectorAction, httpRequest, loop
    case unknown

    public init(from decoder: Decoder) throws {
        self = WorkflowNodeType(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unknown
    }
}

public struct WorkflowNodePosition: Sendable, Hashable, Codable {
    public var x: Double
    public var y: Double

    public init(x: Double, y: Double) {
        self.x = x
        self.y = y
    }
}

public struct WorkflowNode: Sendable, Hashable, Codable, Identifiable {
    public var id: String
    public var type: WorkflowNodeType
    public var label: String
    public var slug: String?
    public var config: JSONValue
    public var position: WorkflowNodePosition
    public var onError: String?
    /// Keys this build does not model, written back untouched so a save loses nothing.
    public var extra: [String: JSONValue] = [:]

    public init(
        id: String, type: WorkflowNodeType, label: String, slug: String? = nil,
        config: JSONValue = .object([:]), position: WorkflowNodePosition = .init(x: 0, y: 0), onError: String? = nil
    ) {
        self.id = id
        self.type = type
        self.label = label
        self.slug = slug
        self.config = config
        self.position = position
        self.onError = onError
    }

    public init(from decoder: Decoder) throws {
        var raw = try decoder.singleValueContainer().decode([String: JSONValue].self)
        guard let id = raw.removeValue(forKey: "id")?.stringValue else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "node without id"))
        }
        self.id = id
        let rawType = raw["type"]
        type = rawType?.stringValue.flatMap(WorkflowNodeType.init(rawValue:)) ?? .unknown
        if type != .unknown { raw["type"] = nil }
        label = raw.removeValue(forKey: "label")?.stringValue ?? ""
        slug = raw.takeString("slug")
        config = raw.removeValue(forKey: "config") ?? .object([:])
        if let p = raw["position"], let x = p["x"]?.numberValue, let y = p["y"]?.numberValue {
            position = .init(x: x, y: y)
            raw["position"] = nil
        } else {
            position = .init(x: 0, y: 0)
        }
        onError = raw.takeString("onError")
        extra = raw
    }

    public func encode(to encoder: Encoder) throws {
        var out = extra
        out["id"] = .string(id)
        if type != .unknown { out["type"] = .string(type.rawValue) }
        out["label"] = .string(label)
        if let slug { out["slug"] = .string(slug) }
        out["config"] = config
        out["position"] = .object(["x": .number(position.x), "y": .number(position.y)])
        if let onError { out["onError"] = .string(onError) }
        var c = encoder.singleValueContainer()
        try c.encode(out)
    }

    /// A trigger's `triggerType`, nil for other nodes.
    public var triggerType: String? {
        type == .trigger ? config["triggerType"]?.stringValue : nil
    }
}

public struct WorkflowEdge: Sendable, Hashable, Codable, Identifiable {
    public var id: String
    public var source: String
    public var target: String
    public var conditionBranch: String?
    /// Keys this build does not model, written back untouched.
    public var extra: [String: JSONValue] = [:]

    public init(id: String, source: String, target: String, conditionBranch: String? = nil) {
        self.id = id
        self.source = source
        self.target = target
        self.conditionBranch = conditionBranch
    }

    public init(from decoder: Decoder) throws {
        var raw = try decoder.singleValueContainer().decode([String: JSONValue].self)
        guard let id = raw.takeString("id"), let source = raw.takeString("source"),
            let target = raw.takeString("target")
        else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "malformed edge"))
        }
        self.id = id
        self.source = source
        self.target = target
        conditionBranch = raw.takeString("conditionBranch")
        extra = raw
    }

    public func encode(to encoder: Encoder) throws {
        var out = extra
        out["id"] = .string(id)
        out["source"] = .string(source)
        out["target"] = .string(target)
        if let conditionBranch { out["conditionBranch"] = .string(conditionBranch) }
        var c = encoder.singleValueContainer()
        try c.encode(out)
    }
}

public struct WorkflowDefinition: Sendable, Hashable, Codable, Identifiable {
    public var id: String
    public var name: String
    public var icon: String
    public var iconColor: String
    public var nodes: [WorkflowNode]
    public var edges: [WorkflowEdge]
    public var enabled: Bool
    public var lastRunAt: String?
    public var lastRunStatus: String?
    public var staggerDelayMs: Double?
    public var workspaceId: String?
    public var autoCleanupWorktrees: Bool?
    /// Keys this build does not model, written back untouched.
    public var extra: [String: JSONValue] = [:]

    public init(
        id: String, name: String, icon: String = "Zap", iconColor: String = "#6b7280",
        nodes: [WorkflowNode] = [], edges: [WorkflowEdge] = [], enabled: Bool = true,
        lastRunAt: String? = nil, lastRunStatus: String? = nil, workspaceId: String? = nil
    ) {
        self.id = id
        self.name = name
        self.icon = icon
        self.iconColor = iconColor
        self.nodes = nodes
        self.edges = edges
        self.enabled = enabled
        self.lastRunAt = lastRunAt
        self.lastRunStatus = lastRunStatus
        self.workspaceId = workspaceId
    }

    private enum CodingKeys: String, CodingKey {
        case id, name, icon, iconColor, nodes, edges, enabled, lastRunAt, lastRunStatus, staggerDelayMs
        case workspaceId, autoCleanupWorktrees
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = (try? c.decode(String.self, forKey: .name)) ?? ""
        icon = (try? c.decode(String.self, forKey: .icon)) ?? ""
        iconColor = (try? c.decode(String.self, forKey: .iconColor)) ?? ""
        nodes = (try? c.decode([LossyNode].self, forKey: .nodes))?.compactMap(\.node) ?? []
        edges = (try? c.decode([LossyEdge].self, forKey: .edges))?.compactMap(\.edge) ?? []
        enabled = (try? c.decode(Bool.self, forKey: .enabled)) ?? false
        lastRunAt = try? c.decode(String.self, forKey: .lastRunAt)
        lastRunStatus = try? c.decode(String.self, forKey: .lastRunStatus)
        staggerDelayMs = try? c.decode(Double.self, forKey: .staggerDelayMs)
        workspaceId = try? c.decode(String.self, forKey: .workspaceId)
        autoCleanupWorktrees = try? c.decode(Bool.self, forKey: .autoCleanupWorktrees)
        var raw = (try? decoder.singleValueContainer().decode([String: JSONValue].self)) ?? [:]
        for key in [CodingKeys.id, .name, .icon, .iconColor, .nodes, .edges, .enabled, .lastRunAt, .lastRunStatus,
                    .staggerDelayMs, .workspaceId, .autoCleanupWorktrees] {
            raw[key.rawValue] = nil
        }
        extra = raw
    }

    public func encode(to encoder: Encoder) throws {
        var out = extra
        out["id"] = .string(id)
        out["name"] = .string(name)
        out["icon"] = .string(icon)
        out["iconColor"] = .string(iconColor)
        out["nodes"] = try JSONValue.from(nodes)
        out["edges"] = try JSONValue.from(edges)
        out["enabled"] = .bool(enabled)
        if let lastRunAt { out["lastRunAt"] = .string(lastRunAt) }
        if let lastRunStatus { out["lastRunStatus"] = .string(lastRunStatus) }
        if let staggerDelayMs { out["staggerDelayMs"] = .number(staggerDelayMs) }
        if let workspaceId { out["workspaceId"] = .string(workspaceId) }
        if let autoCleanupWorktrees { out["autoCleanupWorktrees"] = .bool(autoCleanupWorktrees) }
        var c = encoder.singleValueContainer()
        try c.encode(out)
    }

    public var workspace: String { workspaceId ?? "personal" }

    public var triggerNode: WorkflowNode? { nodes.first { $0.type == .trigger } }

    /// Has a trigger that fires on its own: anything but manual (workflow-helpers isScheduledWorkflow).
    public var isScheduled: Bool {
        nodes.contains { $0.type == .trigger && ($0.triggerType ?? "manual") != "manual" }
    }

    public func node(_ id: String) -> WorkflowNode? { nodes.first { $0.id == id } }
}

/// One element of a list that skips what does not decode instead of failing the list.
private struct LossyNode: Decodable {
    let node: WorkflowNode?
    init(from decoder: Decoder) throws { node = try? WorkflowNode(from: decoder) }
}

private struct LossyEdge: Decodable {
    let edge: WorkflowEdge?
    init(from decoder: Decoder) throws { edge = try? WorkflowEdge(from: decoder) }
}

extension Dictionary where Key == String, Value == JSONValue {
    /// Removes and returns `key` when it holds a string; any other value stays for the round trip.
    fileprivate mutating func takeString(_ key: String) -> String? {
        guard let s = self[key]?.stringValue else { return nil }
        self[key] = nil
        return s
    }
}

public enum NodeExecutionStatus: String, Sendable, Codable {
    case pending, running, success, error, skipped, waiting

    public init(from decoder: Decoder) throws {
        self = NodeExecutionStatus(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .pending
    }
}

public enum RunStatus: String, Sendable, Codable {
    case running, success, error, cancelled

    public init(from decoder: Decoder) throws {
        self = RunStatus(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .error
    }
}

public enum GateDecision: String, Sendable, Codable {
    case approve, reject, changes
}

public struct GateFeedbackEntry: Sendable, Hashable, Codable {
    public var round: Int
    public var decision: String
    public var comment: String
    public var at: String
    public var edited: String?

    public init(round: Int, decision: String, comment: String, at: String, edited: String? = nil) {
        self.round = round
        self.decision = decision
        self.comment = comment
        self.at = at
        self.edited = edited
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        round = (try? c.decode(Int.self, forKey: .round)) ?? 1
        decision = (try? c.decode(String.self, forKey: .decision)) ?? ""
        comment = (try? c.decode(String.self, forKey: .comment)) ?? ""
        at = (try? c.decode(String.self, forKey: .at)) ?? ""
        edited = try? c.decode(String.self, forKey: .edited)
    }
}

public struct NodeExecutionState: Sendable, Hashable, Codable {
    public var nodeId: String
    public var status: NodeExecutionStatus
    public var skipReason: String?
    public var waitingFor: String?
    public var startedAt: String?
    public var completedAt: String?
    public var sessionId: String?
    public var error: String?
    public var logs: String?
    public var output: String?
    public var structuredOutput: JSONValue?
    public var iteration: Int?
    public var taskId: String?
    public var agentType: String?
    public var projectName: String?
    public var approvedAt: String?
    public var rejectedAt: String?
    public var message: String?
    public var viewToken: String?
    public var round: Int?
    public var feedback: [GateFeedbackEntry]?
    public var editableText: String?
    public var editedText: String?
    public var diagnostics: String?

    public init(nodeId: String, status: NodeExecutionStatus) {
        self.nodeId = nodeId
        self.status = status
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        nodeId = try c.decode(String.self, forKey: .nodeId)
        status = (try? c.decode(NodeExecutionStatus.self, forKey: .status)) ?? .pending
        skipReason = try? c.decode(String.self, forKey: .skipReason)
        waitingFor = try? c.decode(String.self, forKey: .waitingFor)
        startedAt = try? c.decode(String.self, forKey: .startedAt)
        completedAt = try? c.decode(String.self, forKey: .completedAt)
        sessionId = try? c.decode(String.self, forKey: .sessionId)
        error = try? c.decode(String.self, forKey: .error)
        logs = try? c.decode(String.self, forKey: .logs)
        output = try? c.decode(String.self, forKey: .output)
        structuredOutput = try? c.decode(JSONValue.self, forKey: .structuredOutput)
        iteration = try? c.decode(Int.self, forKey: .iteration)
        taskId = try? c.decode(String.self, forKey: .taskId)
        agentType = try? c.decode(String.self, forKey: .agentType)
        projectName = try? c.decode(String.self, forKey: .projectName)
        approvedAt = try? c.decode(String.self, forKey: .approvedAt)
        rejectedAt = try? c.decode(String.self, forKey: .rejectedAt)
        message = try? c.decode(String.self, forKey: .message)
        viewToken = try? c.decode(String.self, forKey: .viewToken)
        round = try? c.decode(Int.self, forKey: .round)
        feedback = try? c.decode([GateFeedbackEntry].self, forKey: .feedback)
        editableText = try? c.decode(String.self, forKey: .editableText)
        editedText = try? c.decode(String.self, forKey: .editedText)
        diagnostics = try? c.decode(String.self, forKey: .diagnostics)
    }

    /// Waits for a signed-out connection rather than a person's answer.
    public var isSignInWait: Bool { status == .waiting && waitingFor == "signIn" }

    /// A step that never ran, so it has nothing of its own to report (workflow-graph neverRan).
    public var neverRan: Bool {
        error?.hasPrefix("Skipped:") == true || error == "Run abandoned (no session id recorded)"
    }
}

public struct TriggerSession: Sendable, Hashable, Codable {
    public var id: String
    public var label: String
    public var restore: String
}

public struct ConnectorItemContext: Sendable, Hashable, Codable {
    public var connectionId: String
    public var connectorId: String
    public var externalId: String
    public var externalUrl: String?
    public var title: String
    public var body: String?

    public init(connectionId: String, connectorId: String, externalId: String, externalUrl: String? = nil, title: String) {
        self.connectionId = connectionId
        self.connectorId = connectorId
        self.externalId = externalId
        self.externalUrl = externalUrl
        self.title = title
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        connectionId = (try? c.decode(String.self, forKey: .connectionId)) ?? ""
        connectorId = (try? c.decode(String.self, forKey: .connectorId)) ?? ""
        externalId = (try? c.decode(String.self, forKey: .externalId)) ?? ""
        externalUrl = try? c.decode(String.self, forKey: .externalUrl)
        title = (try? c.decode(String.self, forKey: .title)) ?? ""
        body = try? c.decode(String.self, forKey: .body)
    }
}

public struct WorkflowExecution: Sendable, Hashable, Codable, Identifiable {
    public var runId: String
    public var workflowId: String
    public var startedAt: String
    public var completedAt: String?
    public var status: RunStatus
    public var nodeStates: [NodeExecutionState]
    public var triggerTaskId: String?
    public var triggerSession: TriggerSession?
    public var inputs: [String: JSONValue]?
    public var connectorItem: ConnectorItemContext?
    public var partial: Bool?
    public var retryOfRunId: String?
    public var definition: WorkflowDefinition?
    /// Set by `workflowRun:listAll`, which joins the workflow's name.
    public var workflowName: String?

    public init(
        runId: String, workflowId: String, startedAt: String, completedAt: String? = nil,
        status: RunStatus, nodeStates: [NodeExecutionState] = [], workflowName: String? = nil
    ) {
        self.runId = runId
        self.workflowId = workflowId
        self.startedAt = startedAt
        self.completedAt = completedAt
        self.status = status
        self.nodeStates = nodeStates
        self.workflowName = workflowName
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        workflowId = (try? c.decode(String.self, forKey: .workflowId)) ?? ""
        startedAt = (try? c.decode(String.self, forKey: .startedAt)) ?? ""
        let rawId = (try? c.decode(String.self, forKey: .runId)) ?? ""
        runId = rawId.isEmpty ? "\(workflowId):\(startedAt)" : rawId
        completedAt = try? c.decode(String.self, forKey: .completedAt)
        status = (try? c.decode(RunStatus.self, forKey: .status)) ?? .error
        nodeStates = (try? c.decode([NodeExecutionState].self, forKey: .nodeStates)) ?? []
        triggerTaskId = try? c.decode(String.self, forKey: .triggerTaskId)
        triggerSession = try? c.decode(TriggerSession.self, forKey: .triggerSession)
        inputs = try? c.decode([String: JSONValue].self, forKey: .inputs)
        connectorItem = try? c.decode(ConnectorItemContext.self, forKey: .connectorItem)
        partial = try? c.decode(Bool.self, forKey: .partial)
        retryOfRunId = try? c.decode(String.self, forKey: .retryOfRunId)
        definition = try? c.decode(WorkflowDefinition.self, forKey: .definition)
        workflowName = try? c.decode(String.self, forKey: .workflowName)
    }

    public var id: String { runId }

    /// The step that failed on its own terms (workflow-graph failedStep).
    public var failedStep: NodeExecutionState? {
        nodeStates.first { $0.status == .error && !$0.neverRan }
    }

    public var waitingStep: NodeExecutionState? { nodeStates.first { $0.status == .waiting } }

    public func state(of nodeId: String) -> NodeExecutionState? { nodeStates.first { $0.nodeId == nodeId } }
}

public struct GateResolution: Sendable, Hashable, Codable {
    public var accepted: Bool
    public var reason: String?

    public init(accepted: Bool, reason: String? = nil) {
        self.accepted = accepted
        self.reason = reason
    }
}
