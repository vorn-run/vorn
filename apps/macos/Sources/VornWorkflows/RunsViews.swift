import SwiftUI
import VornCore
import VornUI

/// A run's glyph: its workflow's icon and colour, else how it was triggered.
struct RunIconView: View {
    let presentation: RunPresentation
    var size: CGFloat = 13
    var fallbackTint: Color = Theme.gray400

    var body: some View {
        if let symbol = WorkflowIcons.symbol(presentation.iconName) {
            Icon(symbol: symbol, size: size, weight: .light)
                .foregroundStyle(WorkflowIcons.color(presentation.iconColor))
        } else {
            Icon(Glyph.source(presentation.source), size: size, weight: .light)
                .foregroundStyle(fallbackTint)
        }
    }
}

/// The runs column: header with a count, then one row per run.
struct RunsListView: View {
    let store: WorkflowsStore

    var body: some View {
        let runs = store.visibleRuns
        let waiting = runs.filter { $0.waitingStep != nil }.count
        let selected = store.selectedRun?.runId
        VStack(spacing: 0) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(store.effectiveFilter == .all ? "All runs" : "Runs")
                    .font(.system(size: 15))
                    .foregroundStyle(Theme.ink)
                Text("\(runs.count)" + (waiting > 0 ? " · \(waiting) waiting" : ""))
                    .font(.system(size: 12))
                    .foregroundStyle(Theme.inkFaint)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            Hairline(opacity: 0.04)
            if runs.isEmpty {
                Text("No runs to show")
                    .font(.system(size: 12))
                    .foregroundStyle(Theme.inkFaint)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 40)
                Spacer(minLength: 0)
            } else {
                Scroller {
                    VStack(spacing: 0) {
                        ForEach(runs) { run in
                            RunListRow(
                                run: run, workflow: store.ref(for: run), deleted: store.isDeleted(run),
                                selected: run.runId == selected,
                                onSelect: { store.selectedRunId = run.runId },
                                onOpen: { store.openWorkflow(run.workflowId) })
                        }
                    }
                }
            }
        }
    }
}

struct RunListRow: View {
    let run: WorkflowExecution
    let workflow: RunWorkflowRef?
    let deleted: Bool
    let selected: Bool
    let onSelect: () -> Void
    let onOpen: () -> Void

    @State private var hovered = false

    var body: some View {
        let p = RunPresenter.describe(run, workflow: workflow)
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 10) {
                StatusDot(status: RunPresenter.liveDotStatus(run), size: 6)
                HStack(spacing: 8) {
                    RunIconView(presentation: p)
                    Text(p.title)
                        .font(.system(size: 13))
                        .foregroundStyle(Theme.ink)
                        .lineLimit(1)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                Text(TimeFormat.runDuration(run.startedAt, run.completedAt))
                    .font(.system(size: 12, design: .monospaced))
                    .monospacedDigit()
                    .foregroundStyle(Theme.inkSecondary)
            }
            Text(RunPresenter.detailLine(run, workflow: workflow, deleted: deleted))
                .font(.system(size: 12))
                .foregroundStyle(Theme.inkFaint)
                .lineLimit(1)
                .padding(.leading, 16)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(selected ? Theme.white(0.04) : hovered ? Theme.white(0.02) : .clear)
        .overlay(alignment: .leading) {
            if selected {
                Capsule().fill(Color.white).frame(width: 1).padding(.vertical, 4)
            }
        }
        .overlay(alignment: .bottom) { Hairline(opacity: 0.04) }
        .contentShape(Rectangle())
        .onHover { hovered = $0 }
        .onTapGesture(count: 2) { if !deleted { onOpen() } }
        .simultaneousGesture(TapGesture().onEnded(onSelect))
    }
}

/// The selected run: header, the open gate, then its steps.
struct RunDetailPane: View {
    let store: WorkflowsStore
    let run: WorkflowExecution

    @State private var composingFor: String?
    @State private var editingFor: String?
    @State private var rewrite: (gate: String, text: String)?

    var body: some View {
        let workflow = store.ref(for: run)
        let full = store.workflow(run.workflowId)
        let nodes = workflow?.nodes ?? []
        let p = RunPresenter.describe(run, workflow: workflow)
        let waiting = run.waitingStep
        let verdict = run.status == .success ? RunPresenter.verdict(run) : nil
        Scroller {
            VStack(alignment: .leading, spacing: 0) {
                VStack(alignment: .leading, spacing: 8) {
                    HStack(spacing: 4) {
                        RunIconView(presentation: p, size: 15, fallbackTint: Theme.inkSecondary)
                            .padding(.trailing, 4)
                        Text(p.title)
                            .font(.system(size: 15, weight: .medium))
                            .foregroundStyle(Theme.ink)
                            .lineLimit(1)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        if run.failedStep != nil, full != nil {
                            IconButton(glyph: .rotateCcw, help: "Retry from failed step") { store.retry(run) }
                        }
                        if run.status != .running, full != nil {
                            IconButton(glyph: .play, help: "Run again") { store.rerun(run) }
                        }
                        IconButton(
                            glyph: .externalLink,
                            help: store.isDeleted(run) ? "Workflow no longer exists" : "Open workflow",
                            disabled: store.isDeleted(run)
                        ) { store.openWorkflow(run.workflowId) }
                        if run.status == .running {
                            IconButton(glyph: .square, size: 11, help: "Stop run") { store.stopRun(run) }
                        }
                    }
                    Text(RunPresenter.metaLine(run, workflow: workflow))
                        .font(.system(size: 12))
                        .foregroundStyle(Theme.inkFaint)
                        .lineLimit(1)
                    if let subtitle = p.subtitle {
                        Text(subtitle)
                            .font(.system(size: 13))
                            .foregroundStyle(Theme.inkSecondary)
                    }
                    HStack(spacing: 8) {
                        StatusDot(status: waiting != nil ? .waiting : RunPresenter.dotStatus(run))
                        Text(RunPresenter.statusLine(run, nodes: nodes)).foregroundStyle(Theme.ink)
                        if let verdict {
                            Text("·").foregroundStyle(Theme.inkFaint)
                            Text(verdict).foregroundStyle(Theme.inkSecondary)
                        }
                    }
                    .font(.system(size: 13))
                }
                .padding(.horizontal, 20)
                .padding(.top, 16)
                .padding(.bottom, 16)

                if let waiting {
                    gateBlock(waiting, nodes: nodes)
                        .padding(.horizontal, 20)
                        .padding(.bottom, 16)
                }

                RunStepsList(store: store, run: run, nodes: nodes, includeTrigger: true)
                    .padding(.horizontal, 20)
                    .padding(.bottom, 24)
            }
        }
        .id(run.runId)
    }

