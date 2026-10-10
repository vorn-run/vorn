import Foundation
import VornCore

// Ports of src/renderer/lib/run-presentation.ts, format-time.ts and node-visuals.ts.

/// Which filter a run falls under; a paused run is waiting, not running.
public enum RunBucket: String, Sendable, CaseIterable {
    case all, running, waiting, error, success
}

/// What a dot or a step is, in the shared status vocabulary (workflow-status.ts).
public enum WorkflowStatusKey: String, Sendable {
    case pending, running, success, error, skipped, waiting, cancelled

    init(_ s: NodeExecutionStatus) { self = WorkflowStatusKey(rawValue: s.rawValue) ?? .pending }
    init(_ s: RunStatus) { self = WorkflowStatusKey(rawValue: s.rawValue) ?? .error }

    /// StatusDot's accessible label (RunEntry STATUS_LABELS).
    public var label: String {
        switch self {
        case .success: return "Success"
        case .error: return "Error"
        case .running: return "Running"
        case .pending: return "Pending"
        case .skipped: return "Skipped"
        case .waiting: return "Waiting for approval"
        case .cancelled: return "Stopped"
        }
    }
}

public enum RunSource: String, Sendable {
    case manual, schedule, task, connector, restore
}

/// What a run row needs from its workflow; a deleted workflow leaves only the name.
public struct RunWorkflowRef: Sendable, Hashable {
    public var name: String?
    public var icon: String?
    public var iconColor: String?
    public var nodes: [WorkflowNode]

    public init(name: String? = nil, icon: String? = nil, iconColor: String? = nil, nodes: [WorkflowNode] = []) {
        self.name = name
        self.icon = icon
        self.iconColor = iconColor
        self.nodes = nodes
    }

    public init(_ w: WorkflowDefinition) {
        self.init(name: w.name, icon: w.icon, iconColor: w.iconColor, nodes: w.nodes)
    }
}

public struct RunPresentation: Sendable, Hashable {
    public var title: String
    public var subtitle: String?
    public var source: RunSource
    public var sourceLabel: String
    public var iconName: String?
    public var iconColor: String?
    public var connectorId: String?
}

public enum RunPresenter {
    public static func bucket(of run: WorkflowExecution) -> RunBucket {
        if run.status == .running {
            return run.nodeStates.contains { $0.status == .waiting } ? .waiting : .running
        }
        return run.status == .success ? .success : .error
    }

    public static func nodeLabel(_ node: WorkflowNode?, _ nodeId: String) -> String {
        if let label = node?.label, !label.isEmpty { return label }
        return String(nodeId.prefix(8))
    }

    public static func isRejected(_ run: WorkflowExecution) -> Bool {
        run.status == .error && run.failedStep?.rejectedAt != nil
    }

    /// A settled run's dot: a rejection reads as a decision, like a stop.
    public static func dotStatus(_ run: WorkflowExecution) -> WorkflowStatusKey {
        isRejected(run) ? .cancelled : WorkflowStatusKey(run.status)
    }

    /// The dot a list row or detail header shows: waiting wins over the run's own state.
    public static func liveDotStatus(_ run: WorkflowExecution) -> WorkflowStatusKey {
        run.waitingStep != nil ? .waiting : dotStatus(run)
    }

    public static func statusLine(_ run: WorkflowExecution, nodes: [WorkflowNode]) -> String {
        func named(_ id: String) -> String { nodeLabel(nodes.first { $0.id == id }, id) }
        if let waiting = run.waitingStep {
            return "\(waiting.isSignInWait ? "Waiting for sign-in" : "Waiting") at \(named(waiting.nodeId))"
        }
        if run.status == .running {
            if let active = run.nodeStates.first(where: { $0.status == .running }) {
                return "Running \(named(active.nodeId))"
            }
            return "Running"
        }
        if run.status == .error {
            guard let failed = run.failedStep else { return "Failed" }
            guard failed.rejectedAt != nil else { return "Failed at \(named(failed.nodeId))" }
            let last = failed.feedback?.last
            let note = last?.decision == "reject" ? " · \(last!.comment)" : ""
            return "Rejected at \(named(failed.nodeId))\(note)"
        }
        return run.status == .cancelled ? "Stopped" : "Completed"
    }

