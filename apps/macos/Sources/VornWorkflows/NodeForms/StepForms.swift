import SwiftUI
import VornCore
import VornUI

/// A label with a quieter tail: `Stop early when (optional)`.
struct FormLabelTail: View {
    let text: String
    let tail: String

    var body: some View {
        (Text(text) + Text(" " + tail).foregroundColor(Theme.gray600))
            .font(.system(size: 13, weight: .medium))
            .foregroundStyle(Theme.gray400)
            .cssLineHeight(13)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.bottom, 8)
    }
}

/// Help text that names a variable in mono.
func helpText(_ parts: [(String, Bool)], size: CGFloat = 11) -> Text {
    parts.reduce(Text("")) { acc, p in
        acc + (p.1 ? Text(p.0).font(.system(size: size, design: .monospaced)).foregroundColor(Theme.gray400) : Text(p.0))
    }
}

/// The approval and loop field: `px-3 py-2 bg-white/[0.03] border-white/[0.08] text-[13px] text-gray-200`.
struct QuietField: View {
    let placeholder: String
    @Binding var text: String
    var mono = false
    var size: CGFloat = 13
    var vertical: CGFloat = 8
    var center = false

    var body: some View {
        BareTextInput(placeholder: placeholder, text: $text, size: size, mono: mono, color: Theme.gray200)
            .multilineTextAlignment(center ? .center : .leading)
            .padding(.horizontal, 12)
            .padding(.vertical, vertical)
            .background(Theme.white(0.03), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.08), lineWidth: 1))
    }
}

// MARK: Script

/// ScriptConfigForm.tsx.
struct ScriptConfigForm: View {
    let store: WorkflowsStore
    let data: NodeFormsData
    @Binding var config: JSONValue
    let step: StepContext
    @State private var advancedOpen: Bool

    init(store: WorkflowsStore, data: NodeFormsData, config: Binding<JSONValue>, step: StepContext) {
        self.store = store
        self.data = data
        _config = config
        self.step = step
        _advancedOpen = State(initialValue: !(config.wrappedValue["args"]?.arrayValue ?? []).isEmpty)
    }

    static let types: [SelectOption] = [
        SelectOption("bash", "Bash", icon: AnyView(LucideIcon(.terminal, size: 12).foregroundStyle(Theme.gray400))),
        SelectOption("powershell", "PowerShell", icon: AnyView(LucideIcon(.terminal, size: 12).foregroundStyle(Theme.gray400))),
        SelectOption("python", "Python", icon: AnyView(LucideIcon(.fileCode, size: 12).foregroundStyle(Theme.gray400))),
        SelectOption("node", "Node", icon: AnyView(LucideIcon(.braces, size: 12).foregroundStyle(Theme.gray400))),
    ]

    var body: some View {
        let fromContext = StepEdits.scriptCwdFromContext(config)
        let scriptType = config.str("scriptType") ?? ""
        VStack(alignment: .leading, spacing: 20) {
            FormSection("Type") {
                FormSelect(value: scriptType, options: Self.types) { config.set("scriptType", string: $0) }
            }
            FormSection("Working Directory") {
                FormProjectPicker(
                    current: fromContext ? "" : config.str("projectName") ?? "", projects: store.projects, allowNone: true,
                    isFromContext: fromContext,
                    onSelectFromContext: step.isContextualTrigger ? { config = StepEdits.scriptFromContext(config) } : nil
                ) { name in
                    if name.isEmpty {
                        config = StepEdits.scriptProject(config, nil)
                    } else if let p = store.projects.first(where: { $0.name == name }) {
                        config = StepEdits.scriptProject(config, p)
                    }
                }
            }
            if !data.connectorKeys.isEmpty {
                FormSection("Secrets from") {
                    FormSelect(
                        value: config.str("secretsFrom") ?? "",
                        options: [SelectOption("", "None")] + data.connectorKeys.map { SelectOption($0.connectionId, $0.name) }
                    ) { config = StepEdits.emptyIsAbsent(config, "secretsFrom", $0) }
                    FormHint(
                        "The connection's secrets are read on the server and handed to this step alone, as environment variables. The workflow file names the connection, never the value.",
                        top: 6, color: Theme.gray600)
                }
            }
            FormSection("Script", hint: StepEdits.hasTemplateVars(step) ? "Type {{ to insert step outputs or trigger variables" : nil) {
                VariableAutocomplete(
                    value: config.str("scriptContent") ?? "", onChange: { config.set("scriptContent", string: $0) },
                    placeholder: "Enter \(scriptType) script...", rows: 10, stepGroups: step.stepGroups,
                    contextVars: StepEdits.agentContextVars(step), mono: true)
            }
            AdvancedSection(open: $advancedOpen) {
                FormSection("Arguments") {
                    FormTextField(
                        placeholder: "arg1 arg2 --flag",
                        text: Binding(
                            get: { (config["args"]?.arrayValue ?? []).compactMap(\.stringValue).joined(separator: " ") },
                            set: { config = StepEdits.scriptArgs(config, $0) }))
                }
            }
            .padding(.top, 16)
            .overlay(alignment: .top) { Hairline(opacity: 0.06) }
        }
        .onAppear {
            if let next = StepEdits.scriptClearContext(config, contextual: step.isContextualTrigger) { config = next }
        }
    }
}

