import Foundation
import VornCore

/// A `{{…}}` variable a form offers in its picker (template-vars.ts TemplateVariable).
public struct TemplateVariable: Hashable, Sendable {
    public enum Category: String, CaseIterable, Sendable {
        // Display order of VariableAutocomplete.tsx VAR_CATEGORY_GROUPS.
        case task, trigger, context, inputs, loop, connectorItem
    }

    public var key: String
    public var label: String
    public var category: Category

    public init(_ key: String, _ label: String, _ category: Category) {
        self.key = key
        self.label = label
        self.category = category
    }
}

/// One output key a step group offers.
public struct StepVariableKey: Hashable, Sendable {
    public var key: String
    public var label: String
    public var description: String
    public var value: String?

    public init(key: String, label: String, description: String, value: String? = nil) {
        self.key = key
        self.label = label
        self.description = description
        self.value = value
    }
}

/// An upstream step's outputs in the picker (template-vars.ts StepVariableGroup).
public struct StepVariableGroup: Hashable, Sendable {
    public var nodeId: String
    public var label: String
    public var slug: String
    public var nodeType: WorkflowNodeType
    public var disabled = false
    public var keys: [StepVariableKey]
    public var connectionId: String?
    public var runStatus: NodeExecutionStatus?
    public var runCompletedAt: String?
    public var runOutputs: [String: JSONValue]?
}

/// What buildStepGroups reads from the workflow's most recent run.
public struct LastRunData: Sendable {
    public var outputs: [String: [String: JSONValue]]
    public var states: [String: (status: NodeExecutionStatus, completedAt: String?)]
}

/// The `steps.*` preview under a template field.
public struct TemplatePreview: Equatable, Sendable {
    public var resolved: String?
    public var broken: (token: String, suggestion: String?)?

    public static func == (a: Self, b: Self) -> Bool {
        a.resolved == b.resolved && a.broken?.token == b.broken?.token && a.broken?.suggestion == b.broken?.suggestion
    }
}

/// packages/shared/src/template-vars.ts and workflow-graph.ts buildStepOutputsMap.
public enum TemplateVars {
    public static func slugify(_ label: String) -> String {
        var out = ""
        var lastUnderscore = false
        for ch in label.lowercased() {
            if ch.isASCII && (ch.isLetter || ch.isNumber) {
                out.append(ch)
                lastUnderscore = false
            } else if !lastUnderscore {
                out.append("_")
                lastUnderscore = true
            }
        }
        while out.hasPrefix("_") { out.removeFirst() }
        while out.hasSuffix("_") { out.removeLast() }
        return out.isEmpty ? "step" : out
    }

    public static func ensureUniqueSlug(_ slug: String, existing: Set<String>) -> String {
        guard existing.contains(slug) else { return slug }
        var i = 2
        while existing.contains("\(slug)_\(i)") { i += 1 }
        return "\(slug)_\(i)"
    }

    static let defaultOutputKeys: [StepVariableKey] = [
        .init(key: "output", label: "Output", description: "Primary output (stdout / agent logs)"),
        .init(key: "status", label: "Status", description: "success or error"),
        .init(key: "error", label: "Error", description: "Error message if failed"),
    ]

    static let worktreeOutputKey = StepVariableKey(
        key: "worktreePath", label: "Worktree", description: "Directory the step worked in")