    private func gateKey(_ state: NodeExecutionState) -> String { "\(run.runId):\(state.nodeId)" }

    @ViewBuilder
    private func gateBlock(_ state: NodeExecutionState, nodes: [WorkflowNode]) -> some View {
        let key = gateKey(state)
        let signIn = state.isSignInWait
        let gate = signIn ? nil : nodes.first { $0.id == state.nodeId && $0.type == .approval }
        let saved = rewrite?.gate == key ? rewrite?.text : nil
        let editing = editingFor == key
        let composing = composingFor == key
        VStack(alignment: .leading, spacing: 6) {
            if let gate {
                GateAsk(state: state, config: gate.config)
                if editing {
                    GateTextEditor(
                        state: withSaved(state, saved), large: true,
                        onSave: { next in
                            rewrite = next.map { (key, $0) }
                            editingFor = nil
                        }, onCancel: { editingFor = nil })
                } else if composing {
                    GateComposer(
                        store: store, run: run, state: state, config: gate.config, nodes: nodes, kind: .changes,
                        edited: saved, large: true, onDone: { composingFor = nil })
                } else if state.editableText != nil || GateRounds.canRequestChanges(gate.config, round: state.round) {
                    HStack(spacing: 6) {
                        if state.editableText != nil {
                            neutral(.pencil, saved == nil ? "Edit" : "Edited") { editingFor = key }
                        }
                        if GateRounds.canRequestChanges(gate.config, round: state.round) {
                            neutral(.messageSquare, "Request changes") { composingFor = key }
                        }
                    }
                }
            }
            if signIn {
                SignInButton()
            } else {
                Button { answer(state, .approve, edited: saved) } label: {
                    HStack(spacing: 8) {
                        Icon(.check, size: 14, weight: .semibold)
                        Text("Approve & continue")
                        Spacer(minLength: 0)
                        Kbd(text: "⌘↵")
                    }
                    .font(.system(size: 13))
                }
                .buttonStyle(GateButtonStyle(tone: .approve, vertical: 10))
                .keyboardShortcut(.return, modifiers: .command)
                .disabled(editing || composing)
            }
            Button { answer(state, .reject) } label: {
                HStack(spacing: 8) {
                    Icon(.x, size: 14, weight: .semibold)
                    Text("Reject run")
                    Spacer(minLength: 0)
                    if !signIn { Kbd(text: "R") }
                }
                .font(.system(size: 13))
            }
            .buttonStyle(GateButtonStyle(tone: .reject, vertical: 10))
            .keyboardShortcut(signIn || editing || composing ? nil : KeyboardShortcut("r", modifiers: []))
        }
    }

    private func withSaved(_ s: NodeExecutionState, _ saved: String?) -> NodeExecutionState {
        var s = s
        if let saved { s.editedText = saved }
        return s
    }

    private func neutral(_ glyph: Glyph, _ label: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 8) {
                Icon(glyph, size: 13)
                Text(label)
            }
            .font(.system(size: 12.5))
            .frame(maxWidth: .infinity)
        }
        .buttonStyle(GateButtonStyle(tone: .neutral))
    }

    private func answer(_ state: NodeExecutionState, _ decision: GateDecision, edited: String? = nil) {
        Task { await store.resolveGate(run, nodeId: state.nodeId, decision: decision, edited: edited) }
    }
}

struct EmptyRunDetail: View {
    var body: some View {
        VStack(spacing: 8) {
            Icon(.inbox, size: 22, weight: .light)
            Text("Select a run to see its trace").font(.system(size: 12))
        }
        .foregroundStyle(Theme.inkFaint)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// The landing: the runs column, a draggable seam, and the selected run's trace.
struct WorkflowsLandingView: View {
    @Bindable var store: WorkflowsStore
    @State private var dragStart: CGFloat?
    @State private var seamHovered = false

    var body: some View {
        HStack(spacing: 0) {
            RunsListView(store: store)
                .frame(width: store.listWidth)
            Rectangle()
                .fill(Theme.white(seamHovered || dragStart != nil ? 0.16 : 0.06))
                .frame(width: 1)
                .overlay(
                    Color.clear
                        .frame(width: 7)
                        .contentShape(Rectangle())
                        .onHover { seamHovered = $0 }
                        .gesture(
                            DragGesture(minimumDistance: 0)
                                .onChanged { g in
                                    let start = dragStart ?? store.listWidth
                                    dragStart = start
                                    store.listWidth = min(720, max(280, start + g.translation.width))
                                }
                                .onEnded { _ in dragStart = nil }
                        )
                )
            Group {
                if let run = store.selectedRun {
                    RunDetailPane(store: store, run: run)
                } else {
                    EmptyRunDetail()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}