// MARK: HTTP

/// HttpRequestConfigForm.tsx.
struct HttpRequestConfigForm: View {
    let store: WorkflowsStore
    let data: NodeFormsData
    @Binding var config: JSONValue
    let step: StepContext
    @State private var makingProfile = false
    @State private var rows: [(String, String)] = []

    static let methods = ["GET", "POST", "PUT", "PATCH", "DELETE"].map { SelectOption($0, $0) }

    private var stored: [(String, String)] {
        (config["headers"]?.objectValue ?? [:]).map { ($0.key, $0.value.stringValue ?? "") }.sorted { $0.0 < $1.0 }
    }

    /// The rows on screen, re-read when the stored headers changed under them.
    private var entries: [(String, String)] {
        let mine = Dictionary(StepEdits.headerEntries(rows), uniquingKeysWith: { _, b in b })
        let theirs = Dictionary(stored, uniquingKeysWith: { _, b in b })
        return mine == theirs ? StepEdits.headerEntries(rows) : stored
    }

    private func setHeaders(_ next: [(String, String)]) {
        rows = StepEdits.headerEntries(next)
        config = StepEdits.httpHeaders(config, next)
    }

    var body: some View {
        let vars = StepEdits.agentContextVars(step)
        let profiles = data.connections.filter { $0.connectorId == "http" }
        let list = entries
        VStack(alignment: .leading, spacing: 20) {
            HStack(alignment: .top, spacing: 8) {
                FormSection("Method") {
                    FormSelect(value: config.str("method") ?? "", options: Self.methods) { config.set("method", string: $0) }
                }
                .frame(width: 110)
                FormSection("URL") {
                    VariableAutocomplete(
                        value: config.str("url") ?? "", onChange: { config.set("url", string: $0) },
                        placeholder: (config.str("profileConnectionId") ?? "").isEmpty ? "https://api.example.com/items" : "/v1/items",
                        rows: 1, stepGroups: step.stepGroups, contextVars: vars, mono: true)
                }
            }
            FormSection("Auth profile") {
                FormSelect(
                    value: config.str("profileConnectionId") ?? "",
                    options: [SelectOption("", "None")] + profiles.map { SelectOption($0.id, $0.name) }
                        + [SelectOption(StepEdits.newProfile, "Create profile…")]
                ) { v in
                    if v == StepEdits.newProfile {
                        makingProfile = true
                    } else {
                        config = StepEdits.emptyIsAbsent(config, "profileConnectionId", v)
                    }
                }
                if makingProfile {
                    HttpProfileForm(store: store, name: "") { id in
                        makingProfile = false
                        config.set("profileConnectionId", string: id)
                        Task { await data.load() }
                    } onCancel: {
                        makingProfile = false
                    }
                    .padding(.top, 8)
                }
                FormHint(
                    "A profile adds its base URL and auth to the request on the server, so its secret never enters this workflow.",
                    top: 6, color: Theme.gray600)
            }
            FormSection("Headers") {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(Array(list.enumerated()), id: \.offset) { i, entry in
                        HStack(spacing: 6) {
                            BareTextInput(
                                placeholder: "Name",
                                text: Binding(get: { entry.0 }, set: { var n = list; n[i] = ($0, entry.1); setHeaders(n) }),
                                size: 12, mono: true
                            )
                            .padding(.horizontal, 8)
                            .padding(.vertical, 6)
                            .background(Theme.white(0.03), in: RoundedRectangle(cornerRadius: Theme.radius))
                            .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.08), lineWidth: 1))
                            .frame(width: 140)
                            VariableAutocomplete(
                                value: entry.1, onChange: { var n = list; n[i] = (entry.0, $0); setHeaders(n) },
                                placeholder: "Value", rows: 1, stepGroups: step.stepGroups, contextVars: vars, mono: true)
                            Hovering { hovered in
                                Button {
                                    setHeaders(list.enumerated().filter { $0.offset != i }.map(\.element))
                                } label: {
                                    LucideIcon(.x, size: 12)
                                        .foregroundStyle(hovered ? Color.white : Theme.gray500)
                                        .padding(4)
                                        .background(hovered ? Theme.white(0.06) : .clear, in: RoundedRectangle(cornerRadius: Theme.radius))
                                        .contentShape(Rectangle())
                                }
                                .buttonStyle(.plain)
                                .help("Remove header")
                            }
                        }
                    }
                    Hovering { hovered in
                        Button {
                            setHeaders(list + [("", "")])
                        } label: {
                            HStack(spacing: 4) {
                                LucideIcon(.plus, size: 11)
                                Text("Add header").font(.system(size: 11)).cssLineHeight(11)
                            }
                            .foregroundStyle(hovered ? Theme.gray300 : Theme.gray500)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            FormSection("Body") {
                VariableAutocomplete(
                    value: config.str("body") ?? "", onChange: { config.set("body", string: $0) },
                    placeholder: "{\"name\": \"{{trigger.body.name}}\"}", rows: 6, stepGroups: step.stepGroups,
                    contextVars: vars, mono: true)
                FormHint("Sent as-is. GET requests send no body.", top: 6, color: Theme.gray600)
            }
        }
        .onAppear { rows = stored }
    }
}

