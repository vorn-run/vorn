import Foundation
import Observation
import VornCore

public enum ToastTone: Sendable { case error, success, warning }

/// A labelled button on a toast.
public struct ToastAction {
    public let label: String
    public let run: @MainActor () -> Void

    public init(label: String, run: @escaping @MainActor () -> Void) {
        self.label = label
        self.run = run
    }
}

public enum WorkflowsTab: String, Sendable { case runs, review }

public enum SidebarWorkflowFilter: String, Sendable, CaseIterable {
    case all, manual, scheduled

    public var label: String {
        switch self {
        case .all: return "All"
        case .manual: return "Manual"
        case .scheduled: return "Scheduled"
        }
    }
}

/// State behind the Workflows view: the workspace's workflows, its runs, and what is selected.
@Observable @MainActor
public final class WorkflowsStore {
    public private(set) var workflows: [WorkflowDefinition] = []
    public private(set) var runs: [WorkflowExecution] = []
    /// Runs of the open workflow, for its Run History panel.
    public private(set) var workflowRuns: [WorkflowExecution] = []
    public var workspaceId: String
    public var tab: WorkflowsTab = .runs
    public var runFilter: RunBucket = .all
    public var sidebarFilter: SidebarWorkflowFilter = .all
    public var selectedRunId: String?
    public private(set) var editingWorkflowId: String?
    /// A new workflow's editor is open (it has no id yet).
    public private(set) var editorOpen = false
    /// The open workflow's draft; one editor serves every workflow, reloaded per open.
    public private(set) var editor: WorkflowEditorModel?
    /// Canvas views and the step library's memory; renders and tests use an ephemeral one.
    @ObservationIgnored public var editorMemory: EditorMemory = .userDefaults
    public var historyOpen = false
    public var listWidth: CGFloat = 420
    public private(set) var loading = 0
    /// One transient message, shown like the app's error toast.
    public var toast: String? {
        didSet {
            toastTone = nextTone ?? .error
            nextTone = nil
            toastAction = nextAction
            nextAction = nil
        }
    }
    /// The toast's one button, like a toast's `actions` entry; it dismisses the toast itself.
    public private(set) var toastAction: ToastAction?
    @ObservationIgnored private var nextAction: ToastAction?
    public private(set) var toastTone: ToastTone = .error
    @ObservationIgnored private var nextTone: ToastTone?
    /// The open dropdown, drawn by `workflowsMenuHost`.
    public var menu: WorkflowsMenu?
    public var sidebarSectionOpen = true
    public private(set) var connectionError: String?
    /// The shell's projects and the sidebar's active one, for forms that pick a project.
    public var projects: [Project] = []
    public var activeProjectName: String?
    /// The shell's tasks, for run chips and resume: title and place by task id.
    @ObservationIgnored public var taskTitles: () -> [String: String] = { [:] }
    @ObservationIgnored public var taskContext: (String) -> RunTaskContext? = { _ in nil }
    /// Opens a task from a run's chip, and reopens a step's agent conversation.
    @ObservationIgnored public var openTask: ((String) -> Void)?
    @ObservationIgnored public var resumeSession: ((RunResumeRequest) -> Void)?
    /// The run prompt, full output, review page and missed schedules drawn over the window.
    public let surfaces = RunSurfaces()
    /// Every run update vornd pushes, for the app's notifications.
    @ObservationIgnored public var onRunUpdate: ((WorkflowExecution) -> Void)?

    @ObservationIgnored let backend: WorkflowsBackend
    @ObservationIgnored private var eventsTask: Task<Void, Never>?
    @ObservationIgnored private var reloadTask: Task<Void, Never>?

    public static let runsLimit = 50

    public init(backend: WorkflowsBackend, workspaceId: String = "personal") {
        self.backend = backend
        self.workspaceId = workspaceId
    }

    /// Loads everything and follows vornd's pushes until `stop()`.
    public func start() async {
        guard eventsTask == nil else { return await reload() }
        let backend = backend
        eventsTask = Task { [weak self] in
            do {
                for await event in try await backend.events() { self?.apply(event) }
            } catch {
                self?.connectionError = String(describing: error)
            }
        }
        await reload()
    }

    public func stop() {
        eventsTask?.cancel()
        eventsTask = nil
        reloadTask?.cancel()
    }

