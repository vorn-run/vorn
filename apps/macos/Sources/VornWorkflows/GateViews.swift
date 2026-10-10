import SwiftUI
import VornCore
import VornUI

/// GATE_APPROVE / GATE_NEUTRAL / GATE_REJECT from gate-affordance.ts.
enum GateTone { case approve, neutral, reject }

struct GateButtonStyle: ButtonStyle {
    let tone: GateTone
    var horizontal: CGFloat = 16
    var vertical: CGFloat = 8

    func makeBody(configuration: Configuration) -> some View {
        StyledBody(configuration: configuration, tone: tone, horizontal: horizontal, vertical: vertical)
    }

    private struct StyledBody: View {
        let configuration: Configuration
        let tone: GateTone
        let horizontal: CGFloat
        let vertical: CGFloat
        @Environment(\.isEnabled) private var enabled
        @State private var hovered = false

        var body: some View {
            let hot = hovered && enabled
            configuration.label
                .padding(.horizontal, horizontal)
                .padding(.vertical, vertical)
                .foregroundStyle(foreground(hot))
                .background(background(hot), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                .overlay(
                    RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(border(hot), lineWidth: 1))
                .contentShape(RoundedRectangle(cornerRadius: Theme.radiusMd))
                .opacity(enabled ? 1 : 0.4)
                .onHover { hovered = $0 }
        }

        private func foreground(_ hot: Bool) -> Color {
            switch tone {
            case .approve: return Theme.bronzo
            case .neutral: return hot ? Theme.ink : Theme.inkSecondary
            case .reject: return hot ? Theme.danger : Theme.inkSecondary
            }
        }

        private func background(_ hot: Bool) -> Color {
            switch tone {
            case .approve: return Theme.bronzo.opacity(hot ? 0.2 : 0.1)
            case .neutral: return hot ? Theme.white(0.04) : .clear
            case .reject: return hot ? Theme.danger.opacity(0.1) : .clear
            }
        }

        private func border(_ hot: Bool) -> Color {
            switch tone {
            case .approve: return Theme.bronzo.opacity(0.4)
            case .neutral: return Theme.white(0.06)
            case .reject: return hot ? Theme.danger.opacity(0.3) : Theme.white(0.06)
            }
        }
    }
}

/// A text-only button: ink-faint, hover ink-secondary (Cancel, Revert).
struct QuietButtonStyle: ButtonStyle {
    var horizontal: CGFloat = 0
    var vertical: CGFloat = 0
    var size: CGFloat = 11

    func makeBody(configuration: Configuration) -> some View {
        Hovering { hovered in
            configuration.label
                .font(.system(size: size))
                .padding(.horizontal, horizontal)
                .padding(.vertical, vertical)
                .foregroundStyle(hovered ? Theme.inkSecondary : Theme.inkFaint)
                .contentShape(Rectangle())
        }
    }
}

/// What the gate asks, and the last change the reviewer asked for.
struct GateAsk: View {
    let state: NodeExecutionState
    let config: JSONValue?

    var body: some View {
        let message = state.message ?? config?["message"]?.stringValue
        let asked = state.feedback?.last { $0.decision == "changes" }?.comment
        if let message, !message.isEmpty {
            Scroller {
                Text(message)
                    .font(.system(size: 12))
                    .lineSpacing(6)
                    .foregroundStyle(Theme.ink)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
            }
            .frame(maxHeight: 192)
            .fixedSize(horizontal: false, vertical: true)
            .background(Theme.surfaceBase, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.07), lineWidth: 1))
        } else {
            Text("Waiting for approval.")
                .font(.system(size: 11.5))
                .foregroundStyle(Theme.bronzo)
        }
        if let asked, !asked.isEmpty {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text("You asked").foregroundStyle(Theme.inkFaint)
                Text(asked).foregroundStyle(Theme.inkSecondary).lineLimit(2)
            }
            .font(.system(size: 11.5))
        }
    }
}

/// The renderer's textarea: white .03 fill, white .2 hairline, 12.5px gray-200.
struct GateTextArea: View {
    @Binding var text: String
    let placeholder: String
    let rows: Int