/// HttpProfileForm.tsx: an HTTP auth profile, made where it was asked for, then tested.
struct HttpProfileForm: View {
    let store: WorkflowsStore
    @State var name: String
    let onDone: (String) -> Void
    let onCancel: () -> Void
    @State private var baseUrl = ""
    @State private var authHeader = "Authorization: Bearer {{secret}}"
    @State private var secret = ""
    @State private var saving = false
    @State private var tested: (ok: Bool?, message: String?)?
    @State private var error: String?
    @State private var madeId: String?
    private let original: String

    init(store: WorkflowsStore, name: String, onDone: @escaping (String) -> Void, onCancel: @escaping () -> Void) {
        self.store = store
        _name = State(initialValue: name)
        original = name
        self.onDone = onDone
        self.onCancel = onCancel
    }

    private var ready: Bool {
        !name.trimmingCharacters(in: .whitespaces).isEmpty && !baseUrl.trimmingCharacters(in: .whitespaces).isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("HTTP profile").font(.system(size: 12, weight: .medium)).foregroundStyle(Theme.gray200).padding(.bottom, 8)
            field("Name", $name)
            field("Base URL", $baseUrl, placeholder: "https://api.example.com")
            field("Auth header", $authHeader, mono: true)
            field("Secret", $secret, secure: true)
            if let tested {
                HStack(alignment: .top, spacing: 6) {
                    if tested.ok == false { Image(systemName: "exclamationmark.circle").font(.system(size: 11)) } else {
                        LucideIcon(.check, size: 12)
                    }
                    Text(tested.message ?? (tested.ok == true ? "The profile answered." : "No answer to check."))
                        .font(.system(size: 11))
                }
                .foregroundStyle(tested.ok == false ? Theme.danger : Theme.statusSage)
                .padding(.top, 8)
            }
            if let error { Text(error).font(.system(size: 11)).foregroundStyle(Theme.danger).padding(.top, 8) }
            HStack(spacing: 8) {
                Spacer()
                Button("Cancel", action: onCancel).buttonStyle(ProfileButtonStyle(bordered: false))
                Button(saving ? "Testing…" : "Save and test") { Task { await save() } }
                    .buttonStyle(ProfileButtonStyle(bordered: true))
                    .disabled(saving || !ready)
                    .opacity(saving || !ready ? 0.5 : 1)
            }
            .padding(.top, 12)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .background(Theme.white(0.03), in: RoundedRectangle(cornerRadius: 2))
        .overlay(RoundedRectangle(cornerRadius: 2).strokeBorder(Theme.white(0.08), lineWidth: 1))
    }