    public func reload() async {
        loading += 1
        defer { loading -= 1 }
        do {
            async let wfs = backend.listWorkflows()
            async let rs = backend.listAllRuns(workspaceId: workspaceId, limit: Self.runsLimit)
            workflows = try await wfs
            mergePersisted(try await rs)
            if let id = editingWorkflowId { workflowRuns = try await backend.listRuns(workflowId: id, limit: Self.runsLimit) }
            connectionError = nil
        } catch {
            connectionError = String(describing: error)
        }
    }

    // MARK: Derived

    public var isLoading: Bool { loading > 0 }

    public var workspaceWorkflows: [WorkflowDefinition] {
        workflows.filter { $0.workspace == workspaceId }
    }

    /// The sidebar's list, after its filter.
    public var sidebarWorkflows: [WorkflowDefinition] {
        workspaceWorkflows.filter {
            switch sidebarFilter {
            case .all: return true
            case .manual: return !$0.isScheduled
            case .scheduled: return $0.isScheduled
            }
        }
    }

    public func workflow(_ id: String) -> WorkflowDefinition? { workflows.first { $0.id == id } }

    public var editingWorkflow: WorkflowDefinition? { editingWorkflowId.flatMap(workflow) }

    /// The editor page shows: an open workflow, or a new one.
    public var isEditing: Bool { editorOpen || editingWorkflow != nil }

    /// The row's workflow, or what the run remembers of a deleted one.
    public func ref(for run: WorkflowExecution) -> RunWorkflowRef? {
        if let w = workflow(run.workflowId) { return RunWorkflowRef(w) }
        if let d = run.definition { return RunWorkflowRef(d) }
        if let name = run.workflowName { return RunWorkflowRef(name: name) }
        return nil
    }

    public func findRun(_ id: String) -> WorkflowExecution? {
        runs.first { $0.runId == id } ?? workflowRuns.first { $0.runId == id }
    }

    /// The address of a gate's review page for this round (gate-view-url.ts).
    public func gateViewURL(runId: String, nodeId: String, token: String) async -> URL? {
        guard let base = await backend.serverURL() else { return nil }
        let segment: (String) -> String = { $0.addingPercentEncoding(withAllowedCharacters: .alphanumerics) ?? $0 }
        return URL(string: "\(base.absoluteString)/gate-view/\(segment(runId))/\(segment(nodeId))?t=\(segment(token))")
    }

    public func openReview(_ run: WorkflowExecution, nodeId: String, workflowName: String? = nil) {
        surfaces.review = GateReviewTarget(runId: run.runId, nodeId: nodeId, workflowName: workflowName)
    }

    public func isDeleted(_ run: WorkflowExecution) -> Bool { workflow(run.workflowId) == nil }

    /// Runs in this workspace, newest first.
    public var workspaceRuns: [WorkflowExecution] {
        let ids = Set(workspaceWorkflows.map(\.id))
        return runs.filter { run in
            if ids.contains(run.workflowId) { return true }
            // A deleted workflow's runs stay listed where the server put them.
            return workflow(run.workflowId) == nil
        }
    }

    public var effectiveFilter: RunBucket { tab == .review ? .waiting : runFilter }

    public var visibleRuns: [WorkflowExecution] {
        let filter = effectiveFilter
        guard filter != .all else { return workspaceRuns }
        return workspaceRuns.filter { RunPresenter.bucket(of: $0) == filter }
    }

    /// Gates waiting on the user; a sign-in wait is not one of them.
    public var waitingCount: Int {
        workspaceRuns.reduce(0) { n, run in
            n + run.nodeStates.filter { $0.status == .waiting && !$0.isSignInWait }.count
        }
    }

    /// Gates waiting on the user, one per step, for the session dock (useWaitingApprovals).
    public var waitingApprovals: [WaitingApproval] {
        workspaceRuns.flatMap { run in
            run.nodeStates.filter { $0.status == .waiting && !$0.isSignInWait }.map {
                WaitingApproval(run: run, state: $0, workflow: workflow(run.workflowId))
            }
        }
    }

    public func waitingCount(for workflowId: String) -> Int {
        workspaceRuns.filter { $0.workflowId == workflowId && $0.waitingStep != nil }.count
    }

