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