    var body: some View {
        ZStack(alignment: .topLeading) {
            if text.isEmpty {
                Text(placeholder)
                    .foregroundStyle(Theme.gray600)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
                    .allowsHitTesting(false)
            }
            TextEditor(text: $text)
                .scrollContentBackground(.hidden)
                .foregroundStyle(Theme.gray200)
                .padding(.horizontal, 5)
                .padding(.vertical, 8)
        }
        .font(.system(size: 12.5))
        .lineSpacing(6)
        .frame(height: CGFloat(rows) * 18.75 + 18)
        .background(Theme.white(0.03), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.2), lineWidth: 1))
    }
}

enum GateComposerKind { case changes, reject }

/// The comment a request for changes needs, or the note a rejection may carry.
struct GateComposer: View {
    let store: WorkflowsStore
    let run: WorkflowExecution
    let state: NodeExecutionState
    let config: JSONValue?
    let nodes: [WorkflowNode]
    let kind: GateComposerKind
    var edited: String?
    var large = false
    let onDone: () -> Void

    @State private var text = ""
    @State private var sending = false
    @FocusState private var focused: Bool

    var body: some View {
        let needsText = kind == .changes
        let fromId = config?["feedback"]?["from"]?.stringValue
        let from = nodes.first { $0.id == fromId }?.label
        VStack(alignment: .leading, spacing: 8) {
            GateTextArea(
                text: $text,
                placeholder: needsText ? "What should change?" : "Why, if it should be kept with the run (optional)",
                rows: 3)
                .focused($focused)
                .onAppear { focused = true }
                .onKeyPress(.escape) {
                    onDone()
                    return .handled
                }
                .onKeyPress(.return, phases: .down) { press in
                    guard press.modifiers.contains(.command) else { return .ignored }
                    submit()
                    return .handled
                }
            hint(from: from)
                .font(.system(size: 11))
                .lineSpacing(3)
                .foregroundStyle(Theme.inkFaint)
            HStack(spacing: 6) {
                Spacer(minLength: 0)
                Button("Cancel", action: onDone)
                    .buttonStyle(QuietButtonStyle(horizontal: large ? 14 : 8, vertical: large ? 8 : 4, size: large ? 12.5 : 11))
                Button(action: submit) {
                    HStack(spacing: 4) {
                        Icon(kind == .changes ? .rotateCcw : .x, size: large ? 13 : 11, weight: .semibold)
                        Text(kind == .changes ? "Send back" : "Reject run")
                    }
                    .font(.system(size: large ? 12.5 : 11))
                }
                .buttonStyle(
                    GateButtonStyle(
                        tone: kind == .changes ? .approve : .reject, horizontal: large ? 14 : 8,
                        vertical: large ? 8 : 4))
                .disabled(sending || (needsText && text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty))
            }
        }
    }

    private func hint(from: String?) -> Text {
        let suffix: String
        if let edited, !edited.isEmpty {
            suffix = kind == .changes ? " Your edit goes back with it." : " Your edit is discarded."
        } else {
            suffix = ""
        }
        guard kind == .changes else { return Text("Ends the run. A note is kept as its reason.\(suffix)") }
        let next = (state.round ?? 1) + 1
        let max = GateRounds.maxRounds(config)
        return Text("Runs again from ")
            + Text(from.flatMap { $0.isEmpty ? nil : $0 } ?? "the chosen step").foregroundColor(Theme.inkSecondary)
            + Text(" with your comment, then asks you. Round \(next) of \(max).\(suffix)")
    }

    private func submit() {
        let note = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !sending, !(kind == .changes && note.isEmpty) else { return }
        sending = true
        Task {
            let ok = await store.resolveGate(
                run, nodeId: state.nodeId, decision: kind == .changes ? .changes : .reject,
                comment: note.isEmpty ? nil : note, edited: kind == .changes ? edited : nil)
            sending = false
            if ok { onDone() }
        }
    }
}

/// The gate's prose, for the reviewer to rewrite before they answer.
struct GateTextEditor: View {
    let state: NodeExecutionState
    var large = false
    let onSave: (String?) -> Void
    let onCancel: () -> Void
    var onApprove: ((String?) -> Void)?

    @State private var text: String
    @FocusState private var focused: Bool

    init(
        state: NodeExecutionState, large: Bool = false, onSave: @escaping (String?) -> Void,
        onCancel: @escaping () -> Void, onApprove: ((String?) -> Void)? = nil
    ) {
        self.state = state
        self.large = large
        self.onSave = onSave
        self.onCancel = onCancel
        self.onApprove = onApprove
        _text = State(initialValue: state.editedText ?? state.editableText ?? "")
    }

