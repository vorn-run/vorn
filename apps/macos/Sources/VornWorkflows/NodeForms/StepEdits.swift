import Foundation
import VornCore

/// The writes each step form makes, as pure functions over the config; keys and absences match the TSX forms.
public enum StepEdits {
    typealias Ref = TemplateVars.ContextRef

    // MARK: Shared

    /// ConditionConfigForm and CallConnectorActionNodeForm's own context filter.
    public static func plainContextVars(triggerType: String?, connectorItem: Bool, inputVars: [TemplateVariable])
        -> [TemplateVariable]
    {
        TemplateVars.all.filter { v in
            switch v.category {
            case .task: return triggerType == "taskCreated" || triggerType == "taskStatusChanged"
            case .connectorItem: return connectorItem && triggerType == "connectorPoll"
            case .trigger:
                if triggerType == "webhook" {
                    return v.key.contains("trigger.body") || v.key.contains("trigger.headers") || v.key.contains("trigger.query")
                }
                return triggerType == "taskStatusChanged" && v.key.contains("Status")
            default: return false
            }
        } + inputVars
    }

    /// LaunchAgent, Script and HTTP: getAvailableContextVars plus the inputs.
    public static func agentContextVars(_ step: StepContext) -> [TemplateVariable] {
        TemplateVars.availableContextVars(triggerType: step.triggerType, contextual: step.isContextualTrigger) + step.inputVars
    }

    public static func hasTemplateVars(_ step: StepContext) -> Bool {
        !step.stepGroups.isEmpty || step.isTaskTrigger || step.isContextualTrigger || !step.inputVars.isEmpty
    }

    /// `Number(raw)`: blank is 0, junk is nil.
    static func jsNumber(_ raw: String) -> Double? {
        let t = raw.trimmingCharacters(in: .whitespaces)
        if t.isEmpty { return 0 }
        guard let d = Double(t), d.isFinite else { return nil }
        return d
    }

    /// `Math.round`: halves go up.
    static func jsRound(_ d: Double) -> Double { (d + 0.5).rounded(.down) }

    // MARK: Script

    public static func scriptCwdFromContext(_ c: JSONValue) -> Bool {
        TemplateVars.isContextRef(c.str("cwd")) || TemplateVars.isContextRef(c.str("projectName"))
            || TemplateVars.isContextRef(c.str("projectPath"))
    }

    /// Working Directory: None drops all three keys; a project writes its name and path as both path and cwd.
    public static func scriptProject(_ c: JSONValue, _ project: Project?) -> JSONValue {
        guard let project else { return c.merged(["projectName": nil, "projectPath": nil, "cwd": nil]) }
        return c.merged([
            "projectName": .string(project.name), "projectPath": .string(project.path), "cwd": .string(project.path),
        ])
    }

    public static func scriptFromContext(_ c: JSONValue) -> JSONValue {
        c.merged(["projectName": .string(Ref.projectName), "projectPath": .string(Ref.projectPath), "cwd": .string(Ref.cwd)])
    }

    /// The trigger stopped being contextual: drop the sentinels rather than run with an empty cwd.
    public static func scriptClearContext(_ c: JSONValue, contextual: Bool) -> JSONValue? {
        guard !contextual, scriptCwdFromContext(c) else { return nil }
        return c.merged(["cwd": nil, "projectName": nil, "projectPath": nil])
    }

    public static func emptyIsAbsent(_ c: JSONValue, _ key: String, _ v: String) -> JSONValue {
        c.merged([key: v.isEmpty ? nil : .string(v)])
    }

    /// Script arguments: split on single spaces, empties dropped; an empty list is still written.
    public static func scriptArgs(_ c: JSONValue, _ text: String) -> JSONValue {
        let parts = text.split(separator: " ", omittingEmptySubsequences: true).map { JSONValue.string(String($0)) }
        return c.merged(["args": .array(parts)])
    }

    // MARK: HTTP

    /// `Object.fromEntries`: a repeated name keeps its first place and its last value.
    public static func headerEntries(_ entries: [(String, String)]) -> [(String, String)] {
        var out: [(String, String)] = []
        for (k, v) in entries {
            if let i = out.firstIndex(where: { $0.0 == k }) { out[i].1 = v } else { out.append((k, v)) }
        }
        return out
    }

    public static func httpHeaders(_ c: JSONValue, _ entries: [(String, String)]) -> JSONValue {
        var o: [String: JSONValue] = [:]
        for (k, v) in headerEntries(entries) { o[k] = .string(v) }
        return c.merged(["headers": .object(o)])
    }

    public static let newProfile = "__new_profile__"

    // MARK: Loop

    public static let maxIterations = 10

    public static func loopMax(_ c: JSONValue, _ raw: String) -> JSONValue {
        let n = jsNumber(raw)
        let clamped = n.map { min(max(1, $0.rounded(.down)), Double(maxIterations)) } ?? 1
        return c.merged(["maxIterations": .number(clamped)])
    }

