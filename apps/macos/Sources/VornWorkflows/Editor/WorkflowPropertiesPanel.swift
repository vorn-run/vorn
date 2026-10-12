import SwiftUI
import VornCore
import VornUI

/// WorkflowPropertiesPanel.tsx: the workflow's own settings, edited on the draft.
struct WorkflowPropertiesPanel: View {
    let store: WorkflowsStore
    @Binding var workflow: WorkflowDefinition
    let onClose: () -> Void
    /// Selects the trigger step on the canvas.
    var onSelectTrigger: (() -> Void)? = nil

    @State private var staggerText: String?

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Properties").font(.system(size: 13, weight: .medium)).foregroundStyle(.white)
                Spacer()
                PanelCloseButton(help: "Close properties panel", action: onClose)
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 16)
            .overlay(alignment: .bottom) { Rectangle().fill(Theme.white(0.08)).frame(height: 1) }

            ScrollView {
                VStack(alignment: .leading, spacing: 4) {
                    PropertyRow(label: "Status") {
                        HStack(spacing: 8) {
                            Text(workflow.enabled ? "Enabled" : "Disabled")
                            PanelToggle(isOn: workflow.enabled) { workflow.enabled = $0 }
                        }
                    }
                    PropertyRow(label: "Stagger delay") { staggerField }
                    PropertyRow(label: "Cleanup worktrees") {
                        let disabled = Self.cleanupDisabled(workflow.nodes)
                        PanelToggle(isOn: disabled ? false : workflow.autoCleanupWorktrees ?? false) {
                            if !disabled { workflow.autoCleanupWorktrees = $0 }
                        }
                        .opacity(disabled ? 0.5 : 1)
                        .help(disabled
                            ? "Cleanup applies only to worktrees this workflow creates. All agents inherit their worktree from context, so there is nothing to clean up."
                            : "")
                    }
                    Color.clear.frame(height: 4)
                    PropertyRow(label: "Trigger") {
                        Hovering { hovered in
                            Button { onSelectTrigger?() } label: {
                                Text(Self.triggerSummary(workflow.triggerNode))
                                    .foregroundStyle(hovered ? Color.white : Theme.gray200)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    if let lastRun = store.workflowRuns.first {
                        PropertyRow(label: "Last run") {
                            HStack(spacing: 8) {
                                StatusDot(status: WorkflowStatusKey(lastRun.status), size: 6)
                                Text(lastRun.startedAt.isEmpty ? "Unknown" : TimeFormat.relative(lastRun.startedAt))
                                    .foregroundStyle(Theme.gray300)
                                    .help(Self.localeString(lastRun.startedAt) ?? "")
                            }
                        }
                    }
                }
                .padding(20)
            }
        }
        .frame(width: 340)
        .frame(maxHeight: .infinity, alignment: .top)
        .background(Theme.surfaceNode)
        .overlay(alignment: .leading) { Rectangle().fill(Theme.white(0.08)).frame(width: 1) }
    }

    private var staggerField: some View {
        let text = Binding(
            get: { staggerText ?? workflow.staggerDelayMs.map { OJSON.format($0) } ?? "" },
            set: { raw in
                staggerText = raw
                switch Self.parseStagger(raw) {
                case .clear: workflow.staggerDelayMs = nil
                case .set(let ms): workflow.staggerDelayMs = ms
                case .ignore: break
                }
            })
        return TextField("", text: text, prompt: Text("0ms").foregroundStyle(Theme.gray600))
            .textFieldStyle(.plain)
            .font(.system(size: 12))
            .foregroundStyle(Theme.gray200)
            .padding(.horizontal, 8)
            .padding(.vertical, 2)
            .frame(width: 80)
            .background(Theme.white(0.06), in: RoundedRectangle(cornerRadius: Theme.radius))
            .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.08)))
            .accessibilityLabel("Stagger delay in milliseconds")
    }

    enum StaggerInput: Equatable { case clear, set(Double), ignore }

    /// The input's onChange: empty clears it, a non-negative `parseInt` sets it, anything else is ignored.
    static func parseStagger(_ raw: String) -> StaggerInput {
        if raw.isEmpty { return .clear }
        var rest = Substring(raw.drop { $0.isWhitespace })
        var digits = ""
        if rest.first == "+" || rest.first == "-" { digits.append(rest.removeFirst()) }
        digits += rest.prefix { $0.isASCII && $0.isNumber }
        guard let parsed = Double(digits), parsed >= 0 else { return .ignore }
        return .set(parsed + 0)
    }

    /// When every agent inherits its worktree from context there is nothing for cleanup to act on.
    static func cleanupDisabled(_ nodes: [WorkflowNode]) -> Bool {
        var anyCreates = false
        var anyInherits = false
        for node in nodes where node.type == .launchAgent {
            switch worktreeMode(node.config) {
            case "fromContext": anyInherits = true
            case "new": anyCreates = true
            default: break
            }
        }
        return anyInherits && !anyCreates
    }

    /// workflow-helpers.ts getWorktreeMode.
    static func worktreeMode(_ config: JSONValue) -> String {
        if config["useWorktree"]?.stringValue == "fromContext" { return "fromContext" }
        if let mode = config["worktreeMode"]?.stringValue { return mode }
        return config["useWorktree"]?.boolValue == true ? "new" : "none"
    }

    static func triggerSummary(_ node: WorkflowNode?) -> String {
        guard let node else { return "None" }
        switch node.config["triggerType"]?.stringValue {
        case "manual": return "Manual"
        case "once":
            let at = node.config["runAt"]?.stringValue ?? ""
            return "Once — \(at.isEmpty ? "not set" : localeString(at) ?? "Invalid Date")"
        case "recurring":
            let cron = node.config["cron"]?.stringValue ?? ""
            return "Recurring — \(cron.isEmpty ? "not set" : cron)"
        case "taskCreated": return "Task created"
        case "taskStatusChanged": return "Task status changed"
        default: return "Unknown"
        }
    }

    /// `new Date(value).toLocaleString()`; a datetime-local value without an offset reads as local time.
    static func localeString(_ value: String) -> String? {
        let local = DateFormatter()
        local.locale = Locale(identifier: "en_US_POSIX")
        local.dateFormat = value.count > 16 ? "yyyy-MM-dd'T'HH:mm:ss" : "yyyy-MM-dd'T'HH:mm"
        guard let date = TimeFormat.parse(value) ?? local.date(from: value) else { return nil }
        return date.formatted(date: .numeric, time: .standard)
    }
}

