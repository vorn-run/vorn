import SwiftUI
import VornCore
import VornUI

/// SourcePromptDialog.tsx: asks for a contextual run's source and the trigger's inputs, then runs.
struct RunPromptView: View {
    let store: WorkflowsStore
    @Bindable var prompt: RunPrompt
    @State private var openPicker: String?
    @FocusState private var focused: Bool

    var body: some View {
        RunModal(backdrop: 0.4, onDismiss: store.cancelPrompt) {
            VStack(alignment: .leading, spacing: 0) {
                header
                Hairline(opacity: 0.06)
                fields
                Hairline(opacity: 0.06)
                footer
            }
            .frame(width: 460)
            .modalCard(Theme.surfaceOverlay)
            .focusable()
            .focusEffectDisabled()
            .focused($focused)
            .onAppear { focused = true }
            .onKeyPress(.escape) {
                if openPicker != nil { openPicker = nil } else { store.cancelPrompt() }
                return .handled
            }
            .onKeyPress(.return) {
                // Only with no field focused, so a textarea's Return stays a newline.
                guard focused else { return .ignored }
                store.submitPrompt()
                return .handled
            }
            .onExitCommand(perform: store.cancelPrompt)
            .onSubmit { store.submitPrompt() }
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                LucideIcon(.zap, size: 14).foregroundStyle(Theme.blue400)
                Text("Run \"\(prompt.workflow.name)\"")
            }
            .frame(height: 22.5)
            .font(.system(size: 15, weight: .medium))
            .foregroundStyle(Color.white)
            Text(
                prompt.contextual
                    ? "This contextual workflow needs a source. Pick the folder it should run against."
                    : "This workflow needs a few details before it runs."
            )
            .font(.system(size: 12))
            .lineSpacing(3.5)
            .foregroundStyle(Theme.gray500)
            .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 16)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var fields: some View {
        VStack(alignment: .leading, spacing: 16) {
            if prompt.required.needsProject {
                VStack(alignment: .leading, spacing: 8) {
                    FieldLabel(text: "Project")
                    FormDropdown(
                        options: FormOption.projects(prompt.projects, allowNone: false), value: prompt.projectName,
                        placeholder: "Select project...", showsProjectIcon: true,
                        onSelect: prompt.selectProject, open: pickerBinding("__project"))
                }
                .zIndex(openPicker == "__project" ? 10 : 0)
            }
            if prompt.required.needsBranch {
                VStack(alignment: .leading, spacing: 8) {
                    FieldLabel(text: "Branch")
                    BranchField(text: $prompt.branch, placeholder: "leave blank to use the project's current branch")
                }
            }
            if prompt.required.needsWorktree {
                FormCheckbox(checked: prompt.useWorktree, label: "Run in a new worktree") { prompt.useWorktree.toggle() }
            }
            RunInputFields(
                defs: prompt.defs, values: $prompt.values, projects: prompt.projects, openPicker: $openPicker)
        }
        .padding(24)
        .zIndex(1)
    }