    private func field(_ label: String, _ text: Binding<String>, placeholder: String = "", mono: Bool = false, secure: Bool = false)
        -> some View
    {
        VStack(alignment: .leading, spacing: 4) {
            Text(label).font(.system(size: 11)).foregroundStyle(Theme.gray500)
            Group {
                if secure {
                    SecureField("", text: text).textFieldStyle(.plain).font(.system(size: 12))
                } else {
                    BareTextInput(placeholder: placeholder, text: text, size: 12, mono: mono, color: Theme.gray200)
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .background(Theme.white(0.05), in: RoundedRectangle(cornerRadius: 2))
            .overlay(RoundedRectangle(cornerRadius: 2).strokeBorder(Theme.white(0.1), lineWidth: 1))
        }
        .padding(.bottom, 8)
    }

    private func save() async {
        saving = true
        error = nil
        tested = nil
        defer { saving = false }
        var filters: JSONValue = ["profileName": .string(name), "baseUrl": .string(baseUrl), "authHeader": .string(authHeader)]
        if !secret.isEmpty { filters.set("secret", string: secret) }
        let named = name.trimmingCharacters(in: .whitespaces).isEmpty ? original : name.trimmingCharacters(in: .whitespaces)
        do {
            let id: String
            if let madeId {
                let r = try await store.backend.rpc(
                    "connection:update", ["id": .string(madeId), "updates": ["name": .string(named), "filters": filters]])
                id = r.str("id") ?? madeId
            } else {
                let r = try await store.backend.rpc(
                    "connection:create",
                    ["connectorId": "http", "name": .string(named), "filters": filters, "syncIntervalMinutes": 5,
                     "statusMapping": .object([:])])
                id = r.str("id") ?? ""
            }
            madeId = id
            let result = try await store.backend.rpc("connection:preflight", .string(id))
            tested = (result.bool("ok"), result.str("message"))
            if result.bool("ok") != false { onDone(id) }
        } catch {
            self.error = (error as? LocalizedError)?.errorDescription ?? "The profile could not be saved"
        }
    }
}

private struct ProfileButtonStyle: ButtonStyle {
    let bordered: Bool
    func makeBody(configuration: Configuration) -> some View {
        Hovering { hovered in
            configuration.label
                .font(.system(size: 11))
                .foregroundStyle(hovered ? Color.white : bordered ? Theme.gray200 : Theme.gray400)
                .padding(.horizontal, 10)
                .padding(.vertical, 4)
                .background(hovered ? Theme.white(0.06) : .clear, in: RoundedRectangle(cornerRadius: 2))
                .overlay(RoundedRectangle(cornerRadius: 2).strokeBorder(Theme.white(bordered ? 0.1 : 0), lineWidth: 1))
        }
    }
}

// MARK: Condition

/// ConditionConfigForm.tsx.
struct ConditionConfigForm: View {
    @Binding var config: JSONValue
    let step: StepContext

    static let operators: [(String, String)] = [
        ("equals", "equals"), ("notEquals", "not equals"), ("contains", "contains"), ("notContains", "not contains"),
        ("isEmpty", "is empty"), ("isNotEmpty", "is not empty"),
    ]

    var body: some View {
        let vars = StepEdits.plainContextVars(triggerType: step.triggerType, connectorItem: false, inputVars: step.inputVars)
        let op = config.str("operator") ?? ""
        let hidesValue = op == "isEmpty" || op == "isNotEmpty"
        let variable = config.str("variable") ?? ""
        VStack(alignment: .leading, spacing: 20) {
            FormSection("Variable") {
                VariableAutocomplete(
                    value: variable, onChange: { config.set("variable", string: $0) }, placeholder: "e.g. {{steps.build.status}}",
                    rows: 1, stepGroups: step.stepGroups, contextVars: vars, mono: true)
            }
            FormSection("Operator") {
                FormSelect(value: op, options: Self.operators.map { SelectOption($0.0, $0.1) }) { config.set("operator", string: $0) }
            }
            if !hidesValue {
                FormSection("Value") {
                    VariableAutocomplete(
                        value: config.str("value") ?? "", onChange: { config.set("value", string: $0) },
                        placeholder: "Compare against...", rows: 1, stepGroups: step.stepGroups, contextVars: vars, mono: true)
                }
            }
            if !variable.isEmpty {
                let label = Self.operators.first { $0.0 == op }?.1 ?? ""
                let value = config.str("value") ?? ""
                VStack(alignment: .leading, spacing: 4) {
                    Text("EVALUATES").font(.system(size: 11)).tracking(0.6).foregroundStyle(Theme.gray500)
                    Text("\(variable) \(label)" + (!hidesValue && !value.isEmpty ? " \"\(value)\"" : ""))
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(Theme.gray400)
                        .lineLimit(1)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Theme.white(0.02), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.04), lineWidth: 1))
            }
        }
    }
}

