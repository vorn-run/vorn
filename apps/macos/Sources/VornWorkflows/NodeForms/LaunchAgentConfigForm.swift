import SwiftUI
import VornCore
import VornUI

/// LaunchAgentConfigForm.tsx.
struct LaunchAgentConfigForm: View {
    let store: WorkflowsStore
    let data: NodeFormsData
    @Binding var config: JSONValue
    let step: StepContext
    let nodeId: String
    let nodes: [WorkflowNode]
    @State private var advancedOpen: Bool

    init(
        store: WorkflowsStore, data: NodeFormsData, config: Binding<JSONValue>, step: StepContext, nodeId: String,
        nodes: [WorkflowNode]
    ) {
        self.store = store
        self.data = data
        _config = config
        self.step = step
        self.nodeId = nodeId
        self.nodes = nodes
        _advancedOpen = State(initialValue: !(config.wrappedValue["args"]?.arrayValue ?? []).isEmpty)
    }

    private var project: Project? { store.projects.first { $0.name == config.str("projectName") } }
    private var isRemote: Bool { StepEdits.agentIsRemote(project) }
    private var projectPath: String { config.str("projectPath") ?? "" }
    private var projectFromContext: Bool {
        TemplateVars.isContextRef(config.str("projectName")) || TemplateVars.isContextRef(config.str("projectPath"))
    }
    private var promptSource: String { StepEdits.agentPromptSource(config) }
    private var canUseFromTask: Bool { step.isTaskTrigger || promptSource != "inline" }
    private var isGitRepo: Bool { projectPath.isEmpty || isRemote ? true : data.gitRepo[projectPath] ?? true }

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            FormSection("Agent") {
                FormAgentPicker(current: config.str("agentType") ?? "", data: data, allowFromTask: canUseFromTask) {
                    config = StepEdits.agentSetAgent(config, $0)
                }
            }
            FormSection("Model") {
                FormModelPicker(
                    agent: config.str("agentType") ?? "", projectPath: projectFromContext ? nil : projectPath,
                    remoteHostId: project.map(FormHelpers.remoteHostId) ?? config.str("remoteHostId"),
                    value: config.str("model"), data: data
                ) { config.set("model", string: $0) }
            }
            FormSection("Project") {
                FormProjectPicker(
                    current: projectFromContext ? "" : config.str("projectName") ?? "", projects: store.projects,
                    isFromContext: projectFromContext,
                    onSelectFromContext: step.isContextualTrigger ? { config = StepEdits.agentProjectFromContext(config) } : nil
                ) { name in
                    if let p = store.projects.first(where: { $0.name == name }) { config = StepEdits.agentProject(config, p) }
                }
            }
            promptSection
            if !isRemote && isGitRepo { gitSection }
            executionSection
            AdvancedSection(open: $advancedOpen) {
                FormSection("Extra Arguments") {
                    FormTextField(
                        placeholder: "e.g. --dangerously-skip-permissions",
                        text: Binding(
                            get: { (config["args"]?.arrayValue ?? []).compactMap(\.stringValue).joined(separator: " ") },
                            set: { config = StepEdits.agentArgs(config, $0) }),
                        mono: true)
                }
            }
        }
        .onAppear {
            if let next = StepEdits.agentFixFromTask(config, canUseFromTask: canUseFromTask, defaultAgent: data.defaults.defaultAgent) {
                config = next
            }
            if let next = StepEdits.agentClearContext(config, contextual: step.isContextualTrigger) { config = next }
        }
        .onChange(of: canUseFromTask) { _, can in
            if let next = StepEdits.agentFixFromTask(config, canUseFromTask: can, defaultAgent: data.defaults.defaultAgent) {
                config = next
            }
        }
        .task(id: projectPath) { if !isRemote { data.ensureGit(projectPath) } }
        .task(id: config.str("projectName") ?? "") { data.ensureTasks(config.str("projectName") ?? "") }
    }

    private var projectTasks: [FormTask] { data.todoTasks[config.str("projectName") ?? ""] ?? [] }

    @ViewBuilder private var promptSection: some View {
        VStack(alignment: .leading, spacing: 0) {
            FormLabel("Prompt")
            FormSelect(
                value: promptSource,
                options: [SelectOption("inline", "Inline"), SelectOption("task", "Task"), SelectOption("queue", "Queue")]
            ) { config = StepEdits.agentPromptSource(config, $0, firstTaskId: projectTasks.first?.id) }
            .padding(.bottom, 8)
            switch promptSource {
            case "inline":
                let binding = Binding(get: { config.str("prompt") ?? "" }, set: { config = StepEdits.emptyIsAbsent(config, "prompt", $0) })
                if StepEdits.hasTemplateVars(step) {
                    VariableAutocomplete(
                        value: binding.wrappedValue, onChange: { binding.wrappedValue = $0 }, placeholder: "Enter prompt...",
                        rows: 4, stepGroups: step.stepGroups, contextVars: StepEdits.agentContextVars(step))
                } else {
                    RichMarkdownEditor(value: binding, placeholder: "Enter prompt...").frame(minHeight: 100)
                }
            case "task":
                FormSelect(
                    value: config.str("taskId") ?? "",
                    options: [SelectOption("", "Select task...")] + projectTasks.map { SelectOption($0.id, $0.title) },
                    placeholder: "Select task..."
                ) { config = StepEdits.emptyIsAbsent(config, "taskId", $0) }
            default:
                (Text("Auto-picks the next todo task from ")
                    + Text((config.str("projectName") ?? "").isEmpty ? "the project" : config.str("projectName")!)
                    .foregroundColor(Theme.gray400) + Text(" at runtime."))
                    .font(.system(size: 11))
                    .foregroundStyle(Theme.gray500)
                    .cssLineHeight(11, 1.625)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private static let worktreeHints: [String: String] = [
        "none": "Agent runs in the project directory",
        "new": "Isolated directory — won't affect the main working tree",
        "fromStep": "Reuses the worktree created by a previous step",
        "existing": "Launches into an existing worktree on disk",
        "fromContext": "Inherits the launching card or terminal's worktree (won't be auto-cleaned)",
    ]

    private var gitSection: some View {
        let branchFromContext = TemplateVars.isContextRef(config.str("branch"))
        let mode = FormHelpers.worktreeMode(config)
        let hasBranch = StepEdits.agentHasBranch(config, remote: isRemote)
        let prior = StepEdits.priorWorktreeSteps(nodes, current: nodeId)
        var modes = [
            SelectOption("none", "None"), SelectOption("new", "New worktree", hint: hasBranch ? nil : "needs branch"),
            SelectOption("fromStep", "From step", hint: prior.isEmpty ? "no steps" : nil),
            SelectOption("existing", "Existing worktree"),
        ]
        if step.isContextualTrigger { modes.append(SelectOption("fromContext", "From Context")) }
        return FormBox {
            HStack(spacing: 6) {
                LucideIcon(.gitBranch, size: 11)
                Text("Git & Branch").font(.system(size: 13, weight: .medium)).cssLineHeight(13)
            }
            .foregroundStyle(Theme.gray400)
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 6) {
                    if branchFromContext {
                        LucideIcon(.zap, size: 12).foregroundStyle(Theme.ink)
                        Text("From Context").font(.system(size: 13)).foregroundStyle(Theme.gray200)
                            .frame(maxWidth: .infinity, minHeight: 19.5, alignment: .leading)
                    } else {
                        LucideIcon(.gitBranch, size: 12, strokeWidth: 2).foregroundStyle(Theme.gray500)
                        BareTextInput(
                            placeholder: "feature/my-branch",
                            text: Binding(get: { config.str("branch") ?? "" }, set: { config = StepEdits.agentBranch(config, $0) }))
                    }
                    if step.isContextualTrigger {
                        Hovering { hovered in
                            Button {
                                config = StepEdits.agentToggleBranchContext(config)
                            } label: {
                                LucideIcon(.zap, size: 11)
                                    .foregroundStyle(branchFromContext ? Theme.ink : hovered ? Theme.gray300 : Theme.gray500)
                                    .padding(2)
                                    .background(hovered ? Theme.white(0.08) : .clear, in: RoundedRectangle(cornerRadius: Theme.radius))
                                    .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .help(branchFromContext ? "Use a specific branch" : "Use From Context")
                        }
                    }
                }
                .fieldChrome()
                FormHint("Checks out this branch before launching")
            }
            VStack(alignment: .leading, spacing: 0) {
                Text("Worktree").font(.system(size: 11)).foregroundStyle(Theme.gray500).cssLineHeight(11).padding(.bottom, 6)
                FormSelect(value: mode, options: modes) { key in
                    if let next = StepEdits.agentWorktreeMode(config, key, hasBranch: hasBranch, priorSteps: prior.count) {
                        config = next
                    }
                }
                FormHint(Self.worktreeHints[mode] ?? "", top: 6)
            }
            if mode == "fromStep" {
                FormSelect(
                    value: config.str("worktreeFromStepSlug") ?? "",
                    options: [SelectOption("", "Select step...")]
                        + prior.map { n in
                            let slug = n.slug.flatMap { $0.isEmpty ? nil : $0 }
                            return SelectOption(slug ?? n.id, n.label.isEmpty ? slug ?? n.id : n.label)
                        },
                    placeholder: "Select step..."
                ) { config = StepEdits.emptyIsAbsent(config, "worktreeFromStepSlug", $0) }
            }
            if mode == "existing" {
                FormSelect(
                    value: config.str("existingWorktreePath") ?? "",
                    options: [SelectOption("", "Select worktree...")]
                        + (data.worktrees[projectPath] ?? []).map { SelectOption($0.path, $0.name, hint: $0.branch) },
                    placeholder: "Select worktree..."
                ) { config = StepEdits.emptyIsAbsent(config, "existingWorktreePath", $0) }
            }
        }
    }

    private var executionSection: some View {
        let headless = config.bool("headless") == true
        let fallback = data.defaults.headlessStepTimeoutMinutes
        let fallbackText = fallback == fallback.rounded() ? String(Int(fallback)) : String(fallback)
        return FormBox {
            Text("Execution").font(.system(size: 13, weight: .medium)).foregroundStyle(Theme.gray400).cssLineHeight(13)
            SwitchCard(
                isOn: headless, symbol: .eyeOff, title: "Headless",
                detail: headless
                    ? "Runs in background. Waits for completion before next step."
                    : "Opens a terminal tab. Step completes immediately.",
                disabled: isRemote
            ) { config = StepEdits.agentToggleHeadless(config) }
            if !headless {
                FormSection("Tab Name") {
                    FormTextField(
                        placeholder: (config.str("projectName") ?? "").isEmpty ? "Uses project name" : config.str("projectName")!,
                        text: Binding(
                            get: { config.str("displayName") ?? "" }, set: { config = StepEdits.emptyIsAbsent(config, "displayName", $0) }))
                }
            } else {
                OutputSchemaField(value: config["outputSchema"]) { config = config.merged(["outputSchema": $0]) }
                FormSection("Timeout") {
                    HStack(spacing: 8) {
                        FormTextField(
                            placeholder: fallbackText,
                            text: Binding(
                                get: { StepEdits.agentTimeoutText(config) },
                                set: { if let next = StepEdits.agentTimeout(config, $0) { config = next } }))
                        .frame(width: 96)
                        Text("minutes").font(.system(size: 12)).foregroundStyle(Theme.gray500)
                    }
                    FormHint(
                        config.num("timeoutMs") == 0
                            ? "No limit. A step whose agent never exits keeps its run open indefinitely."
                            : "Kills the agent and fails the step if it hasn't exited by then, so one stuck agent can't hold the run open. Blank uses the \(fallbackText)-minute default; 0 means no limit.",
                        top: 6)
                }
            }
        }
    }
}

/// OutputSchemaField: raw text kept locally; only a parsed object reaches the config.
private struct OutputSchemaField: View {
    let value: JSONValue?
    let onChange: (JSONValue?) -> Void
    @State private var text: String
    @State private var error: String?
    @State private var lastEmitted: String?

    init(value: JSONValue?, onChange: @escaping (JSONValue?) -> Void) {
        self.value = value
        self.onChange = onChange
        _text = State(initialValue: value?.prettyJSONText ?? "")
        _lastEmitted = State(initialValue: value?.jsonText)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            FormLabel("Output Schema")
            FormTextArea(
                placeholder: "{\n  \"type\": \"object\",\n  \"properties\": { \"verdict\": { \"type\": \"string\" } },\n  \"required\": [\"verdict\"]\n}",
                text: Binding(get: { text }, set: edit), rows: 6, mono: true, size: 12, invalid: error != nil)
            if let error {
                Text(error).font(.system(size: 11)).foregroundStyle(Theme.danger).padding(.top, 4)
            } else {
                helpText([
                    ("Agent must return a JSON object matching this schema. Its fields become ", false),
                    ("{{steps.<step>.<field>}}", true),
                    (" for later nodes. A run that doesn't match is marked failed.", false),
                ])
                .font(.system(size: 11))
                .foregroundStyle(Theme.gray500)
                .cssLineHeight(11, 1.625)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, 4)
            }
        }
        .onChange(of: value?.jsonText) { _, incoming in
            if incoming != lastEmitted {
                text = value?.prettyJSONText ?? ""
                error = nil
                lastEmitted = incoming
            }
        }
    }

    private func edit(_ raw: String) {
        text = raw
        switch StepEdits.parseSchema(raw) {
        case .error(let e): error = e
        case .set(let v):
            error = nil
            lastEmitted = v?.jsonText
            onChange(v)
        }
    }
}

