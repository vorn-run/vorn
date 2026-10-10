import SwiftUI
import VornCore
import VornUI

/// A run's steps as a trace: inputs, one row per step, a waiting gate inline, logs on expand.
struct RunStepsList: View {
    let store: WorkflowsStore
    let run: WorkflowExecution
    let nodes: [WorkflowNode]
    var includeTrigger = false

    @State private var expanded: String?
    @State private var openedFor: String?

    private var focus: String? { run.status == .error ? run.failedStep?.nodeId : nil }

    var body: some View {
        let byId = Dictionary(nodes.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        let states = includeTrigger ? run.nodeStates : run.nodeStates.filter { byId[$0.nodeId]?.type != .trigger }
        let inputs = (run.inputs ?? [:]).sorted { $0.key < $1.key }
        VStack(spacing: 0) {
            Hairline(opacity: 0.06)
            if !inputs.isEmpty {
                InputsRow(inputs: inputs)
                Hairline(opacity: 0.05)
            }
            ForEach(Array(states.enumerated()), id: \.element.nodeId) { i, state in
                StepRow(
                    store: store, run: run, state: state, node: byId[state.nodeId], nodes: nodes,
                    expanded: expanded == state.nodeId,
                    toggle: { expanded = expanded == state.nodeId ? nil : state.nodeId })
                if i < states.count - 1 { Hairline(opacity: 0.05) }
            }
        }
        .onAppear { syncFocus() }
        .onChange(of: focus) { syncFocus() }
    }

    /// A failed run opens its failed step, once per failure.
    private func syncFocus() {
        guard openedFor != focus else { return }
        openedFor = focus
        if let focus { expanded = focus }
    }
}

private struct InputsRow: View {
    let inputs: [(key: String, value: JSONValue)]

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text("INPUTS")
                .font(.system(size: 10))
                .tracking(0.5)
                .foregroundStyle(Theme.inkFaint)
            FlowRow(spacingX: 12, spacingY: 2) {
                ForEach(inputs, id: \.key) { item in
                    (Text(item.key).foregroundColor(Theme.inkFaint) + Text(" ")
                        + Text(StepVisuals.inputPreview(item.value)).foregroundColor(Theme.inkSecondary))
                        .font(.system(size: 11.5, design: .monospaced))
                        .lineLimit(1)
                        .help("\(item.key)=\(StepVisuals.inputPreview(item.value))")
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
    }
}

private struct StepRow: View {
    let store: WorkflowsStore
    let run: WorkflowExecution
    let state: NodeExecutionState
    let node: WorkflowNode?
    let nodes: [WorkflowNode]
    let expanded: Bool
    let toggle: () -> Void

    @State private var hovered = false