// MARK: Loop

/// LoopConfigForm.tsx: how the loop repeats and when it stops; its body is drawn on the canvas.
struct LoopConfigForm: View {
    @Binding var config: JSONValue
    let step: StepContext

    static let operators: [(String, String)] = [
        ("equals", "equals"), ("notEquals", "does not equal"), ("contains", "contains"),
        ("notContains", "does not contain"), ("isEmpty", "is empty"), ("isNotEmpty", "is not empty"),
    ]

    var body: some View {
        let mode = config.str("mode") ?? "repeat"
        let until = config["until"]
        VStack(alignment: .leading, spacing: 20) {
            FormSection("Repeat") {
                HStack(spacing: 2) {
                    ForEach([("repeat", "A number of times"), ("forEach", "For each item in a list")], id: \.0) { value, label in
                        Hovering { hovered in
                            Button {
                                config.set("mode", string: value)
                            } label: {
                                Text(label)
                                    .font(.system(size: 12))
                                    .foregroundStyle(mode == value ? Color.white : hovered ? Theme.gray200 : Theme.gray400)
                                    .frame(maxWidth: .infinity, minHeight: 18)
                                    .padding(.horizontal, 10)
                                    .padding(.vertical, 6)
                                    .background(mode == value ? Theme.white(0.08) : .clear, in: RoundedRectangle(cornerRadius: Theme.radius))
                                    .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
                .padding(2)
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.08), lineWidth: 1))
            }
            if mode == "forEach" {
                FormSection(
                    "Items",
                    hint: "A list from an earlier step: a JSON array, or an object holding one. Each pass reads its item as {{loop.item}}, and every item gets a pass."
                ) {
                    VariableAutocomplete(
                        value: config.str("items") ?? "", onChange: { config.set("items", string: $0) },
                        placeholder: "{{steps.review.items}}", rows: 1, stepGroups: step.stepGroups,
                        contextVars: step.inputVars.filter { $0.category != .loop }, mono: true)
                    .padding(.bottom, 2)
                }
            } else {
                FormSection(
                    "Maximum passes",
                    hint: "This is what ends the loop, not a safety net: an agent asked whether its own work is good enough tends to say yes. Capped at \(StepEdits.maxIterations)."
                ) {
                    QuietField(
                        placeholder: "",
                        text: Binding(
                            get: { String(Int(config.num("maxIterations") ?? 1)) },
                            set: { config = StepEdits.loopMax(config, $0) }))
                    .padding(.bottom, 2)
                }
            }
            VStack(alignment: .leading, spacing: 0) {
                FormLabelTail(text: "Stop early when", tail: "(optional)")
                VStack(alignment: .leading, spacing: 8) {
                    VariableAutocomplete(
                        value: until?.str("variable") ?? "", onChange: { config = StepEdits.loopUntil(config, variable: $0) },
                        placeholder: "{{steps.review.approved}}", rows: 1, stepGroups: step.stepGroups,
                        contextVars: step.inputVars, mono: true)
                    HStack(spacing: 8) {
                        FormSelect(
                            value: until?.str("operator") ?? "equals", options: Self.operators.map { SelectOption($0.0, $0.1) },
                            tone: .quiet
                        ) { config = StepEdits.loopUntil(config, op: $0) }
                        .frame(width: 145)
                        QuietField(
                            placeholder: "true",
                            text: Binding(get: { until?.str("value") ?? "" }, set: { config = StepEdits.loopUntil(config, value: $0) }))
                    }
                }
                FormHint(
                    "Checked after each pass, against a step's typed output. "
                        + (mode == "forEach" ? "Leave blank to go through every item." : "Leave blank to always run the maximum."),
                    top: 6)
            }
        }
    }
}

// MARK: Approval

/// ApprovalConfigForm.tsx.
struct ApprovalConfigForm: View {
    @Binding var config: JSONValue
    let step: StepContext
    let slug: String?

