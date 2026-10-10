import SwiftUI
import VornCore

/// Which projects' tasks the board shows: the active project, or every project in the workspace.
public enum TasksScope: Equatable, Sendable {
    case all
    case project(String)
    case projects(Set<String>)

    func contains(_ name: String) -> Bool {
        switch self {
        case .all: true
        case .project(let p): p == name
        case .projects(let set): set.contains(name)
        }
    }

    var activeProject: String? {
        if case .project(let p) = self { return p }
        return nil
    }
}

public enum TaskStatusFilter: Hashable, Sendable {
    case all
    case status(TaskStatus)
}

public enum TaskSourceFilter: Hashable, Sendable {
    case all
    case local
    case connector(String)
}

public enum ToastKind: Sendable { case success, info, error }

/// The board's state and every action it takes on vornd.
@MainActor
@Observable
public final class TasksStore {
    public private(set) var tasks: [VornTask] = []
    public private(set) var projects: [VornProject] = []
    public private(set) var installedAgents: [AgentKind: Bool] = [:]
    public private(set) var viewMode: TaskViewMode = .list
    @ObservationIgnored private var localViewMode: TaskViewMode?
    public private(set) var loaded = false
    public private(set) var lastError: String?

    public var scope: TasksScope = .all
    public var statusFilter: TaskStatusFilter = .all
    public var sourceFilter: TaskSourceFilter = .all
    public var includeArchived = false

    /// The open detail panel: a task id, or `newTaskId` while creating one.
    public var selectedTaskId: String?
    public static let newTaskId = "new"

    /// The quick-add dialog; `dialogTask` is set when it edits rather than creates.
    public var dialogOpen = false
    public var dialogStatus: TaskStatus = .todo
    public var dialogTask: VornTask?

    public var viewOptionsOpen = false
    /// Where the view options button sits, in global coordinates; the panel hangs below its right edge.
    public var viewOptionsAnchor: CGRect?

    var popup: Popup?

    /// Hooks the shell fills in: a live session for a task, and opening it.
    public var isSessionLive: (VornTask) -> Bool = { _ in false }
    public var openSession: ((VornTask) -> Void)?
    public var onToast: (String, ToastKind) -> Void = { _, _ in }

    let service: any TaskService
    @ObservationIgnored private var watch: Task<Void, Never>?

    public init(service: any TaskService) {
        self.service = service
    }

    /// Loads the board and follows it until `stop()`.
    public func start() {
        guard watch == nil else { return }
        watch = Task { [weak self] in
            guard let self else { return }
            await self.reload()
            if let agents = try? await self.service.installedAgents() { self.installedAgents = agents }
            for await _ in await self.service.boardChanges() {
                await self.reload()
            }
        }
    }

    public func stop() {
        watch?.cancel()
        watch = nil
    }

    public func reload() async {
        do {
            async let t = service.listTasks()
            async let p = service.listProjects()
            async let m = service.taskViewMode()
            let (tasks, projects, mode) = try await (t, p, m)
            self.tasks = tasks
            self.projects = projects
            self.viewMode = localViewMode ?? mode
            loaded = true
            lastError = nil
        } catch {
            lastError = error.localizedDescription
        }
    }

    // MARK: - What the board shows

    var scopedProjects: [VornProject] {
        projects.filter { scope.contains($0.name) }
    }

    /// TaskBoardView's filter chain: scope, archived, status, then source.
    var visibleTasks: [VornTask] {
        tasks.filter { t in
            guard scope.contains(t.projectName) else { return false }
            if !includeArchived && t.isArchived { return false }
            if case .status(let s) = statusFilter, t.status != s { return false }
            switch sourceFilter {
            case .all: return true
            case .local: return t.sourceConnectorId == nil
            case .connector(let id): return t.sourceConnectorId == id
            }
        }
    }

    /// One status's tasks; the board sorts every column by order, the list only the queue.
    func tasks(in status: TaskStatus, sortedByOrder: Bool) -> [VornTask] {
        let column = visibleTasks.filter { $0.status == status }
        return sortedByOrder ? column.sorted { $0.order < $1.order } : column
    }

    var connectorIds: [String] {
        var seen: [String] = []
        for t in tasks { if let c = t.sourceConnectorId, !seen.contains(c) { seen.append(c) } }
        return seen
    }

    public var hasActiveFilters: Bool {
        statusFilter != .all || viewMode != .list || sourceFilter != .all || includeArchived
    }

    func task(id: String) -> VornTask? { tasks.first { $0.id == id } }

    var defaultProjectName: String {
        scope.activeProject ?? scopedProjects.first?.name ?? projects.first?.name ?? ""
    }

    // MARK: - Actions

    public func setViewMode(_ mode: TaskViewMode) {
        guard mode != viewMode else { return }
        localViewMode = nil
        viewMode = mode
        run { try await $0.setTaskViewMode(mode) }
    }

    /// Shows `mode` without saving it, for previews over a read-only connection.
    public func showViewMode(_ mode: TaskViewMode) {
        localViewMode = mode
        viewMode = mode
    }

    public func openNewTask(status: TaskStatus = .todo) {
        dialogTask = nil
        dialogStatus = status
        dialogOpen = true
    }

