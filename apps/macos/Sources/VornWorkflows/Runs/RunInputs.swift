import Foundation
import VornCore

/// One manual-run input a trigger declares (WorkflowInputDef in shared/types.ts).
public struct WorkflowInputDef: Sendable, Hashable {
    public enum Kind: String, Sendable { case text, textarea, number, boolean, select, project, branch }

    public struct Option: Sendable, Hashable {
        public var value: String
        public var label: String
    }

    public var key: String
    public var label: String
    public var type: Kind
    public var required: Bool
    public var defaultValue: String?
    public var options: [Option]
    public var placeholder: String?
    public var description: String?

    public init(
        key: String, label: String = "", type: Kind = .text, required: Bool = false, defaultValue: String? = nil,
        options: [Option] = [], placeholder: String? = nil, description: String? = nil
    ) {
        self.key = key
        self.label = label
        self.type = type
        self.required = required
        self.defaultValue = defaultValue
        self.options = options
        self.placeholder = placeholder
        self.description = description
    }

    init?(_ json: JSONValue) {
        guard let key = json["key"]?.stringValue else { return nil }
        self.init(
            key: key, label: json["label"]?.stringValue ?? "",
            type: json["type"]?.stringValue.flatMap(Kind.init(rawValue:)) ?? .text,
            required: json["required"]?.boolValue ?? false, defaultValue: json["defaultValue"]?.stringValue,
            options: (json["options"]?.arrayValue ?? []).compactMap { o in
                o["value"]?.stringValue.map { Option(value: $0, label: o["label"]?.stringValue ?? $0) }
            },
            placeholder: json["placeholder"]?.stringValue, description: json["description"]?.stringValue)
    }

    /// What the field shows as its name.
    public var displayLabel: String { label.isEmpty ? key : label }
}

/// What a launching surface already knows about the run: a card's session, or a task (ManualRunContext).
public struct ManualRunContext: Sendable, Hashable {
    public var source: JSONValue?
    public var task: JSONValue?

    public init(source: JSONValue? = nil, task: JSONValue? = nil) {
        self.source = source
        self.task = task
    }

    public init(session: TerminalSession) {
        self.init(source: try? .from(session))
    }

    public var hasSource: Bool { source != nil || task != nil }

    /// The run context vornd takes: `{task?, source?}`.
    var json: [String: JSONValue] {
        var o: [String: JSONValue] = [:]
        if let task { o["task"] = task }
        if let source { o["source"] = source }
        return o
    }
}

/// workflow-helpers.ts and workflow-inputs.ts: which manual runs must ask first, and the answers they take.
public enum RunInputs {
    static func triggerConfig(_ wf: WorkflowDefinition) -> JSONValue? { wf.triggerNode?.config }

    /// isContextualWorkflow: a manual trigger flagged contextual.
    public static func isContextual(_ wf: WorkflowDefinition) -> Bool {
        let c = triggerConfig(wf)
        return c?["triggerType"]?.stringValue == "manual" && c?["contextual"]?.boolValue == true
    }

    /// getWorkflowInputs: only keys a template can reference count.
    public static func inputs(_ wf: WorkflowDefinition) -> [WorkflowInputDef] {
        let c = triggerConfig(wf)
        guard c?["triggerType"]?.stringValue == "manual" else { return [] }
        return (c?["inputs"]?.arrayValue ?? []).compactMap(WorkflowInputDef.init).filter { isIdentifier($0.key) }
    }

    static func isIdentifier(_ key: String) -> Bool {
        key.range(of: #"^[a-zA-Z_][a-zA-Z0-9_]*$"#, options: .regularExpression) != nil
    }

    /// needsRunPrompt: a contextual run without a source, or any declared input.
    public static func needsPrompt(_ wf: WorkflowDefinition, context: ManualRunContext? = nil) -> Bool {
        (isContextual(wf) && context?.hasSource != true) || !inputs(wf).isEmpty
    }

    /// parseNumberInput: blank and unparseable both mean no value.
    public static func parseNumber(_ raw: String) -> JSONValue {
        let t = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty, let n = Double(t), n.isFinite else { return .string("") }
        return .number(n)
    }

    static func initialValue(_ def: WorkflowInputDef) -> JSONValue {
        switch def.type {
        case .boolean: return .bool(def.defaultValue == "true")
        case .number: return def.defaultValue.map(parseNumber) ?? .string("")
        default: return .string(def.defaultValue ?? "")
        }
    }

    public static func initialValues(_ defs: [WorkflowInputDef]) -> [String: JSONValue] {
        Dictionary(defs.map { ($0.key, initialValue($0)) }, uniquingKeysWith: { a, _ in a })
    }

    /// areInputsValid: every required input answered; false is an answer for a toggle.
    public static func areValid(_ defs: [WorkflowInputDef], _ values: [String: JSONValue]) -> Bool {
        defs.allSatisfy { def in
            let value = values[def.key]
            if case .number(let n)? = value, !n.isFinite { return false }
            guard def.required else { return true }
            switch value {
            case nil, .null?: return false
            case .bool?: return true
            case _ where def.type == .boolean: return false
            case .string(let s)?: return !s.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            default: return true
            }
        }
    }

    // MARK: Source fields (SourcePromptDialog detectRequiredFields)

    public struct RequiredFields: Equatable, Sendable {
        public var needsProject = false
        public var needsBranch = false
        public var needsWorktree = false

        public init(needsProject: Bool = false, needsBranch: Bool = false, needsWorktree: Bool = false) {
            self.needsProject = needsProject
            self.needsBranch = needsBranch
            self.needsWorktree = needsWorktree
        }
    }

    /// Which source fields the workflow's steps reference.
    public static func requiredFields(_ wf: WorkflowDefinition) -> RequiredFields {
        var r = RequiredFields()
        let projectFields = ["cwd", "projectPath", "projectName"]
        for node in wf.nodes {
            let s = { (k: String) in node.config[k]?.stringValue }
            switch node.type {
            case .launchAgent:
                if isContextRef(s("projectName")) || isContextRef(s("projectPath"))
                    || projectFields.contains(where: { containsContextRef(s("prompt"), $0) }) {
                    r.needsProject = true
                }
                if isContextRef(s("branch")) || containsContextRef(s("prompt"), "branch") { r.needsBranch = true }
                if s("useWorktree") == "fromContext" {
                    r.needsWorktree = true
                    r.needsProject = true
                    r.needsBranch = true
                }
            case .script:
                if isContextRef(s("cwd")) || isContextRef(s("projectName")) || isContextRef(s("projectPath"))
                    || projectFields.contains(where: { containsContextRef(s("scriptContent"), $0) }) {
                    r.needsProject = true
                }
                if containsContextRef(s("scriptContent"), "branch") { r.needsBranch = true }
            default:
                break
            }
        }
        return r
    }

    /// A value that is exactly one `{{context.x}}` reference.
    public static func isContextRef(_ value: String?) -> Bool {
        guard let value else { return false }
        return value.range(of: #"^\s*\{\{\s*context\.\w+\s*\}\}\s*$"#, options: .regularExpression) != nil
    }

    /// A value that mentions `{{context.<field>}}`.
    public static func containsContextRef(_ value: String?, _ field: String) -> Bool {
        guard let value else { return false }
        return value.range(of: #"\{\{\s*context\.\#(field)\s*\}\}"#, options: .regularExpression) != nil
    }
}