    private var original: String { state.editableText ?? "" }

    private var edited: String? {
        text.trimmingCharacters(in: .whitespacesAndNewlines) == original.trimmingCharacters(in: .whitespacesAndNewlines)
            ? nil : text
    }

    var body: some View {
        let words = text.split(whereSeparator: \.isWhitespace).count
        let size: CGFloat = large ? 12.5 : 11
        let h: CGFloat = large ? 14 : 8
        let v: CGFloat = large ? 8 : 4
        VStack(alignment: .leading, spacing: 8) {
            GateTextArea(text: $text, placeholder: "", rows: 8)
                .focused($focused)
                .onAppear { focused = true }
                .onKeyPress(.escape) {
                    onCancel()
                    return .handled
                }
            HStack(spacing: 8) {
                Text("\(words) \(words == 1 ? "word" : "words")")
                if text != original { Text("Edited").foregroundStyle(Theme.inkSecondary) }
                Spacer(minLength: 0)
                Button("Revert") { text = original }
                    .buttonStyle(QuietButtonStyle())
                    .disabled(text == original)
                    .opacity(text == original ? 0.4 : 1)
            }
            .font(.system(size: 11))
            .foregroundStyle(Theme.inkFaint)
            HStack(spacing: 6) {
                Spacer(minLength: 0)
                Button("Cancel", action: onCancel)
                    .buttonStyle(QuietButtonStyle(horizontal: h, vertical: v, size: size))
                Button { onSave(edited) } label: {
                    HStack(spacing: 4) {
                        Icon(.pencil, size: large ? 13 : 11)
                        Text("Save")
                    }
                    .font(.system(size: size))
                }
                .buttonStyle(GateButtonStyle(tone: .neutral, horizontal: h, vertical: v))
                if let onApprove {
                    Button { onApprove(edited) } label: {
                        HStack(spacing: 4) {
                            Icon(.check, size: large ? 13 : 11, weight: .bold)
                            Text("Approve")
                        }
                        .font(.system(size: size))
                    }
                    .buttonStyle(GateButtonStyle(tone: .approve, horizontal: h, vertical: v))
                }
            }
        }
    }
}

/// The compact gate row under a waiting step (GateActions).
struct GateActions: View {
    let store: WorkflowsStore
    let run: WorkflowExecution
    let state: NodeExecutionState
    let config: JSONValue?
    let nodes: [WorkflowNode]

    private enum Mode { case changes, reject, edit }
    @State private var mode: Mode?
    @State private var edited: String?

    var body: some View {
        switch mode {
        case .edit:
            GateTextEditor(
                state: withEdit(state), onSave: { next in
                    edited = next
                    mode = nil
                }, onCancel: { mode = nil },
                onApprove: { next in approve(next) })
        case .changes, .reject:
            GateComposer(
                store: store, run: run, state: state, config: config, nodes: nodes,
                kind: mode == .changes ? .changes : .reject, edited: edited, onDone: { mode = nil })
        case nil:
            HStack(spacing: 6) {
                if state.editableText != nil {
                    small(.pencil, edited == nil ? "Edit" : "Edited", tone: .neutral) { mode = .edit }
                }
                Spacer(minLength: 0)
                if GateRounds.canRequestChanges(config, round: state.round) {
                    small(.messageSquare, "Request changes", tone: .neutral) { mode = .changes }
                }
                small(.x, "Reject", tone: .reject, weight: .bold) { mode = .reject }
                small(.check, "Approve", tone: .approve, weight: .bold) { approve(edited) }
            }
        }
    }

    private func withEdit(_ s: NodeExecutionState) -> NodeExecutionState {
        var s = s
        if let edited { s.editedText = edited }
        return s
    }

    private func approve(_ text: String?) {
        Task { await store.resolveGate(run, nodeId: state.nodeId, decision: .approve, edited: text) }
    }

    private func small(
        _ glyph: Glyph, _ label: String, tone: GateTone, weight: Font.Weight = .regular, action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 4) {
                Icon(glyph, size: 11, weight: weight)
                Text(label)
            }
            .font(.system(size: 11))
        }
        .buttonStyle(GateButtonStyle(tone: tone, horizontal: 8, vertical: 4))
    }
}