    /// Any of the three Stop early fields writes the whole `until`.
    public static func loopUntil(_ c: JSONValue, variable: String? = nil, op: String? = nil, value: String? = nil) -> JSONValue {
        let u = c["until"]
        return c.merged([
            "until": [
                "variable": .string(variable ?? u?.str("variable") ?? ""),
                "operator": .string(op ?? u?.str("operator") ?? "equals"),
                "value": .string(value ?? u?.str("value") ?? ""),
            ]
        ])
    }

    // MARK: Approval

    public static let defaultGateRounds: Double = 3
    public static let maxGateRounds: Double = 10

    public static func approvalSteps(_ ancestors: [WorkflowNode]) -> [WorkflowNode] {
        ancestors.filter { $0.type != .trigger }
    }

    public static func approvalFeedback(_ c: JSONValue, steps: [WorkflowNode], from: String? = nil, rounds: Double? = nil)
        -> JSONValue
    {
        let f = c["feedback"]
        return c.merged([
            "feedback": [
                "from": .string(from ?? f?.str("from") ?? steps.first?.id ?? ""),
                "maxRounds": .number(rounds ?? f?.num("maxRounds") ?? defaultGateRounds),
            ]
        ])
    }

    public static func approvalRounds(_ c: JSONValue, steps: [WorkflowNode], _ raw: String) -> JSONValue? {
        guard let n = jsNumber(raw) else { return nil }
        return approvalFeedback(c, steps: steps, rounds: min(max(n.rounded(.down), 2), maxGateRounds))
    }

    /// Seconds; blank, zero or junk waits forever.
    public static func approvalTimeout(_ c: JSONValue, _ raw: String) -> JSONValue {
        let t = raw.trimmingCharacters(in: .whitespaces)
        let secs = Double(t)
        let valid = !t.isEmpty && secs != nil && secs!.isFinite && secs! > 0
        return c.merged(["timeoutMs": valid ? .number(jsRound(secs! * 1000)) : nil])
    }

    public static func approvalTimeoutText(_ c: JSONValue) -> String {
        guard let ms = c.num("timeoutMs"), ms > 0 else { return "" }
        return String(Int(jsRound(ms / 1000)))
    }

    // MARK: Connector action

    public static func connectorConnection(_ c: JSONValue, _ id: String) -> JSONValue {
        c.merged(["connectionId": .string(id), "action": .string(""), "args": .object([:])])
    }

    /// The label is kept beside the action so a card can name it without asking.
    public static func connectorAction(_ c: JSONValue, _ type: String, actions: [ConnectorActionDef]) -> JSONValue {
        c.merged([
            "action": .string(type), "actionLabel": .string(actions.first { $0.type == type }?.label ?? type),
            "args": .object([:]),
        ])
    }

    public static func connectorArg(_ c: JSONValue, _ key: String, _ value: String) -> JSONValue {
        var args = c["args"]?.objectValue ?? [:]
        args[key] = .string(value)
        return c.merged(["args": .object(args)])
    }

    // MARK: Launch agent

    public static func agentPromptSource(_ c: JSONValue) -> String {
        if let t = c.str("taskId"), !t.isEmpty { return "task" }
        return c.bool("taskFromQueue") == true ? "queue" : "inline"
    }

    public static func agentSetAgent(_ c: JSONValue, _ agent: String) -> JSONValue {
        c.merged(["agentType": .string(agent), "model": nil])
    }

    public static func agentProject(_ c: JSONValue, _ p: Project) -> JSONValue {
        c.merged(["projectName": .string(p.name), "projectPath": .string(p.path)])
    }

    public static func agentProjectFromContext(_ c: JSONValue) -> JSONValue {
        c.merged(["projectName": .string(Ref.projectName), "projectPath": .string(Ref.projectPath)])
    }

    public static func agentPromptSource(_ c: JSONValue, _ v: String, firstTaskId: String?) -> JSONValue {
        switch v {
        case "inline": return c.merged(["taskId": nil, "taskFromQueue": nil])
        case "task": return c.merged(["prompt": nil, "taskFromQueue": nil, "taskId": firstTaskId.map(JSONValue.string)])
        default: return c.merged(["prompt": nil, "taskId": nil, "taskFromQueue": .bool(true)])
        }
    }

    /// Clearing the branch also drops the worktree flag.
    public static func agentBranch(_ c: JSONValue, _ text: String) -> JSONValue {
        text.isEmpty ? c.merged(["branch": nil, "useWorktree": nil]) : c.merged(["branch": .string(text)])
    }

    public static func agentToggleBranchContext(_ c: JSONValue) -> JSONValue {
        c.merged(["branch": TemplateVars.isContextRef(c.str("branch")) ? nil : .string(Ref.branch)])
    }

    public static func agentIsRemote(_ project: Project?) -> Bool { FormHelpers.remoteHostId(project) != nil }

    public static func agentHasBranch(_ c: JSONValue, remote: Bool) -> Bool {
        !remote && !(c.str("branch") ?? "").trimmingCharacters(in: .whitespaces).isEmpty
    }