/// PropertyRow.tsx: a 110pt label column beside the value.
private struct PropertyRow<Content: View>: View {
    let label: String
    @ViewBuilder let content: Content

    var body: some View {
        HStack(spacing: 0) {
            Text(label).foregroundStyle(Theme.gray500).frame(width: 110, alignment: .leading)
            content.foregroundStyle(Theme.gray200).frame(maxWidth: .infinity, alignment: .leading)
        }
        .font(.system(size: 12))
        .frame(minHeight: 32)
        .padding(.vertical, 4)
    }
}

/// settings/ToggleSwitch.tsx: 40×24, blue when on, a 16pt knob.
struct PanelToggle: View {
    let isOn: Bool
    let onChange: (Bool) -> Void

    var body: some View {
        Button { onChange(!isOn) } label: {
            Capsule()
                .fill(isOn ? Theme.blue500 : Theme.white(0.1))
                .frame(width: 40, height: 24)
                .overlay(alignment: .leading) {
                    Circle().fill(Color.white).frame(width: 16, height: 16).offset(x: isOn ? 20 : 4)
                }
                .animation(.easeOut(duration: 0.15), value: isOn)
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(.isToggle)
        .accessibilityValue(isOn ? "On" : "Off")
    }
}

/// The panels' X: gray until hovered.
struct PanelCloseButton: View {
    let help: String
    let action: () -> Void

    var body: some View {
        Hovering { hovered in
            Button(action: action) {
                Icon(.x, size: 14)
                    .foregroundStyle(hovered ? Color.white : Theme.gray500)
                    .padding(4)
                    .background(hovered ? Theme.white(0.06) : .clear, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(help)
            .accessibilityLabel(help)
        }
    }
}