    /// The selection, falling back to the first visible run.
    public var selectedRun: WorkflowExecution? {
        let visible = visibleRuns
        if let id = selectedRunId, let run = visible.first(where: { $0.runId == id }) { return run }
        return visible.first
    }

    // MARK: Navigation

    /// Opens a workflow's page; the returned task settles once its runs are loaded.
    @discardableResult
    public func openWorkflow(_ id: String) -> Task<Void, Never> {
        editingWorkflowId = id
        workflowRuns = runs.filter { $0.workflowId == id }
        editorModel().load(workflow(id))
        return Task { await loadWorkflowRuns(id) }
    }

    /// The sidebar's New workflow: a blank editor, offered a template first.
    public func newWorkflow() {
        editingWorkflowId = nil
        workflowRuns = []
        editorModel().load(nil)
        editorOpen = true
    }

    /// Opens a run on the runs landing, whatever filter was set (showRun).
    public func showRun(_ runId: String) {
        closeEditor()
        tab = .runs
        runFilter = .all
        selectedRunId = runId
    }

    public func showAllRuns() {
        closeEditor()
    }

    /// Leaves the editor for the runs landing; an import's leftovers go with it.
    public func closeEditor() {
        editingWorkflowId = nil
        workflowRuns = []
        editorOpen = false
        importedRequirements = nil
    }

    /// A new workflow's first save makes it the open one.
    func adoptEditing(_ id: String) {
        editingWorkflowId = id
        editorOpen = false
    }

    private func editorModel() -> WorkflowEditorModel {
        if let editor { return editor }
        let made = WorkflowEditorModel(store: self)
        editor = made
        return made
    }

    /// Writes a definition through vornd and keeps the local list in step.
    func saveWorkflow(_ wf: WorkflowDefinition, isNew: Bool) async {
        if let i = workflows.firstIndex(where: { $0.id == wf.id }) { workflows[i] = wf } else { workflows.append(wf) }
        do {
            let json = try JSONValue.from(wf)
            if isNew {
                _ = try await backend.rpc("workflow:create", .object(["workflow": json]))
            } else {
                _ = try await backend.rpc("workflow:update", .object(["id": .string(wf.id), "updates": json]))
            }
        } catch {
            notify(String(describing: error))
        }
    }

    /// Shows one toast; errors stay red, the rest carry their own tone.
    public func notify(_ message: String, tone: ToastTone = .error, action: ToastAction? = nil) {
        nextTone = tone
        nextAction = action
        toast = message
    }

    private func loadWorkflowRuns(_ id: String) async {
        loading += 1
        defer { loading -= 1 }
        do {
            let list = try await backend.listRuns(workflowId: id, limit: Self.runsLimit)
            if editingWorkflowId == id { workflowRuns = list }
        } catch {
            toast = String(describing: error)
        }
    }

    // MARK: Actions

    /// Starts a manual run, the same checks as every manual surface; asks first when it needs inputs or a source.
    public func runNow(_ workflow: WorkflowDefinition, context: ManualRunContext? = nil, targetNodeId: String? = nil) {
        startManualRun(workflow, context: context, targetNodeId: targetNodeId)
    }

    public func setEnabled(_ workflow: WorkflowDefinition, _ enabled: Bool) {
        if let i = workflows.firstIndex(where: { $0.id == workflow.id }) { workflows[i].enabled = enabled }
        perform { [backend] in try await backend.setEnabled(workflowId: workflow.id, enabled: enabled) }
    }

    public func delete(_ workflow: WorkflowDefinition) {
        if editingWorkflowId == workflow.id { showAllRuns() }
        workflows.removeAll { $0.id == workflow.id }
        perform { [backend] in try await backend.deleteWorkflow(id: workflow.id) }
    }

    public func retry(_ run: WorkflowExecution) {
        perform { [backend] in
            if let next = try await backend.retry(runId: run.runId) { self.upsert(next, select: true) }
        }
    }

    public func rerun(_ run: WorkflowExecution) {
        perform { [backend] in
            if let next = try await backend.rerun(runId: run.runId) { self.upsert(next, select: true) }
        }
    }

    public func stopRun(_ run: WorkflowExecution) {
        perform { [backend] in try await backend.stop(runId: run.runId) }
    }

