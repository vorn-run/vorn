import SwiftUI
import VornCore
import VornUI

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

/// What resuming a step's agent conversation needs (RunEntry.tsx onResumeSession).
public struct RunResumeRequest: Sendable, Hashable {
    public var agentSessionId: String
    public var agentType: AgentType
    public var projectName: String
    public var projectPath: String
    public var branch: String?
    public var useWorktree: Bool?

    public init(agentSessionId: String, agentType: AgentType, projectName: String, projectPath: String,
                branch: String? = nil, useWorktree: Bool? = nil) {
        self.agentSessionId = agentSessionId
        self.agentType = agentType
        self.projectName = projectName
        self.projectPath = projectPath
        self.branch = branch
        self.useWorktree = useWorktree
    }
}

/// RunEntry.tsx: one run in the history panel or a task's runs; expands to its steps, open by default while a gate waits.
public struct RunEntryCard: View {
    let store: WorkflowsStore
    let run: WorkflowExecution
    let nodes: [WorkflowNode]
    var workflowName: String?
    /// Task titles by id, for steps that made or worked a task.
    var taskTitles: [String: String] = [:]
    var onViewFullOutput: ((String) -> Void)?
    var onResumeSession: ((RunResumeRequest) -> Void)?
    @State private var expanded: Bool?

    public init(store: WorkflowsStore, run: WorkflowExecution, nodes: [WorkflowNode], workflowName: String? = nil,
                taskTitles: [String: String] = [:], onViewFullOutput: ((String) -> Void)? = nil,
                onResumeSession: ((RunResumeRequest) -> Void)? = nil) {
        self.store = store
        self.run = run
        self.nodes = nodes
        self.workflowName = workflowName
        self.taskTitles = taskTitles
        self.onViewFullOutput = onViewFullOutput
        self.onResumeSession = onResumeSession
    }

    public var body: some View {
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