    func select(_ task: VornTask) { selectedTaskId = task.id }

    /// The drop rules of TaskBoardView.handleKanbanDrop.
    func drop(taskId: String, on status: TaskStatus) {
        guard let task = task(id: taskId), task.status != status else { return }
        switch status {
        case .inProgress:
            if task.status == .todo { setStatus(taskId, .inProgress) }
        case .inReview:
            setStatus(taskId, .inReview)
        case .done:
            complete(taskId)
        case .cancelled:
            cancel(taskId)
        case .todo:
            reopen(taskId, toast: false)
        }
    }

    /// Dropping a card on another card of its column moves it to that place.
    func move(taskId: String, before targetId: String) {
        guard taskId != targetId, let moving = task(id: taskId), let target = task(id: targetId) else { return }
        guard moving.status == target.status else {
            drop(taskId: taskId, on: target.status)
            return
        }
        var column = tasks.filter { $0.status == target.status && $0.projectName == moving.projectName }
            .sorted { $0.order < $1.order }
        guard column.contains(where: { $0.id == targetId }) else { return }
        column.removeAll { $0.id == taskId }
        let at = column.firstIndex { $0.id == targetId } ?? column.endIndex
        column.insert(moving, at: at)
        let slots = column.map(\.order).sorted()
        for (task, order) in zip(column, slots) {
            mutate(task.id) { $0.order = order }
        }
        let ids = column.map(\.id)
        run { try await $0.reorderTasks(ids: ids) }
    }

    func setStatus(_ id: String, _ status: TaskStatus) {
        let now = Self.now()
        mutate(id) {
            if status.isTerminal && !$0.status.isTerminal { $0.completedAt = now }
            if !status.isTerminal && $0.status.isTerminal { $0.completedAt = nil; $0.archivedAt = nil }
            $0.status = status
        }
        run { _ = try await $0.updateTask(id: id, TaskPatch(status: status)) }
    }

    func complete(_ id: String) {
        setStatus(id, .done)
        onToast("Task completed", .success)
    }

    func cancel(_ id: String) {
        setStatus(id, .cancelled)
        onToast("Task cancelled", .info)
    }

    func reopen(_ id: String, toast: Bool = true) {
        setStatus(id, .todo)
        if toast { onToast("Task reopened", .success) }
    }

    func moveToReview(_ id: String) {
        setStatus(id, .inReview)
        onToast("Task moved to review", .info)
    }

    func archive(_ id: String) {
        guard let t = task(id: id), t.status.isTerminal, !t.isArchived else { return }
        mutate(id) { $0.archivedAt = Self.now() }
        run { try await $0.archiveTask(id: id, archived: true) }
        onToast("Task archived", .success)
    }

    func unarchive(_ id: String) {
        guard task(id: id)?.isArchived == true else { return }
        mutate(id) { $0.archivedAt = nil }
        run { try await $0.archiveTask(id: id, archived: false) }
        onToast("Task unarchived", .success)
    }

    func delete(_ id: String) {
        tasks.removeAll { $0.id == id }
        if selectedTaskId == id { selectedTaskId = nil }
        run { try await $0.deleteTask(id: id) }
        onToast("Task deleted", .success)
    }

    /// Creates a task; the answer is its id once vornd has stored it.
    @discardableResult
    func create(_ draft: TaskDraft) async -> String? {
        do {
            let task = try await service.createTask(draft)
            if !tasks.contains(where: { $0.id == task.id }) { tasks.append(task) }
            onToast("Task created", .success)
            return task.id
        } catch {
            fail(error)
            return nil
        }
    }

    func update(_ id: String, _ patch: TaskPatch, toast: String? = nil) {
        mutate(id) { t in
            if let v = patch.projectName { t.projectName = v }
            if let v = patch.title { t.title = v }
            if let v = patch.description { t.description = v }
            if let v = patch.branch { t.branch = v.isEmpty ? nil : v }
            if let v = patch.useWorktree { t.useWorktree = v }
            if let v = patch.assignedAgent { t.assignedAgent = v } else if patch.clearsAgent { t.assignedAgent = nil }
        }
        if let status = patch.status, status != task(id: id)?.status { setStatus(id, status) }
        var stripped = patch
        stripped.status = nil
        let rest = stripped
        run { _ = try await $0.updateTask(id: id, rest) }
        if let toast { onToast(toast, .success) }
    }

    // MARK: - Plumbing

    private func mutate(_ id: String, _ change: (inout VornTask) -> Void) {
        guard let i = tasks.firstIndex(where: { $0.id == id }) else { return }
        change(&tasks[i])
        tasks[i].updatedAt = Self.now()
    }

    /// Sends a write; a refusal puts the board back as vornd has it.
    private func run(_ op: @escaping @Sendable (any TaskService) async throws -> Void) {
        let service = service
        Task {
            do {
                try await op(service)
            } catch {
                fail(error)
                await reload()
            }
        }
    }

    private func fail(_ error: Error) {
        lastError = error.localizedDescription
        onToast(error.localizedDescription, .error)
    }

    static func now() -> String {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f.string(from: Date())
    }
}
