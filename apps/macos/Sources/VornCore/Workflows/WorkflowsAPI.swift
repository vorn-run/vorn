import Foundation

/// vornd's workflow methods (protocol.ts `workflow:*`, `workflowRun:*`).
extension VorndClient {
    public func listWorkflows() async throws -> [WorkflowDefinition] {
        try await call("workflow:list", nil, as: [WorkflowDefinition].self)
    }

    public func workflow(id: String) async throws -> WorkflowDefinition? {
        try await call("workflow:get", .object(["id": .string(id)]), as: WorkflowDefinition?.self)
    }

    public func createWorkflow(_ workflow: WorkflowDefinition) async throws {
        try await call("workflow:create", .object(["workflow": try .from(workflow)]))
    }

    public func updateWorkflow(id: String, updates: WorkflowDefinition) async throws {
        try await call("workflow:update", .object(["id": .string(id), "updates": try .from(updates)]))
    }

    public func deleteWorkflow(id: String) async throws {
        try await call("workflow:delete", .object(["id": .string(id)]))
    }

    public func setWorkflowEnabled(id: String, enabled: Bool) async throws {
        try await call("workflow:setEnabled", .object(["id": .string(id), "enabled": .bool(enabled)]))
    }

    /// Starts a run now; nil when an identical run is already in flight.
    public func runWorkflow(id: String, targetNodeId: String? = nil) async throws -> WorkflowExecution? {
        var params: [String: JSONValue] = ["workflowId": .string(id)]
        if let targetNodeId { params["targetNodeId"] = .string(targetNodeId) }
        return try await call("workflow:run", .object(params), as: WorkflowExecution?.self)
    }

    /// Resumes a failed run from its failed step, reusing what already succeeded.
    public func retryRun(runId: String) async throws -> WorkflowExecution? {
        try await call("workflow:retryRun", .object(["runId": .string(runId)]), as: WorkflowExecution?.self)
    }

    /// Runs the same workflow again with the same trigger context.
    public func rerun(runId: String) async throws -> WorkflowExecution? {
        try await call("workflow:rerun", .object(["runId": .string(runId)]), as: WorkflowExecution?.self)
    }

    public func stopRun(runId: String) async throws {
        try await call("workflow:stopRun", .object(["runId": .string(runId)]))
    }

    public func resolveGate(
        runId: String, nodeId: String, decision: GateDecision, comment: String? = nil, edited: String? = nil
    ) async throws -> GateResolution {
        var params: [String: JSONValue] = [
            "runId": .string(runId), "nodeId": .string(nodeId), "decision": .string(decision.rawValue),
        ]
        if let comment { params["comment"] = .string(comment) }
        if let edited { params["edited"] = .string(edited) }
        return try await call("workflow:resolveGate", .object(params), as: GateResolution.self)
    }

    /// Every workflow's runs, newest first, each with its workflow's name.
    public func listAllRuns(workspaceId: String? = nil, limit: Int? = nil) async throws -> [WorkflowExecution] {
        var params: [String: JSONValue] = [:]
        if let workspaceId { params["workspaceId"] = .string(workspaceId) }
        if let limit { params["limit"] = .number(Double(limit)) }
        return try await call("workflowRun:listAll", .object(params), as: [WorkflowExecution].self)
    }

    public func listRuns(workflowId: String, limit: Int? = nil) async throws -> [WorkflowExecution] {
        var params: [String: JSONValue] = ["workflowId": .string(workflowId)]
        if let limit { params["limit"] = .number(Double(limit)) }
        return try await call("workflowRun:list", .object(params), as: [WorkflowExecution].self)
    }
}

/// Pushes about workflows, decoded from a `VorndEvent`.
public enum WorkflowEvent: Sendable {
    case runUpdated(WorkflowExecution)
    case runCompleted(WorkflowExecution)
    case gateResolved(runId: String, nodeId: String)
    /// The config changed; carries its workflows when the push included them.
    case workflowsChanged([WorkflowDefinition]?)
    case disconnected(String)

    public static let topics = ["workflow:*", "config:changed"]

    public init?(_ event: VorndEvent) {
        switch event {
        case .disconnected(let why):
            self = .disconnected(why)
        case .notification(let method, let params):
            switch method {
            case "workflow:runUpdated":
                guard let run = try? params.decode(WorkflowExecution.self) else { return nil }
                self = .runUpdated(run)
            case "workflow:executionComplete":
                guard let run = try? params.decode(WorkflowExecution.self) else { return nil }
                self = .runCompleted(run)
            case "workflow:gateResolved":
                self = .gateResolved(
                    runId: params["runId"]?.stringValue ?? "", nodeId: params["nodeId"]?.stringValue ?? "")
            case "config:changed":
                self = .workflowsChanged(try? params["workflows"]?.decode([WorkflowDefinition].self))
            default:
                return nil
            }
        }
    }
}
