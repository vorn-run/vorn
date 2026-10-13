import Foundation
import Observation
import VornCore

/// What the step forms read from vornd: connections, connectors, agents, git state and tasks.
/// One per open panel; previews and tests fill it directly.
@MainActor @Observable
public final class NodeFormsData {
    public var connections: [FormConnection] = []
    public var connectors: [ConnectorInfo] = []
    public var connectorKeys: [ConnectorKeyInfo] = []
    /// Install state by agent id; an agent missing here counts as installed until asked.
    public var installed: [String: Bool] = [:]
    public var defaults = FormDefaults()
    public var webhookBaseURL: String?
    /// Actions by connection id; a connection that would not answer has an empty list.
    public var actions: [String: [ConnectorActionDef]] = [:]
    /// Catalog actions by connector id, for steps whose connector is not installed yet.
    public var catalogActions: [String: [ConnectorActionDef]] = [:]
    public var gitRepo: [String: Bool] = [:]
    public var worktrees: [String: [Worktree]] = [:]
    public var todoTasks: [String: [FormTask]] = [:]
    /// Model lists by `agent|projectPath`.
    public var models: [String: AgentModelCatalog] = [:]
    public var loadingModels: Set<String> = []

    @ObservationIgnored private var backend: (any WorkflowsBackend)?
    @ObservationIgnored private var asked: Set<String> = []

    public init(backend: (any WorkflowsBackend)? = nil) {
        self.backend = backend
    }

    /// Asks for everything that does not depend on the step; each call fails alone.
    public func load() async {
        guard let backend else { return }
        async let conns = try? backend.listConnections()
        async let connectors = try? backend.listConnectors()
        async let keys = try? backend.listConnectorKeys()
        async let installed = try? backend.detectInstalledAgents()
        async let defaults = try? backend.formDefaults()
        async let base = try? backend.webhookBaseURL()
        if let v = await conns { connections = v }
        if let v = await connectors { self.connectors = v }
        if let v = await keys { connectorKeys = v }
        if let v = await installed { self.installed = v }
        if let v = await defaults { self.defaults = v }
        if let v = await base { webhookBaseURL = v }
    }

    func isInstalled(_ agent: String) -> Bool { installed[agent] ?? true }

    private func once(_ key: String) -> (any WorkflowsBackend)? {
        guard let backend, !asked.contains(key) else { return nil }
        asked.insert(key)
        return backend
    }

    /// Fetches a connection's actions once.
    func ensureActions(_ connectionId: String) {
        guard !connectionId.isEmpty, let backend = once("actions|\(connectionId)") else { return }
        Task {
            actions[connectionId] = (try? await backend.listConnectionActions(connectionId)) ?? []
        }
    }

    func ensureCatalog() {
        guard let backend = once("catalog") else { return }
        Task {
            if let v = try? await backend.connectorCatalogActions() { catalogActions = v }
        }
    }

    /// Whether a project is a git repo and its worktrees, once per path.
    func ensureGit(_ path: String) {
        guard !path.isEmpty, let backend = once("git|\(path)") else { return }
        Task {
            gitRepo[path] = (try? await backend.isGitRepo(path)) ?? false
            worktrees[path] = ((try? await backend.listWorktrees(path)) ?? []).filter { !$0.isMain }
        }
    }

    func ensureTasks(_ projectName: String) {
        guard !projectName.isEmpty, let backend = once("tasks|\(projectName)") else { return }
        Task {
            todoTasks[projectName] = (try? await backend.listTodoTasks(projectName: projectName)) ?? []
        }
    }

    static func modelKey(_ agent: String, _ path: String) -> String { "\(agent)|\(path)" }

    /// useAgentModelCatalog: the agent's models for a project; `refresh` asks the agent again.
    func ensureModels(agent: String, projectPath: String, refresh: Bool = false) {
        let key = Self.modelKey(agent, projectPath)
        guard let backend else { return }
        if !refresh && (models[key] != nil || loadingModels.contains(key)) { return }
        loadingModels.insert(key)
        Task {
            let catalog = try? await backend.listModels(agentType: agent, projectPath: projectPath, refresh: refresh)
            models[key] = catalog
                ?? AgentModelCatalog(choices: [], status: "unavailable", error: "Could not list models — type an id")
            loadingModels.remove(key)
        }
    }