    /// Steps that succeeded out of all the run reached, the trigger left out.
    public static func stepProgress(_ run: WorkflowExecution, nodes: [WorkflowNode]) -> (done: Int, total: Int) {
        let triggers = Set(nodes.filter { $0.type == .trigger }.map(\.id))
        let steps = run.nodeStates.filter { !triggers.contains($0.nodeId) }
        return (steps.filter { $0.status == .success }.count, steps.count)
    }

    /// "N of M steps" for an unfinished run that got somewhere (RunListRow progressOf).
    public static func progressLabel(_ run: WorkflowExecution, workflow: RunWorkflowRef?) -> String? {
        guard run.status != .success, let nodes = workflow?.nodes, !nodes.isEmpty else { return nil }
        let p = stepProgress(run, nodes: nodes)
        return p.done > 0 ? "\(p.done) of \(p.total) steps" : nil
    }

    static func source(_ run: WorkflowExecution, triggerType: String?) -> RunSource {
        if run.connectorItem != nil { return .connector }
        if run.triggerTaskId != nil { return .task }
        if triggerType == "once" || triggerType == "recurring" { return .schedule }
        if triggerType == "connectorPoll" { return .connector }
        if triggerType == "taskCreated" || triggerType == "taskStatusChanged" { return .task }
        if run.triggerSession != nil || triggerType == "sessionRestored" { return .restore }
        return .manual
    }

    static func connectorTitle(_ item: ConnectorItemContext, connectorId: String) -> String? {
        if item.externalUrl?.contains("/pull/") == true { return "PR #\(item.externalId)" }
        if item.externalUrl?.contains("/issues/") == true { return "Issue #\(item.externalId)" }
        return item.externalId.isEmpty ? nil : "\(connectorId) \(item.externalId)"
    }

    public static func describe(_ run: WorkflowExecution, workflow: RunWorkflowRef?) -> RunPresentation {
        let nodes = workflow?.nodes ?? []
        let trigger = nodes.first { $0.type == .trigger }
        let src = source(run, triggerType: trigger?.config["triggerType"]?.stringValue)
        let trimmed = workflow?.name?.trimmingCharacters(in: .whitespacesAndNewlines)
        let name = (trimmed?.isEmpty ?? true) ? nil : trimmed
        let icon = workflow?.icon
        let color = workflow?.iconColor

        if let item = run.connectorItem {
            let connectorId = item.connectorId
            let title = connectorTitle(item, connectorId: connectorId) ?? item.title
            return RunPresentation(
                title: title, subtitle: item.title != title ? item.title : name, source: .connector,
                sourceLabel: connectorId, iconName: icon, iconColor: color, connectorId: connectorId)
        }
        if let session = run.triggerSession {
            return RunPresentation(
                title: name ?? session.label, subtitle: "restore · \(session.restore) · \(session.label)",
                source: .restore, sourceLabel: "restore", iconName: icon, iconColor: color)
        }
        if let task = run.triggerTaskId {
            let short = "Task \(task.prefix(6))"
            return RunPresentation(
                title: name ?? short, subtitle: short, source: .task, sourceLabel: "task",
                iconName: icon, iconColor: color)
        }
        let label = src == .schedule ? "scheduled" : src == .restore ? "restore" : "manual"
        return RunPresentation(
            title: name ?? String(run.workflowId.prefix(8)), subtitle: nil, source: src, sourceLabel: label,
            iconName: icon, iconColor: color)
    }

    static let verdictKeys = ["verdict", "recommendation", "decision", "summary", "result", "status"]