    /// Earlier agent steps that make or reuse a worktree, the ones From step can point at.
    public static func priorWorktreeSteps(_ nodes: [WorkflowNode], current: String) -> [WorkflowNode] {
        nodes.filter { n in
            guard n.id != current, n.type == .launchAgent else { return false }
            let mode = FormHelpers.worktreeMode(n.config)
            return mode == "new" || mode == "fromStep"
        }
    }

    /// The Worktree select; nil when the choice is refused (New without a branch, From step without steps).
    public static func agentWorktreeMode(_ c: JSONValue, _ key: String, hasBranch: Bool, priorSteps: Int) -> JSONValue? {
        if key == "new" && !hasBranch { return nil }
        if key == "fromStep" && priorSteps == 0 { return nil }
        if key == "fromContext" {
            return c.merged([
                "useWorktree": .string("fromContext"), "worktreeMode": nil, "worktreeFromStepSlug": nil,
                "existingWorktreePath": nil,
            ])
        }
        var updates: [String: JSONValue?] = [
            "worktreeMode": .string(key), "useWorktree": key == "new" ? .bool(true) : nil, "worktreeFromStepSlug": nil,
            "existingWorktreePath": nil,
        ]
        if key == "none" {
            updates["branch"] = .some(nil)
            updates["useWorktree"] = .some(nil)
        }
        return c.merged(updates)
    }

    public static func agentToggleHeadless(_ c: JSONValue) -> JSONValue {
        c.merged(["headless": .bool(c.bool("headless") == true ? false : true)])
    }

    /// Timeout in minutes: blank removes it, junk or negative is ignored, the rest is whole minutes in ms.
    public static func agentTimeout(_ c: JSONValue, _ raw: String) -> JSONValue? {
        let t = raw.trimmingCharacters(in: .whitespaces)
        if t.isEmpty { return c.merged(["timeoutMs": nil]) }
        guard let m = Double(t), m.isFinite, m >= 0 else { return nil }
        return c.merged(["timeoutMs": .number(jsRound(m) * 60_000)])
    }

    public static func agentTimeoutText(_ c: JSONValue) -> String {
        guard let ms = c.num("timeoutMs") else { return "" }
        return String(Int(jsRound(ms / 60_000)))
    }

    /// Extra Arguments: whitespace-split, absent when blank.
    public static func agentArgs(_ c: JSONValue, _ text: String) -> JSONValue {
        c.merged(["args": splitArgs(text.trimmingCharacters(in: .whitespaces)).map { .array($0.map(JSONValue.string)) }])
    }

    /// OutputSchemaField: an object is committed, blank clears it, anything else is an error and leaves the config.
    public enum SchemaParse: Equatable {
        case set(JSONValue?)
        case error(String)
    }

    public static func parseSchema(_ raw: String) -> SchemaParse {
        let t = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if t.isEmpty { return .set(nil) }
        guard let v = JSONValue.parse(t) else { return .error("Not valid JSON") }
        guard v.objectValue != nil else { return .error("Schema must be a JSON object") }
        return .set(v)
    }

    /// The trigger stopped being contextual: From Context project, branch and worktree are reset.
    public static func agentClearContext(_ c: JSONValue, contextual: Bool) -> JSONValue? {
        guard !contextual else { return nil }
        let project = TemplateVars.isContextRef(c.str("projectName")) || TemplateVars.isContextRef(c.str("projectPath"))
        let branch = TemplateVars.isContextRef(c.str("branch"))
        let worktree = c.str("useWorktree") == "fromContext"
        guard project || branch || worktree else { return nil }
        var updates: [String: JSONValue?] = [:]
        if project {
            updates["projectName"] = .string("")
            updates["projectPath"] = .string("")
        }
        if branch { updates["branch"] = .some(nil) }
        if worktree {
            updates["useWorktree"] = .some(nil)
            updates["worktreeMode"] = .string("none")
        }
        return c.merged(updates)
    }

    /// From Task only makes sense where a task is in reach; otherwise fall back to the default agent.
    public static func agentFixFromTask(_ c: JSONValue, canUseFromTask: Bool, defaultAgent: String) -> JSONValue? {
        guard !canUseFromTask, c.str("agentType") == "fromTask" else { return nil }
        return c.merged(["agentType": .string(defaultAgent)])
    }

    /// agent-models.ts: the agents whose model can be chosen.
    public static func supportsModelSelection(_ agent: String) -> Bool {
        ["claude", "copilot", "opencode", "codex"].contains(agent)
    }

    /// validateModelId: one command-line argument.
    public static func validateModelId(_ value: String) -> String? {
        let m = value.trimmingCharacters(in: .whitespacesAndNewlines)
        if m.isEmpty || m.hasPrefix("-") || m.contains(where: { $0.isWhitespace })
            || m.unicodeScalars.contains(where: { $0.value < 32 || $0.value == 127 })
        {
            return nil
        }
        return m
    }
}
