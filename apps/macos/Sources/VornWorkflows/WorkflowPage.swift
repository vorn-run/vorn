import SwiftUI
import VornCore
import VornUI

/// One workflow: the editor's top bar, its graph, and the Run History panel.
struct WorkflowPage<Leading: View>: View {
    @Bindable var store: WorkflowsStore
    let workflow: WorkflowDefinition
    @ViewBuilder var leading: Leading

    var body: some View {
        let running = store.workflowRuns.first { $0.status == .running }
        VStack(spacing: 0) {
            WorkflowPageBar(store: store, workflow: workflow, running: running, leading: { leading })
            HStack(spacing: 0) {
                WorkflowCanvasView(workflow: workflow, latestRun: store.workflowRuns.first)
                if store.historyOpen {
                    RunHistoryPanel(store: store, workflow: workflow)
                }
            }
        }
        .background(Theme.surfaceBase)
    }
}

/// The 40pt bar: icon and name on the left; run, history, save and more on the right.
struct WorkflowPageBar<Leading: View>: View {
    let store: WorkflowsStore
    let workflow: WorkflowDefinition
    let running: WorkflowExecution?
    @ViewBuilder var leading: Leading
    @State private var moreFrame = CGRect.zero

    var body: some View {
        let hasTrigger = workflow.triggerNode != nil
        let moreOpen = store.menu?.kind == .pageMore(workflow.id)
        HStack(spacing: 0) {
            HStack(spacing: 4) {
                leading
                Icon(symbol: WorkflowIcons.symbolOrWorkflow(workflow.icon), size: 16)
                    .foregroundStyle(WorkflowIcons.color(workflow.iconColor))
                    .padding(6)
                Text(workflow.name)
                    .font(.system(size: 15, weight: .medium))
                    .foregroundStyle(Color.white)
                    .lineLimit(1)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .frame(width: 240, alignment: .leading)
            }
            Spacer(minLength: 8)
            HStack(spacing: 4) {
                if let running {
                    HStack(spacing: 2) {
                        HStack(spacing: 6) {
                            Spinning(active: true) { Icon(.loader, size: 13) }
                            TimelineView(.periodic(from: .now, by: 1)) { ctx in
                                Text(TimeFormat.elapsed(since: running.startedAt, now: ctx.date))
                                    .font(.system(size: 11, design: .monospaced))
                                    .monospacedDigit()
                            }
                        }
                        .foregroundStyle(Theme.gray300)
                        .padding(.horizontal, 6)
                        BarButton(glyph: .square, size: 13, help: "Stop run", hoverTint: Theme.danger) {
                            store.stopRun(running)
                        }
                    }
                } else {
                    BarButton(
                        glyph: .play, size: 15,
                        help: hasTrigger ? "Run workflow" : "Add a trigger before running",
                        disabled: !hasTrigger
                    ) { store.runNow(workflow) }
                }
                BarButton(
                    glyph: .history, size: 15,
                    help: "Run history\(store.workflowRuns.isEmpty ? "" : " (\(store.workflowRuns.count))")",
                    pressed: store.historyOpen
                ) { store.historyOpen.toggle() }
                Button {} label: {
                    HStack(spacing: 6) {
                        Icon(.save, size: 13)
                        Text("Save")
                    }
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(Color.white)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                    .background(Theme.white(0.12), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                }
                .buttonStyle(.plain)
                .disabled(true)
                .help("Editing arrives with the visual editor")
                .padding(.leading, 4)
                BarButton(glyph: .more, size: 15, help: "More options", pressed: moreOpen) {
                    store.menu = moreOpen ? nil : WorkflowsMenu(kind: .pageMore(workflow.id), anchor: moreFrame, edge: .trailing)
                }
                .trackFrame($moreFrame)
                .padding(.leading, 2)
            }
        }
        .padding(.horizontal, 12)
        .frame(height: Theme.toolbarHeight)
        .overlay(alignment: .bottom) { Hairline(opacity: 0.08) }
    }
}

/// `p-1.5 rounded-md` bar button: gray-400, hover white on white .06, pressed white on white .08.
struct BarButton: View {
    let glyph: Glyph
    var size: CGFloat = 15
    var help: String
    var disabled = false
    var pressed = false
    var hoverTint: Color = .white
    let action: () -> Void