    /// The conclusion the run's last typed step wrote, when short enough to be one.
    public static func verdict(_ run: WorkflowExecution) -> String? {
        for state in run.nodeStates.reversed() {
            guard let out = state.structuredOutput?.objectValue else { continue }
            for key in verdictKeys {
                if let v = out[key]?.stringValue, !v.trimmingCharacters(in: .whitespaces).isEmpty, v.count <= 40 {
                    return v.trimmingCharacters(in: .whitespacesAndNewlines)
                }
            }
        }
        return nil
    }

    /// One quiet line under a run row's title.
    public static func detailLine(
        _ run: WorkflowExecution, workflow: RunWorkflowRef?, deleted: Bool, now: Date = Date()
    ) -> String {
        let p = describe(run, workflow: workflow)
        let parts: [String?] = [
            statusLine(run, nodes: workflow?.nodes ?? []),
            p.subtitle ?? p.sourceLabel,
            progressLabel(run, workflow: workflow),
            run.partial == true ? "partial" : nil,
            deleted ? "deleted" : nil,
            TimeFormat.relative(run.startedAt, now: now),
        ]
        return parts.compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
    }

    /// The meta line under the detail pane's title.
    public static func metaLine(
        _ run: WorkflowExecution, workflow: RunWorkflowRef?, now: Date = Date()
    ) -> String {
        let p = describe(run, workflow: workflow)
        let name = workflow?.name?.trimmingCharacters(in: .whitespacesAndNewlines)
        let parts: [String?] = [
            (name?.isEmpty == false && name != p.title) ? name : nil,
            "Run \(run.runId.prefix(8))",
            p.sourceLabel,
            TimeFormat.relative(run.startedAt, now: now),
            TimeFormat.runDuration(run.startedAt, run.completedAt),
        ]
        return parts.compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
    }
}

public enum TimeFormat {
    nonisolated(unsafe) private static let isoFractional: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()

    nonisolated(unsafe) private static let iso: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f
    }()

    private static let lock = NSLock()

    public static func parse(_ s: String) -> Date? {
        lock.lock()
        defer { lock.unlock() }
        return isoFractional.date(from: s) ?? iso.date(from: s)
    }

    /// "1.4s", "2m 13s"; "running..." without an end; "—" when unreadable.
    public static func runDuration(_ start: String, _ end: String?) -> String {
        guard let end else { return "running..." }
        guard let s = parse(start), let e = parse(end) else { return "—" }
        let ms = Int((e.timeIntervalSince(s) * 1000).rounded())
        if ms < 0 { return "—" }
        if ms < 1000 { return "\(ms)ms" }
        if ms < 60000 { return String(format: "%.1fs", Double(ms) / 1000) }
        return "\(ms / 60000)m \((ms % 60000) / 1000)s"
    }

    public static func relative(_ iso: String, now: Date = Date()) -> String {
        guard let date = parse(iso) else { return "Unknown" }
        let diff = now.timeIntervalSince(date)
        if diff < 0 {
            return date.formatted(date: .numeric, time: .standard)
        }
        let mins = Int(diff / 60)
        if mins < 1 { return "Just now" }
        if mins < 60 { return "\(mins)m ago" }
        let hrs = mins / 60
        if hrs < 24 { return "\(hrs)h ago" }
        let days = hrs / 24
        if days < 30 { return "\(days)d ago" }
        return date.formatted(.dateTime.month(.abbreviated).day())
    }

    /// "M:SS" since a start, for the editor's running timer.
    public static func elapsed(since start: String, now: Date = Date()) -> String {
        let total = max(0, Int(now.timeIntervalSince(parse(start) ?? now)))
        return "\(total / 60):\(String(format: "%02d", total % 60))"
    }
}