    public static let all: [TemplateVariable] = [
        .init("{{task.title}}", "Title", .task),
        .init("{{task.description}}", "Description", .task),
        .init("{{task.id}}", "ID", .task),
        .init("{{task.status}}", "Status", .task),
        .init("{{task.branch}}", "Branch", .task),
        .init("{{task.projectName}}", "Project", .task),
        .init("{{trigger.fromStatus}}", "Previous Status", .trigger),
        .init("{{trigger.toStatus}}", "New Status", .trigger),
        .init("{{trigger.restore}}", "Cold or Warm", .trigger),
        .init("{{trigger.body}}", "Request Body", .trigger),
        .init("{{trigger.headers}}", "Request Headers", .trigger),
        .init("{{trigger.query}}", "Query Parameters", .trigger),
        .init("{{connectorItem.externalId}}", "External ID", .connectorItem),
        .init("{{connectorItem.title}}", "Item Title", .connectorItem),
        .init("{{connectorItem.externalUrl}}", "Item URL", .connectorItem),
        .init("{{connectorItem.body}}", "Item Body", .connectorItem),
        .init("{{connectorItem.connectorId}}", "Connector", .connectorItem),
        .init("{{context.cwd}}", "Working Directory", .context),
        .init("{{context.projectPath}}", "Project Path", .context),
        .init("{{context.projectName}}", "Project Name", .context),
        .init("{{context.branch}}", "Branch", .context),
        .init("{{context.worktreePath}}", "Worktree Path", .context),
    ]

    /// CONTEXT_REF: the sentinels "From Context" writes into a string field.
    public enum ContextRef {
        public static let cwd = "{{context.cwd}}"
        public static let projectPath = "{{context.projectPath}}"
        public static let projectName = "{{context.projectName}}"
        public static let branch = "{{context.branch}}"
        public static let worktreePath = "{{context.worktreePath}}"
    }