    private func help(_ parts: [(String, Bool)]) -> some View {
        helpText(parts)
            .font(.system(size: 11))
            .foregroundStyle(Theme.gray500)
            .cssLineHeight(11, 1.45)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.top, 6)
    }

    var body: some View {
        let steps = StepEdits.approvalSteps(step.ancestors)
        let feedback = config["feedback"]?.objectValue != nil ? config["feedback"] : nil
        let gate = (slug ?? "").isEmpty ? "this_gate" : slug!
        VStack(alignment: .leading, spacing: 20) {
            VStack(alignment: .leading, spacing: 0) {
                FormLabel("Message")
                VariableAutocomplete(
                    value: config.str("message") ?? "", onChange: { config.set("message", string: $0) },
                    placeholder: "Shown with the answers, e.g. {{steps.draft.output}}", rows: 3, stepGroups: step.stepGroups,
                    contextVars: step.inputVars)
                help([("Step outputs fill in when the gate opens.", false)])
            }
            VStack(alignment: .leading, spacing: 0) {
                FormLabelTail(text: "Editable text", tail: "(optional)")
                VariableAutocomplete(
                    value: config.str("edit") ?? "", onChange: { config = StepEdits.emptyIsAbsent(config, "edit", $0) },
                    placeholder: "{{steps.draft.output}}", rows: 1, stepGroups: step.stepGroups, contextVars: step.inputVars,
                    mono: true)
                help([
                    ("The reviewer may rewrite this before approving. Later steps read ", false),
                    ("{{steps.\(gate).text}}", true), (", which is the rewrite when there is one and the original otherwise.", false),
                ])
            }
            VStack(alignment: .leading, spacing: 0) {
                FormLabelTail(text: "Review page", tail: "(optional)")
                VariableAutocomplete(
                    value: config.str("view") ?? "", onChange: { config = StepEdits.emptyIsAbsent(config, "view", $0) },
                    placeholder: "{{steps.review_page.output}}", rows: 1, stepGroups: step.stepGroups,
                    contextVars: step.inputVars, mono: true)
                help([("HTML, or the path of a .html file. Opens beside the answers, sandboxed, with no network.", false)])
            }
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 12) {
                    Text("Reviewer can request changes")
                        .font(.system(size: 13, weight: .medium))
                        .foregroundStyle(Theme.gray400)
                    Spacer(minLength: 0)
                    ToggleSwitch(isOn: feedback != nil, disabled: feedback == nil && steps.isEmpty) { on in
                        config = on ? StepEdits.approvalFeedback(config, steps: steps) : config.merged(["feedback": nil])
                    }
                }
                if feedback == nil && steps.isEmpty {
                    help([("Add a step before this gate to send the work back to.", false)])
                }
                if let feedback {
                    let from = feedback.str("from") ?? ""
                    VStack(alignment: .leading, spacing: 10) {
                        HStack(spacing: 8) {
                            Text("Redo from").font(.system(size: 12.5)).foregroundStyle(Theme.gray400)
                            FormSelect(
                                value: from,
                                options: (steps.contains { $0.id == from } ? [] : [SelectOption(from, "Pick a step")])
                                    + steps.map { SelectOption($0.id, $0.label.isEmpty ? $0.id : $0.label) },
                                tone: .quiet
                            ) { config = StepEdits.approvalFeedback(config, steps: steps, from: $0) }
                        }
                        HStack(spacing: 8) {
                            Text("up to").font(.system(size: 12.5)).foregroundStyle(Theme.gray400)
                            QuietField(
                                placeholder: "",
                                text: Binding(
                                    get: { String(Int(feedback.num("maxRounds") ?? StepEdits.defaultGateRounds)) },
                                    set: { if let next = StepEdits.approvalRounds(config, steps: steps, $0) { config = next } }),
                                mono: true, size: 12, vertical: 6, center: true
                            )
                            .frame(width: 64)
                            Text("rounds").font(.system(size: 12.5)).foregroundStyle(Theme.gray400)
                        }
                        help([
                            ("Those steps run again with the comment, read as ", false), ("{{steps.\(gate).feedback}}", true),
                            (", then the gate asks again.", false),
                        ])
                    }
                    .padding(.top, 12)
                }
            }
            VStack(alignment: .leading, spacing: 0) {
                FormLabelTail(text: "Timeout", tail: "(seconds, optional)")
                QuietField(
                    placeholder: "Leave blank to wait forever",
                    text: Binding(get: { StepEdits.approvalTimeoutText(config) }, set: { config = StepEdits.approvalTimeout(config, $0) }))
                help([("Auto-rejects the gate if nobody answers in time. Each round starts a new timeout.", false)])
            }
        }
    }
}

