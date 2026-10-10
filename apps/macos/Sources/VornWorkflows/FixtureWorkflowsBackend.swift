import Foundation
import VornCore

/// An in-memory backend over captured vornd JSON, for previews and tests.
public actor FixtureWorkflowsBackend: WorkflowsBackend {
    public private(set) var workflows: [WorkflowDefinition]
    public private(set) var runs: [WorkflowExecution]
    /// Calls that would have written, in order, for tests to assert on.
    public private(set) var writes: [String] = []

    public init(workflows: [WorkflowDefinition], runs: [WorkflowExecution]) {
        self.workflows = workflows
        self.runs = runs
    }

    public init(workflowsJSON: Data, runsJSON: Data) throws {
        let decoder = JSONDecoder()
        self.init(
            workflows: try decoder.decode([WorkflowDefinition].self, from: workflowsJSON),
            runs: try decoder.decode([WorkflowExecution].self, from: runsJSON))
    }

    public func listWorkflows() async throws -> [WorkflowDefinition] { workflows }

    public func listAllRuns(workspaceId: String, limit: Int) async throws -> [WorkflowExecution] {
        let ids = Set(workflows.filter { $0.workspace == workspaceId }.map(\.id))
        return Array(runs.filter { ids.contains($0.workflowId) }.sorted { $0.startedAt > $1.startedAt }.prefix(limit))
    }

    public func listRuns(workflowId: String, limit: Int) async throws -> [WorkflowExecution] {
        Array(runs.filter { $0.workflowId == workflowId }.sorted { $0.startedAt > $1.startedAt }.prefix(limit))
    }

    public func setEnabled(workflowId: String, enabled: Bool) async throws {
        writes.append("setEnabled \(workflowId) \(enabled)")
        if let i = workflows.firstIndex(where: { $0.id == workflowId }) { workflows[i].enabled = enabled }
    }

    public func deleteWorkflow(id: String) async throws {
        writes.append("delete \(id)")
        workflows.removeAll { $0.id == id }
    }

    public func run(workflowId: String, inputs: [String: JSONValue]?) async throws -> WorkflowExecution? {
        writes.append("run \(workflowId)")
        return nil
    }

    public func retry(runId: String) async throws -> WorkflowExecution? {
        writes.append("retry \(runId)")
        return nil
    }

    public func rerun(runId: String) async throws -> WorkflowExecution? {
        writes.append("rerun \(runId)")
        return nil
    }

    public func stop(runId: String) async throws { writes.append("stop \(runId)") }

    public func resolveGate(
        runId: String, nodeId: String, decision: GateDecision, comment: String?, edited: String?
    ) async throws -> GateResolution {
        writes.append("resolveGate \(runId) \(nodeId) \(decision.rawValue)")
        return GateResolution(accepted: true)
    }

    public func events() async throws -> AsyncStream<WorkflowEvent> {
        AsyncStream { _ in }
    }
}