    public static func isContextRef(_ value: String?) -> Bool {
        guard let value else { return false }
        return value.range(of: #"^\s*\{\{\s*context\.\w+\s*\}\}\s*$"#, options: .regularExpression) != nil
    }

    public static func containsContextRef(_ value: String?, field: String? = nil) -> Bool {
        guard let value else { return false }
        let pattern = field.map { #"\{\{\s*context\."# + $0 + #"\s*\}\}"# } ?? #"\{\{\s*context\.\w+\s*\}\}"#
        return value.range(of: pattern, options: .regularExpression) != nil
    }

    /// getAvailableContextVars: what the trigger puts in reach of LaunchAgent and Script steps.
    public static func availableContextVars(triggerType: String?, contextual: Bool) -> [TemplateVariable] {
        let isTaskTrigger = triggerType == "taskCreated" || triggerType == "taskStatusChanged"
        return all.filter { v in
            if isTaskTrigger && v.category == .task { return true }
            if isTaskTrigger && v.category == .trigger && triggerType == "taskStatusChanged" {
                return v.key.contains("Status")
            }
            if triggerType == "sessionRestored" {
                if v.category == .context { return true }
                if v.category == .trigger { return v.key.contains("trigger.restore") }
            }
            if triggerType == "webhook" && v.category == .trigger {
                return v.key.contains("trigger.body") || v.key.contains("trigger.headers")
                    || v.key.contains("trigger.query")
            }
            if contextual && v.category == .context { return true }
            return false
        }
    }

    /// buildLoopVars: `{{loop.*}}` for a step in a loop's body, or the loop itself.
    public static func loopVars(nodes: [WorkflowNode], nodeId: String?) -> [TemplateVariable] {
        guard let nodeId,
            let loop = nodes.first(where: { n in
                n.type == .loop && (n.id == nodeId || loopBodyIds(n).contains(nodeId))
            })
        else { return [] }
        let forEach = loop.config.str("mode") == "forEach"
        var vars: [TemplateVariable] = []
        if forEach { vars.append(.init("{{loop.item}}", "This item", .loop)) }
        vars.append(.init("{{loop.number}}", forEach ? "Item number, from 1" : "Pass number, from 1", .loop))
        vars.append(.init("{{loop.index}}", "Index, from 0", .loop))
        vars.append(.init("{{loop.count}}", forEach ? "How many items" : "Most passes", .loop))
        return vars
    }

    static func loopBodyIds(_ node: WorkflowNode) -> [String] {
        node.config["bodyNodeIds"]?.arrayValue?.compactMap(\.stringValue) ?? []
    }

    /// buildInputVars: `{{inputs.<key>}}` for each declared run input.
    public static func inputVars(_ inputs: [WorkflowRunInput]?) -> [TemplateVariable] {
        (inputs ?? []).filter { !$0.key.isEmpty }.map {
            .init("{{inputs.\($0.key)}}", $0.label.isEmpty ? $0.key : $0.label, .inputs)
        }
    }

    /// getAncestorNodes: every upstream step by BFS over edges, triggers left out.
    public static func ancestorNodes(nodes: [WorkflowNode], edges: [WorkflowEdge], of nodeId: String) -> [WorkflowNode] {
        let byId = Dictionary(nodes.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        var preds: [String: [String]] = [:]
        for e in edges { preds[e.target, default: []].append(e.source) }
        var out: [WorkflowNode] = []
        var visited: Set<String> = [nodeId]
        var queue = [nodeId]
        while !queue.isEmpty {
            let id = queue.removeFirst()
            for p in preds[id] ?? [] where !visited.contains(p) {
                visited.insert(p)
                queue.append(p)
                if let n = byId[p], n.type != .trigger { out.append(n) }
            }
        }
        return out
    }

    /// schemaTopLevelKeys: a JSON Schema's top-level properties, with description or type.
    static func schemaKeys(_ schema: JSONValue?) -> [StepVariableKey] {
        guard let props = schema?["properties"]?.objectValue else { return [] }
        return props.keys.sorted().map { key in
            let raw = props[key]!
            let desc = raw.str("description").flatMap { $0.isEmpty ? nil : $0 } ?? schemaTypeHint(raw) ?? ""
            return .init(key: key, label: key, description: desc)
        }
    }

    /// json-schema-utils.ts schemaTypeHint.
    static func schemaTypeHint(_ prop: JSONValue) -> String? {
        if let s = prop.str("type") { return s }
        if let arr = prop["type"]?.arrayValue?.compactMap(\.stringValue) {
            return arr.first(where: { $0 != "null" }) ?? arr.first
        }
        return nil
    }

    /// formatRunValue: a run value on one line, clipped to 60 characters.
    public static func formatRunValue(_ val: JSONValue?) -> String {
        guard let val else { return "" }
        if case .string(let s) = val {
            let flat = s.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
            let clipped = flat.count > 60 ? String(flat.prefix(60)) + "…" : flat
            return JSONValue.string(clipped).jsonText
        }
        let text = val.jsonText
        return text.count > 60 ? String(text.prefix(60)) + "…" : text
    }

    static let gateOutputKeys: [StepVariableKey] = [
        .init(key: "items", label: "items", description: "The rows the reviewer kept, when the text is a JSON list"),
        .init(key: "text", label: "text", description: "The text the gate showed, as the reviewer left it"),
        .init(
            key: "comments", label: "comments",
            description: "Anchored comments from the latest round, as JSON: quote, comment, anchored"),
        .init(key: "feedback", label: "feedback", description: "The general note from the latest round"),
        .init(key: "feedbackAll", label: "feedbackAll", description: "Every note, one line per round"),
        .init(key: "round", label: "round", description: "Which time the gate asked, from 1"),
    ]

    static let loopOutputKeys: [StepVariableKey] = [
        .init(key: "results", label: "results", description: "Each pass: its item, status and step outputs"),
        .init(key: "outputs", label: "outputs", description: "Each pass's final output, in order"),
        .init(key: "passes", label: "passes", description: "How many passes ran"),
        .init(key: "count", label: "count", description: "Items in the list, or the most passes allowed"),
    ]

    /// buildStepGroups: one group of `{{steps.<slug>.<key>}}` per upstream step with a slug.
    public static func stepGroups(
        _ ancestors: [WorkflowNode],
        lookupAction: ((String, String) -> ConnectorActionDef?)? = nil,
        lastRun: LastRunData? = nil
    ) -> [StepVariableGroup] {
        ancestors.compactMap { n -> StepVariableGroup? in
            guard let slug = n.slug, !slug.isEmpty else { return nil }
            var keys = defaultOutputKeys
            switch n.type {
            case .callConnectorAction:
                if let lookupAction, let conn = n.config.str("connectionId"), !conn.isEmpty,
                    let action = n.config.str("action"), !action.isEmpty
                {
                    let sk = schemaKeys(lookupAction(conn, action)?.outputSchema)
                    if !sk.isEmpty { keys = sk + defaultOutputKeys }
                }
            case .httpRequest:
                keys = [
                    .init(key: "status", label: "status", description: "HTTP status code"),
                    .init(key: "body", label: "body", description: "Response body, JSON-parsed when possible"),
                    .init(key: "headers", label: "headers", description: "Response headers"),
                ] + defaultOutputKeys
            case .approval:
                keys = gateOutputKeys + defaultOutputKeys
            case .loop:
                keys = loopOutputKeys + defaultOutputKeys
            case .launchAgent:
                let sk = n.config.bool("headless") == true ? schemaKeys(n.config["outputSchema"]) : []
                keys = sk + defaultOutputKeys + [worktreeOutputKey]
            default:
                break
            }
            let runOutputs = lastRun?.outputs[slug]
            let runState = lastRun?.states[n.id]
            if let runOutputs {
                let declared = Set(keys.map(\.key))
                let discovered = runOutputs.keys.sorted().filter { !declared.contains($0) }
                    .map { StepVariableKey(key: $0, label: $0, description: "From the last run") }
                keys = (discovered + keys).map { k in
                    var k = k
                    let v = formatRunValue(runOutputs[k.key])
                    k.value = v.isEmpty ? nil : v
                    return k
                }
            }
            return StepVariableGroup(
                nodeId: n.id, label: n.label, slug: slug, nodeType: n.type, keys: keys,
                connectionId: n.type == .callConnectorAction ? n.config.str("connectionId") : nil,
                runStatus: runState?.status, runCompletedAt: runState?.completedAt, runOutputs: runOutputs)
        }
    }

    static func editDistance(_ a: String, _ b: String) -> Int {
        let a = Array(a), b = Array(b)
        if a.isEmpty { return b.count }
        if b.isEmpty { return a.count }
        var prev = Array(0...b.count)
        for i in 1...a.count {
            var cur = [i] + Array(repeating: 0, count: b.count)
            for j in 1...b.count {
                cur[j] = min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (a[i - 1] == b[j - 1] ? 0 : 1))
            }
            prev = cur
        }
        return prev[b.count]
    }

    public static func suggestNearestPath(_ path: String, candidates: [String]) -> String? {
        var best: String?
        var bestScore = Int.max
        for c in candidates where c != path {
            let s = editDistance(path, c)
            if s < bestScore {
                bestScore = s
                best = c
            }
        }
        return bestScore <= max(3, path.count / 3) ? best : nil
    }

    static func walk(_ root: JSONValue?, _ path: [String]) -> JSONValue? {
        var cur = root
        for seg in path {
            switch cur {
            case .object(let o)?: cur = o[seg]
            case .array(let a)?:
                guard let i = Int(seg), a.indices.contains(i) else { return nil }
                cur = a[i]
            default: return nil
            }
        }
        return cur
    }

    static func stringifyResolved(_ v: JSONValue) -> String {
        switch v {
        case .null: return ""
        case .string, .number, .bool: return v.displayString ?? ""
        default: return v.jsonText
        }
    }

    /// previewStepTokens: resolves known `steps.*` tokens to last-run text and flags the first unknown one.
    public static func previewStepTokens(_ template: String, groups: [StepVariableGroup]) -> TemplatePreview {
        guard let regex = try? NSRegularExpression(pattern: #"\{\{\s*steps\.([\w.]+)\s*\}\}"#) else { return .init() }
        let ns = template as NSString
        let matches = regex.matches(in: template, range: NSRange(location: 0, length: ns.length))
        if matches.isEmpty { return .init() }
        let bySlug = Dictionary(groups.map { ($0.slug, $0) }, uniquingKeysWith: { a, _ in a })
        let knownPaths = groups.flatMap { g in g.keys.map { "steps.\(g.slug).\($0.key)" } }
        let knownSet = Set(knownPaths)
        var broken: (token: String, suggestion: String?)?
        var anyData = false
        var resolved = ""
        var last = 0
        for m in matches {
            resolved += ns.substring(with: NSRange(location: last, length: m.range.location - last))
            last = m.range.location + m.range.length
            let match = ns.substring(with: m.range)
            let path = ns.substring(with: m.range(at: 1))
            let parts = path.split(separator: ".", omittingEmptySubsequences: false).map(String.init)
            let slug = parts[0]
            let rest = Array(parts.dropFirst())
            func flag() {
                if broken == nil {
                    broken = ("steps.\(path)", suggestNearestPath("steps.\(path)", candidates: knownPaths))
                }
            }
            guard let group = bySlug[slug], !rest.isEmpty else {
                flag()
                resolved += match
                continue
            }
            guard let outputs = group.runOutputs else {
                resolved += match
                continue
            }
            guard let value = walk(.object(outputs), rest) else {
                let exactKnown = knownSet.contains("steps.\(path)")
                let keyDeclared = knownSet.contains("steps.\(slug).\(rest[0])")
                let keyEmitted = outputs[rest[0]] != nil
                if !exactKnown && !(keyDeclared && !keyEmitted) { flag() }
                resolved += match
                continue
            }
            anyData = true
            resolved += stringifyResolved(value)
        }
        resolved += ns.substring(from: last)
        if let broken { return TemplatePreview(resolved: nil, broken: broken) }
        return anyData ? TemplatePreview(resolved: resolved, broken: nil) : .init()
    }

    /// buildStepOutputsMap: each settled step's outputs by slug, plus a gate's outputs once it asked.
    public static func stepOutputsMap(_ run: WorkflowExecution, nodes: [WorkflowNode]) -> [String: [String: JSONValue]] {
        let byId = Dictionary(nodes.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        var out: [String: [String: JSONValue]] = [:]
        for s in run.nodeStates {
            guard let node = byId[s.nodeId], let slug = node.slug, !slug.isEmpty else { continue }
            let settled = s.status == .success || s.status == .error
            let gate = node.type == .approval ? gateOutputs(s) : nil
            if !settled && gate == nil { continue }
            var entry: [String: JSONValue] = [:]
            if settled {
                entry = s.structuredOutput?.objectValue ?? [:]
                let primary = (s.output?.isEmpty == false ? s.output : s.logs) ?? ""
                entry["output"] = .string(primary)
                entry["status"] = .string(s.status.rawValue)
                entry["error"] = .string(s.error ?? "")
                entry["worktreePath"] = .string("")
            }
            for (k, v) in gate ?? [:] { entry[k] = v }
            out[slug] = entry
        }
        return out
    }

    static func gateOutputs(_ s: NodeExecutionState) -> [String: JSONValue]? {
        if s.round == nil && (s.feedback ?? []).isEmpty { return nil }
        let entries = s.feedback ?? []
        let text = s.editedText ?? s.editableText ?? ""
        var out: [String: JSONValue] = [
            "text": .string(text),
            "feedback": .string(entries.last?.comment ?? ""),
            "feedbackAll": .string(entries.map { "Round \($0.round): \($0.comment)" }.joined(separator: "\n")),
            "comments": .array([]),
            "round": .number(Double(s.round ?? 1)),
        ]
        if case .array(let rows)? = JSONValue.parse(text), !rows.isEmpty,
            rows.allSatisfy({ $0.objectValue != nil })
        {
            out["items"] = .array(rows)
        }
        return out
    }

    /// The last run as buildStepGroups reads it.
    public static func lastRunData(_ run: WorkflowExecution?, nodes: [WorkflowNode]) -> LastRunData? {
        guard let run else { return nil }
        var states: [String: (status: NodeExecutionStatus, completedAt: String?)] = [:]
        for s in run.nodeStates { states[s.nodeId] = (s.status, s.completedAt) }
        return LastRunData(outputs: stepOutputsMap(run, nodes: nodes), states: states)
    }
}