    private var footer: some View {
        HStack(spacing: 8) {
            Spacer(minLength: 0)
            Hovering { hovered in
                Button(action: store.cancelPrompt) {
                    Text("Cancel")
                        .font(.system(size: 13))
                        .frame(height: 19.5)
                        .foregroundStyle(hovered ? Color.white : Theme.gray300)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(hovered ? Theme.white(0.06) : .clear, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            Hovering { hovered in
                Button(action: store.submitPrompt) {
                    Text("Run")
                        .font(.system(size: 13))
                        .frame(height: 19.5)
                        .foregroundStyle(Color.white)
                        .padding(.horizontal, 16)
                        .padding(.vertical, 6)
                        .background(
                            Theme.white(hovered && prompt.canSubmit ? 0.15 : 0.1),
                            in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(!prompt.canSubmit)
                .opacity(prompt.canSubmit ? 1 : 0.4)
            }
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 12)
    }

    private func pickerBinding(_ key: String) -> Binding<Bool> {
        Binding(get: { openPicker == key }, set: { openPicker = $0 ? key : nil })
    }
}

/// `text-[11px] font-medium text-gray-500 uppercase tracking-wider`, with a red star when required.
struct FieldLabel: View {
    let text: String
    var required = false

    var body: some View {
        HStack(spacing: 4) {
            Text(text.uppercased()).tracking(0.55)
            if required { Text("*").foregroundStyle(Theme.danger) }
        }
        .font(.system(size: 11, weight: .medium))
        .frame(height: 16.5)
        .foregroundStyle(Theme.gray500)
    }
}

/// The GitBranch-prefixed text field.
struct BranchField: View {
    @Binding var text: String
    let placeholder: String

    var body: some View {
        HStack(spacing: 6) {
            LucideIcon(.gitBranch, size: 12).foregroundStyle(Theme.gray500)
            BareTextInput(placeholder: placeholder, text: $text)
        }
        .formInput()
    }
}

/// WorkflowInputFields.tsx: one field per declared input, by kind.
struct RunInputFields: View {
    let defs: [WorkflowInputDef]
    @Binding var values: [String: JSONValue]
    let projects: [Project]
    @Binding var openPicker: String?

    var body: some View {
        ForEach(defs, id: \.key) { def in
            VStack(alignment: .leading, spacing: 0) {
                if def.type != .boolean {
                    FieldLabel(text: def.displayLabel, required: def.required).padding(.bottom, 8)
                }
                field(def)
                if let d = def.description, !d.isEmpty {
                    Text(d)
                        .font(.system(size: 11))
                        .foregroundStyle(Theme.gray500)
                        .fixedSize(horizontal: false, vertical: true)
                        // A textarea sits on the text baseline, which leaves a gap under it.
                        .padding(.top, def.type == .textarea ? 9 : 4)
                }
            }
            .zIndex(openPicker == def.key ? 10 : 0)
        }
    }

    @ViewBuilder private func field(_ def: WorkflowInputDef) -> some View {
        let text = Binding(
            get: { values[def.key]?.stringValue ?? "" }, set: { values[def.key] = .string($0) })
        switch def.type {
        case .textarea:
            ZStack(alignment: .topLeading) {
                if text.wrappedValue.isEmpty, let p = def.placeholder {
                    Text(p).foregroundStyle(Theme.gray600).padding(.leading, 5).allowsHitTesting(false)
                }
                TextEditor(text: text)
                    .scrollContentBackground(.hidden)
                    .frame(height: 3 * 19.5)
                    .accessibilityLabel(def.displayLabel)
            }
            .formInput()
        case .number:
            NumberInput(value: values[def.key], placeholder: def.placeholder ?? "") { values[def.key] = $0 }
                .accessibilityLabel(def.displayLabel)
        case .select:
            FormDropdown(
                options: def.options.map { FormOption(value: $0.value, label: $0.label) },
                value: text.wrappedValue,
                placeholder: (def.placeholder?.isEmpty == false ? def.placeholder : nil) ?? "Select...",
                onSelect: { text.wrappedValue = $0 }, open: picker(def.key))
        case .boolean:
            FormCheckbox(checked: values[def.key]?.boolValue == true, label: def.displayLabel) {
                values[def.key] = .bool(values[def.key]?.boolValue != true)
            }
        case .project:
            FormDropdown(
                options: FormOption.projects(projects, allowNone: !def.required), value: text.wrappedValue,
                placeholder: "Select project...", showsProjectIcon: true,
                onSelect: { text.wrappedValue = $0 }, open: picker(def.key))
        case .branch:
            BranchField(
                text: text, placeholder: (def.placeholder?.isEmpty == false ? def.placeholder : nil) ?? "branch name")
                .accessibilityLabel(def.displayLabel)
        case .text:
            BareTextInput(placeholder: def.placeholder ?? "", text: text)
                .formInput()
                .accessibilityLabel(def.displayLabel)
        }
    }

    private func picker(_ key: String) -> Binding<Bool> {
        Binding(get: { openPicker == key }, set: { openPicker = $0 ? key : nil })
    }
}

/// `<input type="number">`: keeps what was typed, stores parseNumberInput of it.
private struct NumberInput: View {
    let placeholder: String
    let onChange: (JSONValue) -> Void
    @State private var text: String

    init(value: JSONValue?, placeholder: String, onChange: @escaping (JSONValue) -> Void) {
        self.placeholder = placeholder
        self.onChange = onChange
        if case .number(let n)? = value {
            _text = State(initialValue: OJSON.format(n))
        } else {
            _text = State(initialValue: "")
        }
    }

    var body: some View {
        BareTextInput(placeholder: placeholder, text: $text)
            .formInput()
            .onChange(of: text) { _, t in onChange(RunInputs.parseNumber(t)) }
    }
}

extension View {
    /// Draws the run prompt, a gate's review page and the missed-schedules dialog over the window, and follows `scheduler:missed`.
    /// Apply once at the window root, as App.tsx mounts SourcePromptDialog and MissedScheduleDialog.
    public func runSurfacesHost(_ store: WorkflowsStore) -> some View {
        modifier(RunSurfacesHost(store: store))
    }
}

private struct RunSurfacesHost: ViewModifier {
    let store: WorkflowsStore

    func body(content: Content) -> some View {
        content
            .blur(radius: store.surfaces.review == nil ? 0 : 4)
            .overlay {
                if let prompt = store.surfaces.prompt {
                    RunPromptView(store: store, prompt: prompt).id(ObjectIdentifier(prompt))
                }
            }
            .overlay { if !store.surfaces.missed.isEmpty { MissedSchedulesView(store: store) } }
            .overlay {
                if let review = store.surfaces.review { GateReviewModal(store: store, target: review).id(review.runId + review.nodeId) }
            }
            .overlay {
                if let logs = store.surfaces.fullOutput {
                    LogReplayModal(logs: logs) { store.surfaces.fullOutput = nil }
                }
            }
            .task { await store.followMissedSchedules() }
    }
}