// MARK: Create task

/// CreateTaskFromItemNodeForm.tsx.
struct CreateTaskFromItemNodeForm: View {
    let store: WorkflowsStore
    @Binding var config: JSONValue

    static let statuses: [(String, String)] = [
        ("todo", "To Do"), ("in_progress", "In Progress"), ("in_review", "In Review"), ("done", "Done"), ("cancelled", "Cancelled"),
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            FormSection("Project") {
                FormSelect(
                    value: config.str("project") ?? "",
                    options: [SelectOption("fromConnection", "From connection")] + store.projects.map { SelectOption($0.name, $0.name) }
                ) { config.set("project", string: $0) }
                FormHint("\"From connection\" uses the executionProject set on the source connection.", top: 6)
            }
            FormSection("Initial Status") {
                FormSelect(value: config.str("initialStatus") ?? "", options: Self.statuses.map { SelectOption($0.0, $0.1) }) {
                    config.set("initialStatus", string: $0)
                }
                FormHint("Applied on first import. Local status edits are never overwritten by re-sync.", top: 6)
            }
        }
    }
}

// MARK: Connector action

/// CallConnectorActionNodeForm.tsx.
struct CallConnectorActionNodeForm: View {
    let data: NodeFormsData
    @Binding var config: JSONValue
    let step: StepContext

