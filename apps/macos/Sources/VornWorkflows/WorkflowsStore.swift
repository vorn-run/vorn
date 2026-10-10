import Foundation
import Observation
import VornCore

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
    public var historyOpen = true
    public var listWidth: CGFloat = 420
    public private(set) var loading = 0
    /// One transient message, shown like the app's error toast.
    public var toast: String?
    /// The open dropdown, drawn by `workflowsMenuHost`.
    public var menu: WorkflowsMenu?
    public var sidebarSectionOpen = true
    public private(set) var connectionError: String?

    @ObservationIgnored private let backend: WorkflowsBackend
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

    /// The row's workflow, or what the run remembers of a deleted one.
    public func ref(for run: WorkflowExecution) -> RunWorkflowRef? {
        if let w = workflow(run.workflowId) { return RunWorkflowRef(w) }
        if let d = run.definition { return RunWorkflowRef(d) }
        if let name = run.workflowName { return RunWorkflowRef(name: name) }
        return nil
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
        return Task { await loadWorkflowRuns(id) }
    }

    public func showAllRuns() {
        editingWorkflowId = nil
        workflowRuns = []
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

    /// Starts a manual run, the same checks as every manual surface.
    public func runNow(_ workflow: WorkflowDefinition) {
        guard workflow.triggerNode != nil else {
            toast = "\"\(workflow.name)\" has no trigger — add one in the editor first"
            return
        }
        perform { [backend] in
            if let run = try await backend.run(workflowId: workflow.id, inputs: nil) { self.upsert(run) }
        }
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

    private func perform(_ work: @escaping @MainActor () async throws -> Void) {
        Task {
            do { try await work() } catch { toast = String(describing: error) }
        }
    }

    // MARK: Live updates

    func apply(_ event: WorkflowEvent) {
        switch event {
        case .runUpdated(let run):
            upsert(run)
        case .runCompleted(let run):
            upsert(run)
            scheduleReload()
        case .gateResolved:
            scheduleReload()
        case .workflowsChanged(let list):
            if let list { workflows = list } else { scheduleReload() }
        case .disconnected(let why):
            connectionError = why
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

    private func scheduleReload() {
        reloadTask?.cancel()
        reloadTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(150))
            guard !Task.isCancelled else { return }
            await self?.reload()
        }
    }
}