    var body: some View {
        let waitingGate = state.status == .waiting && node?.type == .approval
        let faint = state.status == .pending || state.status == .skipped
        VStack(alignment: .leading, spacing: 0) {
            Button(action: toggle) {
                HStack(spacing: 10) {
                    StatusDot(status: state.rejectedAt != nil ? .cancelled : WorkflowStatusKey(state.status))
                        .frame(width: 8)
                    HStack(spacing: 6) {
                        ZStack {
                            if hovered {
                                Icon(.chevronRight, size: 12, weight: .bold)
                                    .foregroundStyle(Theme.inkFaint)
                                    .rotationEffect(.degrees(expanded ? 90 : 0))
                            } else {
                                Icon(Glyph.node(node?.type ?? .unknown), size: 12)
                                    .foregroundStyle(Theme.inkFaint)
                            }
                        }
                        .frame(width: 12, height: 12)
                        Text(RunPresenter.nodeLabel(node, state.nodeId))
                            .font(.system(size: 12.5))
                            .foregroundStyle(faint ? Theme.inkFaint : Theme.ink)
                            .lineLimit(1)
                        if let meta = StepVisuals.meta(node) {
                            Text(meta)
                                .font(.system(size: 11.5))
                                .foregroundStyle(Theme.inkFaint)
                                .lineLimit(1)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    trailing(waitingGate: waitingGate)
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 8)
                .background(hovered ? Theme.white(0.02) : .clear)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .onHover { hovered = $0 }

            if waitingGate, node != nil {
                VStack(alignment: .leading, spacing: 8) {
                    GateAsk(state: state, config: node?.config)
                    GateActions(store: store, run: run, state: state, config: node?.config, nodes: nodes)
                }
                .underLabel()
            }
            if state.isSignInWait {
                HStack(alignment: .top, spacing: 8) {
                    Text(state.error ?? "Signed out. Sign in, and this step runs again.")
                        .font(.system(size: 11.5))
                        .foregroundStyle(Theme.bronzo)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    SignInButton(compact: true)
                }
                .underLabel()
            }
            if expanded { ExpandedStep(state: state).underLabel() }
        }
    }

    @ViewBuilder private func trailing(waitingGate: Bool) -> some View {
        if let start = state.startedAt, let end = state.completedAt {
            Text(TimeFormat.runDuration(start, end))
                .font(.system(size: 12, design: .monospaced))
                .monospacedDigit()
                .foregroundStyle(Theme.inkSecondary)
        } else if waitingGate, let label = GateRounds.label(node?.config, round: state.round) {
            Text(label)
                .font(.system(size: 11))
                .foregroundStyle(Theme.inkFaint)
        }
    }
}

private struct ExpandedStep: View {
    let state: NodeExecutionState

    var body: some View {
        let timeline = StepVisuals.timeline(logs: state.logs, diagnostics: state.diagnostics)
        VStack(alignment: .leading, spacing: 6) {
            if let error = state.error, !error.isEmpty {
                Text(error)
                    .font(.system(size: 12))
                    .foregroundStyle(state.rejectedAt != nil ? Theme.inkSecondary : Theme.danger)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            if !timeline.isEmpty {
                Scroller {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(timeline.enumerated()), id: \.offset) { _, entry in
                            Text(entry.text)
                                .font(.system(size: 12, design: .monospaced))
                                .lineSpacing(4)
                                .foregroundStyle(entry.kind == .agent ? Theme.inkSecondary : Theme.inkFaint)
                                .textSelection(.enabled)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .padding(.horizontal, 10)
                                .padding(.vertical, entry.kind == .agent ? 6 : 2)
                                .background(entry.kind == .engine ? Theme.white(0.02) : .clear)
                        }
                    }
                }
                .frame(maxHeight: 280)
                .fixedSize(horizontal: false, vertical: true)
                .background(Color.black.opacity(0.3), in: RoundedRectangle(cornerRadius: Theme.radius))
                .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.05), lineWidth: 1))
            }
            if timeline.isEmpty && (state.error ?? "").isEmpty {
                Text(StepVisuals.emptyLog(state.status))
                    .font(.system(size: 11.5))
                    .italic()
                    .foregroundStyle(Theme.inkFaint)
            }
        }
    }
}

/// Opens a connection's sign-in; the native app has no sign-in window yet, so it stays shut.
struct SignInButton: View {
    var compact = false

    var body: some View {
        Button {} label: {
            HStack(spacing: compact ? 4 : 8) {
                Icon(symbol: "person.crop.circle.badge.checkmark", size: compact ? 11 : 14)
                Text("Sign in to the connection")
            }
            .font(.system(size: compact ? 11 : 13))
            .frame(maxWidth: compact ? nil : .infinity, alignment: .leading)
        }
        .buttonStyle(GateButtonStyle(tone: .approve, horizontal: compact ? 8 : 16, vertical: compact ? 4 : 10))
        .disabled(true)
    }
}

extension View {
    /// UNDER_LABEL: content indented under a step's name.
    func underLabel() -> some View {
        frame(maxWidth: .infinity, alignment: .leading)
            .padding(.leading, 34)
            .padding(.trailing, 16)
            .padding(.bottom, 10)
    }
}

/// Wraps children onto new lines, like `flex flex-wrap`.
struct FlowRow: Layout {
    var spacingX: CGFloat
    var spacingY: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(width: proposal.width ?? .infinity, subviews: subviews)
        let width = rows.map { $0.width }.max() ?? 0
        let height = rows.map(\.height).reduce(0, +) + spacingY * CGFloat(max(0, rows.count - 1))
        return CGSize(width: proposal.width ?? width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in arrange(width: bounds.width, subviews: subviews) {
            var x = bounds.minX
            for i in row.items {
                let size = subviews[i].sizeThatFits(ProposedViewSize(width: bounds.width, height: nil))
                subviews[i].place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(width: min(size.width, bounds.width), height: size.height))
                x += size.width + spacingX
            }
            y += row.height + spacingY
        }
    }

    private struct Row { var items: [Int] = []; var width: CGFloat = 0; var height: CGFloat = 0 }

    private func arrange(width: CGFloat, subviews: Subviews) -> [Row] {
        var rows: [Row] = [Row()]
        for i in subviews.indices {
            let size = subviews[i].sizeThatFits(ProposedViewSize(width: width, height: nil))
            if !rows[rows.count - 1].items.isEmpty, rows[rows.count - 1].width + spacingX + size.width > width {
                rows.append(Row())
            }
            var row = rows[rows.count - 1]
            row.width += (row.items.isEmpty ? 0 : spacingX) + size.width
            row.height = max(row.height, size.height)
            row.items.append(i)
            rows[rows.count - 1] = row
        }
        return rows.filter { !$0.items.isEmpty }
    }
}