    var body: some View {
        Hovering { hovered in
            Button(action: action) {
                Icon(glyph, size: size)
                    .foregroundStyle(pressed ? .white : hovered && !disabled ? hoverTint : Theme.gray400)
                    .padding(6)
                    .background(
                        pressed ? Theme.white(0.08) : hovered && !disabled ? Theme.white(0.06) : .clear,
                        in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(disabled)
            .opacity(disabled ? 0.4 : 1)
            .help(help)
        }
    }
}

/// The 340pt side panel listing this workflow's runs.
struct RunHistoryPanel: View {
    let store: WorkflowsStore
    let workflow: WorkflowDefinition

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Run History")
                    .font(.system(size: 13, weight: .medium))
                    .foregroundStyle(Color.white)
                Spacer()
                Hovering { hovered in
                    Button { store.historyOpen = false } label: {
                        Icon(.x, size: 14)
                            .foregroundStyle(hovered ? .white : Theme.gray500)
                            .padding(4)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .overlay(alignment: .bottom) { Hairline(opacity: 0.08) }
            Scroller {
                VStack(spacing: 8) {
                    if store.workflowRuns.isEmpty {
                        Text("No runs yet")
                            .font(.system(size: 12))
                            .foregroundStyle(Theme.gray600)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 32)
                    } else {
                        ForEach(store.workflowRuns) { run in
                            RunEntryCard(store: store, run: run, nodes: run.definition?.nodes ?? workflow.nodes)
                        }
                    }
                }
                .padding(12)
            }
        }
        .frame(width: 340)
        .frame(maxHeight: .infinity)
        .background(Theme.surfaceNode)
        .overlay(alignment: .leading) { Hairline(opacity: 0.08, vertical: true) }
    }
}

/// One run in the history panel; expands to its steps, open by default while a gate waits.
struct RunEntryCard: View {
    let store: WorkflowsStore
    let run: WorkflowExecution
    let nodes: [WorkflowNode]
    @State private var expanded: Bool?

    var body: some View {
        let isOpen = expanded ?? (run.waitingStep != nil)
        VStack(spacing: 0) {
            Hovering { hovered in
                HStack(spacing: 0) {
                    Button { expanded = !isOpen } label: {
                        HStack(spacing: 8) {
                            Icon(isOpen ? .chevronDown : .chevronRight, size: 12)
                                .foregroundStyle(Theme.inkFaint)
                            StatusDot(status: RunPresenter.liveDotStatus(run))
                            Text(TimeFormat.relative(run.startedAt))
                                .font(.system(size: 12))
                                .foregroundStyle(Theme.ink)
                                .lineLimit(1)
                                .frame(maxWidth: .infinity, alignment: .leading)
                            if run.partial == true {
                                Text("partial").font(.system(size: 11)).foregroundStyle(Theme.inkFaint)
                            }
                            Text(TimeFormat.runDuration(run.startedAt, run.completedAt))
                                .font(.system(size: 11, design: .monospaced))
                                .monospacedDigit()
                                .foregroundStyle(Theme.inkSecondary)
                        }
                        .padding(.horizontal, 12)
                        .padding(.vertical, 10)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    if run.failedStep != nil {
                        IconButton(glyph: .rotateCcw, size: 12, help: "Retry from failed step") { store.retry(run) }
                    }
                    if run.status != .running {
                        IconButton(glyph: .play, size: 12, help: "Run again") { store.rerun(run) }
                    }
                    if run.status == .running {
                        IconButton(glyph: .square, size: 11, help: "Stop run", hoverTint: Theme.danger) { store.stopRun(run) }
                            .padding(.trailing, 8)
                    }
                }
                .background(hovered ? Theme.white(0.03) : .clear)
            }
            if isOpen {
                RunStepsList(store: store, run: run, nodes: nodes)
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: Theme.radius))
        .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.08), lineWidth: 1))
    }
}