    /// Answers a gate; a refusal shows as a toast and returns false, like the renderer's answerGate.
    @discardableResult
    public func resolveGate(
        _ run: WorkflowExecution, nodeId: String, decision: GateDecision, comment: String? = nil,
        edited: String? = nil
    ) async -> Bool {
        do {
            let result = try await backend.resolveGate(
                runId: run.runId, nodeId: nodeId, decision: decision, comment: comment, edited: edited)
            guard result.accepted else {
                toast = result.reason
                    ?? (decision == .changes
                        ? "This gate takes no more changes. Approve or reject it."
                        : "The gate did not take that answer.")
                return false
            }
            scheduleReload()
            return true
        } catch {
            toast = String(describing: error)
            return false
        }
    }

    func perform(_ work: @escaping @MainActor () async throws -> Void) {
        Task {
            do { try await work() } catch { toast = String(describing: error) }
        }
    }

    // MARK: Live updates

    func apply(_ event: WorkflowEvent) {
        switch event {
        case .runUpdated(let run):
            upsert(run)
            onRunUpdate?(run)
        case .runCompleted(let run):
            upsert(run)
            onRunUpdate?(run)
            scheduleReload()
        case .gateResolved:
            scheduleReload()
        case .workflowsChanged(let list):
            if let list { workflows = list } else { scheduleReload() }
        case .disconnected(let why):
            connectionError = why
        case .catalogChanged:
            catalogGeneration += 1
        case .schedulerMissed:
            // App-level: `followMissedSchedules` shows these whether or not this view is open.
            break
        }
    }

    /// Folds a live or returned run into the lists, keeping the name the server attached before.
    func upsert(_ run: WorkflowExecution, select: Bool = false) {
        var run = run
        if let i = runs.firstIndex(where: { $0.runId == run.runId }) {
            if run.workflowName == nil { run.workflowName = runs[i].workflowName }
            runs[i] = run
        } else {
            if run.workflowName == nil { run.workflowName = workflow(run.workflowId)?.name }
            runs.append(run)
        }
        sortRuns()
        if run.workflowId == editingWorkflowId {
            if let i = workflowRuns.firstIndex(where: { $0.runId == run.runId }) {
                workflowRuns[i] = run
            } else {
                workflowRuns.insert(run, at: 0)
            }
            workflowRuns.sort { $0.startedAt > $1.startedAt }
        }
        if select { selectedRunId = run.runId }
    }

    /// Replaces the persisted runs while keeping live ones the listing does not have yet.
    func mergePersisted(_ persisted: [WorkflowExecution]) {
        var byId = Dictionary(runs.map { ($0.runId, $0) }, uniquingKeysWith: { a, _ in a })
        for run in persisted { byId[run.runId] = run }
        let persistedIds = Set(persisted.map(\.runId))
        // Drop settled runs the listing no longer returns; live ones stay until it does.
        runs = byId.values.filter { persistedIds.contains($0.runId) || $0.status == .running }
        sortRuns()
    }

    private func sortRuns() {
        runs.sort { $0.startedAt > $1.startedAt }
    }

    func scheduleReload() {
        reloadTask?.cancel()
        reloadTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(150))
            guard !Task.isCancelled else { return }
            await self?.reload()
        }
    }

    /// Bumped when the connector catalog refreshes, so the Start from list reads it again.
    public var catalogGeneration = 0

    // Panels: workflow files (WorkflowFiles.swift).
    /// An import's unbound steps, waiting for the editor to show them.
    public var importedRequirements: ImportedRequirements?
    /// The save and open dialogs, replaced in tests and renders.
    @ObservationIgnored public var workflowFiles: WorkflowFileIO = .panels
}

/// A gate waiting on the user, with the run and workflow it belongs to.
public struct WaitingApproval: Identifiable, Hashable, Sendable {
    public let run: WorkflowExecution
    public let state: NodeExecutionState
    public let workflow: WorkflowDefinition?

    public var id: String { "\(run.runId):\(state.nodeId)" }

    /// What the gate asks: the run's word for it, else the step's configured message.
    public var message: String? {
        state.message ?? workflow?.nodes.first { $0.id == state.nodeId }?.config["message"]?.stringValue
    }
}