    /// Re-reads one connection's actions, after the user picked a different one.
    func connection(_ id: String?) -> FormConnection? {
        guard let id, !id.isEmpty else { return nil }
        return connections.first { $0.id == id }
    }
}

/// The trigger-side facts each form reads (what WorkflowEditor.tsx derives for the panel).
public struct StepContext {
    public var triggerType: String?
    public var isContextualTrigger: Bool
    public var inputVars: [TemplateVariable]
    public var stepGroups: [StepVariableGroup]
    public var ancestors: [WorkflowNode]

    public init(
        triggerType: String? = nil, isContextualTrigger: Bool = false, inputVars: [TemplateVariable] = [],
        stepGroups: [StepVariableGroup] = [], ancestors: [WorkflowNode] = []
    ) {
        self.triggerType = triggerType
        self.isContextualTrigger = isContextualTrigger
        self.inputVars = inputVars
        self.stepGroups = stepGroups
        self.ancestors = ancestors
    }

    var isTaskTrigger: Bool { triggerType == "taskCreated" || triggerType == "taskStatusChanged" }

    /// WorkflowEditor.tsx: the derivations for the selected step.
    public static func derive(
        node: WorkflowNode, nodes: [WorkflowNode], edges: [WorkflowEdge], latestRun: WorkflowExecution?,
        lookupAction: ((String, String) -> ConnectorActionDef?)?
    ) -> StepContext {
        let trigger = nodes.first { $0.type == .trigger }?.config
        let triggerType = trigger?.str("triggerType")
        let contextual = triggerType == "manual" && trigger?.bool("contextual") == true
        let inputs =
            triggerType == "manual"
            ? (trigger?["inputs"]?.arrayValue ?? []).compactMap(WorkflowInputDef.init) : []
        let inputVars = TemplateVars.inputVars(inputs) + TemplateVars.loopVars(nodes: nodes, nodeId: node.id)
        let ancestors = TemplateVars.ancestorNodes(nodes: nodes, edges: edges, of: node.id)
        var visible = ancestors
        if node.type == .loop {
            let body = Set(TemplateVars.loopBodyIds(node))
            visible += nodes.filter { body.contains($0.id) }
        }
        let groups = TemplateVars.stepGroups(
            visible, lookupAction: lookupAction, lastRun: TemplateVars.lastRunData(latestRun, nodes: nodes))
        return StepContext(
            triggerType: triggerType, isContextualTrigger: contextual, inputVars: inputVars, stepGroups: groups,
            ancestors: ancestors)
    }
}

/// workflow-helpers.ts pieces the forms need.
public enum FormHelpers {
    /// REPLACEABLE_NODE_TYPES: steps the library can swap in place.
    public static let replaceable: Set<WorkflowNodeType> = [
        .launchAgent, .script, .approval, .createTaskFromItem, .callConnectorAction, .httpRequest,
    ]

    /// switchTriggerType: the fresh config a trigger type starts from.
    public static func switchTriggerType(_ type: String, now: Date = Date(), token: String = UUID().uuidString.lowercased())
        -> JSONValue
    {
        var c: JSONValue = ["triggerType": .string(type)]
        switch type {
        case "once": c.set("runAt", string: isoString(now))
        case "recurring": c.set("cron", string: "0 9 * * *")
        case "connectorPoll":
            c.set("connectionId", string: "")
            c.set("event", string: "")
            c.set("cron", string: "*/5 * * * *")
        case "webhook":
            c.set("method", string: "POST")
            c.set("token", string: token)
        default: break
        }
        return c
    }

    /// `Date.toISOString()`: UTC with milliseconds.
    static func isoString(_ date: Date) -> String {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f.string(from: date)
    }

    static func parseISO(_ s: String) -> Date? {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let d = f.date(from: s) { return d }
        f.formatOptions = [.withInternetDateTime]
        return f.date(from: s)
    }

    /// getWorktreeMode: fromContext, the stored mode, `new` for the legacy flag, else none.
    public static func worktreeMode(_ c: JSONValue) -> String {
        if c.str("useWorktree") == "fromContext" { return "fromContext" }
        if let m = c.str("worktreeMode") { return m }
        if c.bool("useWorktree") == true { return "new" }
        return "none"
    }

    /// getProjectRemoteHostId: the first host that is not this machine.
    public static func remoteHostId(_ project: Project?) -> String? {
        project?.hostIds?.first { $0 != "local" }
    }
}