public enum StepVisuals {
    /// The one-line "what this step is configured to do" under a step's name.
    public static func meta(_ node: WorkflowNode?) -> String? {
        guard let node else { return nil }
        func str(_ key: String) -> String? {
            guard let v = node.config[key]?.stringValue?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !v.isEmpty else { return nil }
            return v
        }
        func join(_ parts: String?...) -> String? {
            let kept = parts.compactMap { $0 }
            return kept.isEmpty ? nil : kept.joined(separator: " · ")
        }
        switch node.type {
        case .trigger:
            return str("triggerType") == "connectorPoll"
                ? join(str("connectorId"), str("event"))
                : join(str("triggerType"), str("cron"))
        case .script: return join(str("scriptType"), str("projectName"))
        case .launchAgent: return join(str("projectName"), str("agentType"), str("branch"))
        case .condition: return join(str("variable"), str("operator"), str("value"))
        case .createTaskFromItem: return join(str("project"), str("initialStatus"))
        case .callConnectorAction: return join(str("connectorId"), str("action"))
        case .httpRequest: return join(str("method"), str("url"))
        default: return nil
        }
    }

    public struct TimelineEntry: Hashable, Sendable {
        public enum Kind: Sendable { case engine, agent }
        public var kind: Kind
        public var text: String
    }

    static let inlineLogChars = 8000

    public static func inlineLogTail(_ logs: String) -> String {
        guard logs.count > inlineLogChars else { return logs }
        let elided = logs.count - inlineLogChars
        let n = elided.formatted(.number.grouping(.automatic))
        return "… \(n) earlier characters hidden — use View full output\n\n\(logs.suffix(inlineLogChars))"
    }

    /// Engine notes up to the agent's first output, then its words, then how the step ended.
    public static func timeline(logs: String?, diagnostics: String?) -> [TimelineEntry] {
        let notes = (diagnostics ?? "").split(separator: "\n", omittingEmptySubsequences: true).map(String.init)
        let trimmed = logs?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !trimmed.isEmpty, let logs else { return notes.map { TimelineEntry(kind: .engine, text: $0) } }
        let agentText = inlineLogTail(logs)
        let seam = notes.firstIndex { $0.range(of: #"^\[\+[\d.]+s\] First output from the agent\b"#, options: .regularExpression) != nil }
        let before = seam.map { Array(notes[...$0]) } ?? notes
        let after = seam.map { Array(notes[($0 + 1)...]) } ?? []
        return before.map { TimelineEntry(kind: .engine, text: $0) }
            + [TimelineEntry(kind: .agent, text: agentText)]
            + after.map { TimelineEntry(kind: .engine, text: $0) }
    }

    /// What an expanded step with no output says, by its status.
    public static func emptyLog(_ status: NodeExecutionStatus) -> String {
        switch status {
        case .running: return "No output captured yet…"
        case .pending: return "Step hasn't started yet."
        case .skipped: return "Step was skipped."
        default: return "No output recorded."
        }
    }

    /// One-line preview of a run input, clipped at 60 characters.
    public static func inputPreview(_ value: JSONValue) -> String {
        let text: String
        switch value {
        case .null: text = ""
        case .object, .array:
            text = (try? String(decoding: JSONEncoder().encode(value), as: UTF8.self)) ?? ""
        default: text = value.displayString ?? ""
        }
        return text.count > 60 ? "\(text.prefix(60))…" : text
    }
}

/// Approval gate rounds (workflow-graph gateMaxRounds / canRequestChanges, gate-round roundLabel).
public enum GateRounds {
    public static func maxRounds(_ config: JSONValue?) -> Int {
        guard let n = config?["feedback"]?["maxRounds"]?.numberValue, n.isFinite else { return 3 }
        return min(max(1, Int(n.rounded(.down))), 10)
    }

    public static func canRequestChanges(_ config: JSONValue?, round: Int?) -> Bool {
        guard config?["feedback"]?["from"]?.stringValue?.isEmpty == false else { return false }
        return (round ?? 1) < maxRounds(config)
    }

    public static func label(_ config: JSONValue?, round: Int?) -> String? {
        guard config?["feedback"]?["from"]?.stringValue?.isEmpty == false else { return nil }
        return "round \(round ?? 1) of \(maxRounds(config))"
    }
}