// MARK: Agent picker

/// AgentPicker.tsx, the form variant: From Task when a task is in reach, every agent, uninstalled ones disabled.
struct FormAgentPicker: View {
    let current: String
    let data: NodeFormsData
    var allowFromTask = false
    let onChange: (String) -> Void
    @State private var open = false
    @Environment(\.formRenderOpen) private var renderOpen

    var body: some View {
        let kind = AgentKind(rawValue: current)
        Hovering { hovered in
            Button {
                open.toggle()
            } label: {
                HStack(spacing: 8) {
                    if current == "fromTask" {
                        LucideIcon(.clipboardList, size: 14).foregroundStyle(Theme.blue400)
                    } else if let kind {
                        AgentIcon(kind, size: 14)
                    } else {
                        LucideIcon(.bot, size: 14).foregroundStyle(Theme.gray500)
                    }
                    Text(current == "fromTask" ? "From Task" : kind?.label ?? "Unassigned")
                        .font(.system(size: 13))
                        .foregroundStyle(current.isEmpty ? Theme.gray600 : .white)
                        .frame(maxWidth: .infinity, minHeight: 19.5, alignment: .leading)
                    LucideIcon(.chevronDown, size: 11).foregroundStyle(Theme.gray500)
                }
                .fieldChrome(focused: hovered)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .formPopup(isPresented: $open) {
            DropdownPanel(verticalPadding: 4) {
                VStack(alignment: .leading, spacing: 0) {
                    if allowFromTask {
                        CheckTrailingRow(label: "From Task", selected: current == "fromTask") {
                            LucideIcon(.clipboardList, size: 14).foregroundStyle(Theme.blue400)
                        } action: {
                            select("fromTask")
                        }
                        .help("Resolved from the task's assigned agent at run time")
                        Hairline(opacity: 0.06).padding(.vertical, 4)
                    }
                    ForEach(AgentKind.allCases, id: \.self) { agent in
                        let installed = data.isInstalled(agent.rawValue)
                        CheckTrailingRow(
                            label: agent.label, selected: installed && agent.rawValue == current, disabled: !installed,
                            hint: installed ? nil : "Not installed"
                        ) {
                            AgentIcon(agent, size: 14)
                        } action: {
                            select(agent.rawValue)
                        }
                        .help(installed ? "" : "\(agent.rawValue) is not installed")
                    }
                }
            }
        }
        .onAppear { if renderOpen == "agent" { open = true } }
    }

    private func select(_ agent: String) {
        if agent != "fromTask" && !data.isInstalled(agent) { return }
        open = false
        if agent != current { onChange(agent) }
    }
}

// MARK: Model picker

/// ModelPicker.tsx, the form variant: the agent's models, the default, and a typed id.
struct FormModelPicker: View {
    let agent: String
    let projectPath: String?
    let remoteHostId: String?
    let value: String?
    let data: NodeFormsData
    let onChange: (String?) -> Void
    @State private var open = false
    @State private var query = ""
    @State private var invalid = ""
    @State private var openedAt = Date()
    @Environment(\.formRenderOpen) private var renderOpen

    private var followsTask: Bool { agent == "fromTask" }
    private var supported: Bool { followsTask || StepEdits.supportsModelSelection(agent) }
    private var key: String { NodeFormsData.modelKey(agent, projectPath ?? "") }
    private var catalog: AgentModelCatalog? { data.models[key] }
    private var loading: Bool { data.loadingModels.contains(key) }

    var body: some View {
        if supported {
            let selected = catalog?.choices.first { $0.id == value }
            let label = selected?.label ?? value
            let text = followsTask ? "Follows the task's agent" : label ?? "Agent default"
            Hovering { hovered in
                Button {
                    guard !followsTask else { return }
                    if !open {
                        query = ""
                        invalid = ""
                        openedAt = Date()
                        ask()
                    }
                    open.toggle()
                } label: {
                    HStack(spacing: 8) {
                        LucideIcon(.cpu, size: 14, strokeWidth: 1.75).foregroundStyle(label != nil ? Theme.gray300 : Theme.gray500)
                        Text(text)
                            .font(.system(size: 13))
                            .foregroundStyle(followsTask ? Theme.gray600 : label != nil ? .white : Theme.gray500)
                            .lineLimit(1)
                            .frame(maxWidth: 170, minHeight: 19.5, alignment: .leading)
                        if !followsTask { LucideIcon(.chevronDown, size: 12).foregroundStyle(Theme.gray500) }
                        Spacer(minLength: 0)
                    }
                    .fieldChrome(focused: hovered && !followsTask)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(followsTask)
            }
            .formPopup(isPresented: $open, minWidth: 288) { menu(selected: selected) }
            .task(id: "\(agent)|\(projectPath ?? "")|\(remoteHostId ?? "")") {
                if !followsTask, let p = projectPath, !p.isEmpty, remoteHostId == nil { data.ensureModels(agent: agent, projectPath: p) }
            }
            .onAppear { if renderOpen == "model" { open = true } }
        }
    }

    private func ask() {
        if remoteHostId == nil { data.ensureModels(agent: agent, projectPath: projectPath ?? "") }
    }

    private func select(_ model: String?) {
        open = false
        if model != value { onChange(model) }
    }

    private func providerOf(_ id: String) -> String? {
        guard let slash = id.firstIndex(of: "/"), slash != id.startIndex else { return nil }
        return String(id[..<slash])
    }

    private func footer() -> (text: String, spinner: Bool) {
        if loading { return catalog != nil ? ("Asking \(agent)…", true) : ("", false) }
        guard let catalog else { return ("", false) }
        if catalog.status == "unavailable" { return (catalog.error ?? "Could not list models — type an id", false) }
        var ago = ""
        if let at = catalog.fetchedAt {
            let minutes = Int((openedAt.timeIntervalSince1970 * 1000 - at) / 60_000 + 0.5)
            ago = minutes < 1 ? "just now" : "\(minutes) min ago"
        }
        return ("From \(agent) · \(ago)" + (catalog.status == "stale" ? " · refreshing failed" : ""), false)
    }

    private func menu(selected: AgentModelCatalog.Choice?) -> some View {
        let choices = catalog?.choices ?? []
        let needle = query.trimmingCharacters(in: .whitespaces).lowercased()
        let shown = needle.isEmpty ? choices : choices.filter { "\($0.label) \($0.id)".lowercased().contains(needle) }
        let exact = shown.first { $0.id.lowercased() == needle }
        let foot = footer()
        return VStack(alignment: .leading, spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    if loading && catalog == nil {
                        HStack(spacing: 8) {
                            LucideIcon(.loaderCircle, size: 11)
                            Text("Asking \(agent) for its models…")
                        }
                        .font(.system(size: 12)).foregroundStyle(Theme.gray500)
                        .padding(.horizontal, 12).padding(.vertical, 6)
                    }
                    if needle.isEmpty {
                        modelRow(label: "Agent default", id: nil, current: value == nil) { select(nil) }
                        if let value, selected == nil { modelRow(label: value, id: value, current: true) { select(value) } }
                    }
                    ForEach(Array(shown.enumerated()), id: \.element.id) { i, c in
                        let provider = agent == "opencode" ? providerOf(c.id) : nil
                        if let provider, i == 0 || providerOf(shown[i - 1].id) != provider {
                            Text(provider.uppercased())
                                .font(.system(size: 10)).tracking(0.5).foregroundStyle(Theme.gray600)
                                .padding(.horizontal, 12).padding(.top, 8).padding(.bottom, 2)
                        }
                        modelRow(label: c.label, id: c.id, current: c.id == value) { select(c.id) }
                            .help(c.description ?? "")
                    }
                    if !needle.isEmpty && shown.isEmpty {
                        Text("No listed model matches").font(.system(size: 12)).foregroundStyle(Theme.gray600)
                            .padding(.horizontal, 12).padding(.vertical, 6)
                    }
                }
                .padding(.vertical, 4)
            }
            .frame(maxHeight: 240)
            .fixedSize(horizontal: false, vertical: true)
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 8) {
                    Text("ID").font(.system(size: 10)).tracking(0.5).foregroundStyle(Theme.gray600)
                    BareTextInput(placeholder: "type a model id", text: $query, size: 11, mono: true, color: Theme.gray200)
                        .onSubmit {
                            if let exact { select(exact.id) } else if !needle.isEmpty {
                                if let id = StepEdits.validateModelId(query) { select(id) } else {
                                    invalid = "Enter a model id without spaces, control characters, or a leading dash."
                                }
                            }
                        }
                        .padding(.horizontal, 8).padding(.vertical, 2)
                        .background(Theme.white(0.04), in: RoundedRectangle(cornerRadius: Theme.radius))
                        .overlay(
                            RoundedRectangle(cornerRadius: Theme.radius)
                                .strokeBorder(invalid.isEmpty ? Theme.white(0.08) : Theme.danger, lineWidth: 1))
                }
                if !invalid.isEmpty { Text(invalid).font(.system(size: 11)).foregroundStyle(Theme.danger) }
            }
            .padding(.horizontal, 12).padding(.vertical, 8)
            .overlay(alignment: .top) { Hairline(opacity: 0.06) }
            HStack(spacing: 8) {
                if foot.spinner { LucideIcon(.loaderCircle, size: 10) }
                Text(foot.text).lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
                if let catalog, catalog.status != "unavailable", !loading {
                    Button {
                        data.ensureModels(agent: agent, projectPath: projectPath ?? "", refresh: true)
                    } label: {
                        LucideIcon(.refreshCw, size: 11).foregroundStyle(Theme.gray500).frame(width: 20, height: 20)
                    }
                    .buttonStyle(.plain)
                    .help("Ask \(agent) again")
                }
            }
            .font(.system(size: 10)).foregroundStyle(Theme.gray600)
            .padding(.horizontal, 12).padding(.bottom, 8)
        }
        .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusLg))
        .clipShape(RoundedRectangle(cornerRadius: Theme.radiusLg))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.08), lineWidth: 1))
    }

    private func modelRow(label: String, id: String?, current: Bool, action: @escaping () -> Void) -> some View {
        Hovering { hovered in
            Button(action: action) {
                HStack(spacing: 8) {
                    Group { if current { LucideIcon(.check, size: 11, strokeWidth: 3) } else { Color.clear } }.frame(width: 11, height: 11)
                    Text(label).lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
                    if let id, id != label {
                        Text(id).font(.system(size: 10, design: .monospaced)).foregroundStyle(Theme.gray600).lineLimit(1)
                            .frame(maxWidth: 120, alignment: .trailing)
                    }
                }
                .font(.system(size: 12))
                .foregroundStyle(current || hovered ? Color.white : Theme.gray300)
                .frame(minHeight: 18)
                .padding(.horizontal, 12).padding(.vertical, 6)
                .background(current ? Theme.white(0.06) : hovered ? Theme.white(0.04) : .clear)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }
}
