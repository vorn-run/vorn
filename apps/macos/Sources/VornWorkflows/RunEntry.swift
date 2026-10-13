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
                        let titles = store.taskTitles()
                        ForEach(store.workflowRuns) { run in
                            RunEntryCard(store: store, run: run, nodes: run.definition?.nodes ?? workflow.nodes,
                                         taskTitles: titles, onViewFullOutput: { store.showFullOutput($0) },
                                         onResumeSession: store.resumeSession, onClickTask: store.openTask,
                                         taskContext: store.taskContext)
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
    var onClickTask: ((String) -> Void)?
    /// What resume falls back to for a step: the trigger task's project, branch and worktree.
    var taskContext: (String) -> RunTaskContext? = { _ in nil }
    /// Run History offers retry and rerun; a task's runs do not.
    var offersRerun = true
    @State private var expanded: Bool?

    public init(store: WorkflowsStore, run: WorkflowExecution, nodes: [WorkflowNode], workflowName: String? = nil,
                taskTitles: [String: String] = [:], onViewFullOutput: ((String) -> Void)? = nil,
                onResumeSession: ((RunResumeRequest) -> Void)? = nil, onClickTask: ((String) -> Void)? = nil,
                taskContext: @escaping (String) -> RunTaskContext? = { _ in nil }, offersRerun: Bool = true) {
        self.offersRerun = offersRerun
        self.store = store
        self.run = run
        self.nodes = nodes
        self.workflowName = workflowName
        self.taskTitles = taskTitles
        self.onViewFullOutput = onViewFullOutput
        self.onResumeSession = onResumeSession
        self.onClickTask = onClickTask
        self.taskContext = taskContext
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
                            ((workflowName.map { Text($0 + " ").foregroundColor(Theme.inkFaint) } ?? Text(""))
                                + Text(TimeFormat.relative(run.startedAt)).foregroundColor(Theme.ink))
                                .font(.system(size: 12))
                                .lineLimit(1)
                                .truncationMode(.tail)
                                .frame(maxWidth: .infinity, alignment: .leading)
                            if let id = run.triggerTaskId, let title = taskTitles[id] {
                                TaskChip(title: title, maxWidth: 100) { onClickTask?(id) }
                            }
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
                    if offersRerun, run.failedStep != nil {
                        IconButton(lucide: .rotateCcw, size: 12, help: "Retry from failed step") { store.retry(run) }
                    }
                    if offersRerun, run.status != .running {
                        IconButton(lucide: .play, size: 12, help: "Run again") { store.rerun(run) }
                    }
                    if run.status == .running {
                        IconButton(glyph: .square, size: 11, help: "Stop run", hoverTint: Theme.danger) { store.stopRun(run) }
                            .padding(.trailing, 8)
                    }
                }
                .background(hovered ? Theme.white(0.03) : .clear)
            }
            if isOpen {
                RunStepsList(store: store, run: run, nodes: nodes, taskTitles: taskTitles,
                             onViewFullOutput: onViewFullOutput, onResumeSession: onResumeSession,
                             onClickTask: onClickTask, taskContext: taskContext)
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: Theme.radius))
        .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.08), lineWidth: 1))
    }
}

/// The trigger task's place, for resuming a step that did not record its own.
public struct RunTaskContext: Sendable, Hashable {
    public var projectName: String
    public var branch: String?
    public var useWorktree: Bool?

    public init(projectName: String, branch: String? = nil, useWorktree: Bool? = nil) {
        self.projectName = projectName
        self.branch = branch
        self.useWorktree = useWorktree
    }
}

/// TASK_CHIP: a task's title in a hairline box, truncated, opening the task.
struct TaskChip: View {
    let title: String
    var maxWidth: CGFloat
    let action: () -> Void
    @State private var hovered = false

    var body: some View {
        Text(title)
            .font(.system(size: 10))
            .foregroundStyle(Theme.inkSecondary)
            .lineLimit(1)
            .truncationMode(.tail)
            .padding(.horizontal, 6).padding(.vertical, 2)
            .background(hovered ? Theme.white(0.06) : .clear, in: RoundedRectangle(cornerRadius: Theme.radius))
            .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.08), lineWidth: 1))
            .frame(maxWidth: maxWidth, alignment: .leading)
            .fixedSize(horizontal: false, vertical: true)
            .help(title)
            .onHover { hovered = $0 }
            .onTapGesture(perform: action)
    }
}