    var body: some View {
        let connId = config.str("connectionId") ?? ""
        let actions = data.actions[connId] ?? []
        let selectedConn = data.connection(connId)
        let action = config.str("action") ?? ""
        let selected = actions.first { $0.type == action }
        let unbound = connId.isEmpty && !(config.str("connectorId") ?? "").isEmpty
        let fields: [ConnectorConfigField] =
            selected?.configFields
            ?? (unbound ? data.catalogActions[config.str("connectorId") ?? ""]?.first { $0.type == action }?.configFields ?? [] : [])
        let vars = StepEdits.plainContextVars(triggerType: step.triggerType, connectorItem: true, inputVars: step.inputVars)
        VStack(alignment: .leading, spacing: 20) {
            FormSection("Connection") {
                FormSelect(
                    value: connId,
                    options: data.connections.map {
                        SelectOption($0.id, $0.name, icon: AnyView(ConnectorMark(connectorId: $0.realConnectorId)))
                    },
                    placeholder: "Select a connection..."
                ) { config = StepEdits.connectorConnection(config, $0) }
                if unbound {
                    FormHint(
                        "\(config.str("actionLabel").flatMap { $0.isEmpty ? nil : $0 } ?? action) comes from a connector that is not installed yet — the workflow panel offers to install and connect it.",
                        top: 6)
                } else if data.connections.isEmpty {
                    FormHint("No connections yet. Add one from Settings › Connectors first.", top: 6)
                }
            }
            if selectedConn != nil {
                FormSection("Action") {
                    FormSelect(
                        value: action,
                        options: actions.map { SelectOption($0.type, $0.label, hint: $0.label != $0.type ? $0.type : nil) },
                        placeholder: actions.isEmpty ? "No actions available for this connection" : "Select an action..."
                    ) { config = StepEdits.connectorAction(config, $0, actions: actions) }
                    if let d = selected?.description, !d.isEmpty { FormHint(d, top: 6) }
                }
            }
            if !fields.isEmpty {
                VStack(alignment: .leading, spacing: 12) {
                    (Text("ARGUMENTS").tracking(0.6) + Text(" · type ").foregroundColor(Theme.gray600)
                        + Text("{{").font(.system(size: 11, design: .monospaced)).foregroundColor(Theme.gray500)
                        + Text(" to pick from previous steps").foregroundColor(Theme.gray600))
                        .font(.system(size: 11))
                        .foregroundStyle(Theme.gray500)
                    ForEach(fields, id: \.key) { field in
                        VStack(alignment: .leading, spacing: 0) {
                            (Text(field.label) + (field.required == true ? Text(" *").foregroundColor(Theme.danger) : Text("")))
                                .font(.system(size: 12))
                                .foregroundStyle(Theme.gray500)
                                .padding(.bottom, 4)
                            ArgumentField(
                                field: field, value: config["args"]?[field.key]?.stringValue ?? "",
                                stepGroups: step.stepGroups, contextVars: vars
                            ) { config = StepEdits.connectorArg(config, field.key, $0) }
                            if let d = field.description, !d.isEmpty {
                                Text(d).font(.system(size: 10)).foregroundStyle(Theme.gray600).padding(.top, 2)
                            }
                        }
                    }
                }
            }
        }
        .task(id: connId) { data.ensureActions(connId) }
        .task(id: unbound) { if unbound { data.ensureCatalog() } }
    }
}

/// A picker while the value is one of the listed choices, the template input otherwise.
private struct ArgumentField: View {
    let field: ConnectorConfigField
    let value: String
    let stepGroups: [StepVariableGroup]
    let contextVars: [TemplateVariable]
    let onChange: (String) -> Void
    @State private var chosenFree = false
    @State private var parked = ""

    var body: some View {
        let choices = field.options ?? []
        let listed = choices.contains { $0.value == value }
        let free = chosenFree || (!value.isEmpty && !listed)
        let pickable = field.type == "select" && !choices.isEmpty
        VStack(alignment: .leading, spacing: 0) {
            if pickable && !free {
                FormSelect(
                    value: value, options: choices.map { SelectOption($0.value, $0.label) }, placeholder: field.placeholder ?? "—",
                    onChange: onChange)
                link("Use a template instead") {
                    chosenFree = true
                    if !parked.isEmpty { onChange(parked) }
                }
            } else {
                VariableAutocomplete(
                    value: value, onChange: onChange, placeholder: field.placeholder ?? "", rows: field.type == "textarea" ? 3 : 1,
                    stepGroups: stepGroups, contextVars: contextVars, mono: true)
                if pickable {
                    link("Choose from the list") {
                        chosenFree = false
                        if !listed {
                            parked = value
                            onChange("")
                        }
                    }
                }
            }
        }
    }

    private func link(_ title: String, action: @escaping () -> Void) -> some View {
        Hovering { hovered in
            Button(action: action) {
                Text(title).font(.system(size: 10)).foregroundStyle(hovered ? Theme.gray300 : Theme.gray500)
            }
            .buttonStyle(.plain)
        }
        .padding(.top, 4)
    }
}
