import Foundation
import VornCore

/// What the Workflows view reads and writes; vornd in the app, fixtures in previews and tests.
public protocol WorkflowsBackend: Sendable {
    func listWorkflows() async throws -> [WorkflowDefinition]
    func listAllRuns(workspaceId: String, limit: Int) async throws -> [WorkflowExecution]
    func listRuns(workflowId: String, limit: Int) async throws -> [WorkflowExecution]
    func setEnabled(workflowId: String, enabled: Bool) async throws
    func deleteWorkflow(id: String) async throws
    func run(workflowId: String, inputs: [String: JSONValue]?) async throws -> WorkflowExecution?
    func retry(runId: String) async throws -> WorkflowExecution?
    func rerun(runId: String) async throws -> WorkflowExecution?
    func stop(runId: String) async throws
    func resolveGate(runId: String, nodeId: String, decision: GateDecision, comment: String?, edited: String?)
        async throws
        -> GateResolution
    /// Pushes until the caller stops listening.
    func events() async throws -> AsyncStream<WorkflowEvent>
}

/// The backend over a vornd connection.
public struct VorndWorkflowsBackend: WorkflowsBackend {
    public let client: VorndClient

    public init(client: VorndClient) {
        self.client = client
    }

    /// Connects to the vornd whose files are in `dataDirectory`.
    public static func connect(
        dataDirectory: URL = VorndEndpoint.defaultDataDirectory, access: VorndClient.Access = .readWrite
    ) async throws -> VorndWorkflowsBackend {
        let client = VorndClient(endpoint: try VorndEndpoint.discover(dataDirectory: dataDirectory), access: access)
        try await client.connect()
        return VorndWorkflowsBackend(client: client)
    }

    public func listWorkflows() async throws -> [WorkflowDefinition] { try await client.listWorkflows() }

    public func listAllRuns(workspaceId: String, limit: Int) async throws -> [WorkflowExecution] {
        try await client.listAllRuns(workspaceId: workspaceId, limit: limit)
    }

    public func listRuns(workflowId: String, limit: Int) async throws -> [WorkflowExecution] {
        try await client.listRuns(workflowId: workflowId, limit: limit)
    }

    public func setEnabled(workflowId: String, enabled: Bool) async throws {
        try await client.setWorkflowEnabled(id: workflowId, enabled: enabled)
    }

    public func deleteWorkflow(id: String) async throws { try await client.deleteWorkflow(id: id) }

    public func run(workflowId: String, inputs: [String: JSONValue]?) async throws -> WorkflowExecution? {
        guard let inputs, !inputs.isEmpty else { return try await client.runWorkflow(id: workflowId) }
        let params: JSONValue = .object(["workflowId": .string(workflowId), "context": .object(["inputs": .object(inputs)])])
        return try await client.call("workflow:run", params, as: WorkflowExecution?.self)
    }

    public func retry(runId: String) async throws -> WorkflowExecution? { try await client.retryRun(runId: runId) }

    public func rerun(runId: String) async throws -> WorkflowExecution? { try await client.rerun(runId: runId) }

    public func stop(runId: String) async throws { try await client.stopRun(runId: runId) }

    public func resolveGate(
        runId: String, nodeId: String, decision: GateDecision, comment: String?, edited: String?
    ) async throws -> GateResolution {
        try await client.resolveGate(
            runId: runId, nodeId: nodeId, decision: decision, comment: comment, edited: edited)
    }

    public func events() async throws -> AsyncStream<WorkflowEvent> {
        let raw = await client.events()
        try await client.subscribe(topics: WorkflowEvent.topics)
        return AsyncStream { continuation in
            let task = Task {
                for await event in raw {
                    if let e = WorkflowEvent(event) { continuation.yield(e) }
                }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}
