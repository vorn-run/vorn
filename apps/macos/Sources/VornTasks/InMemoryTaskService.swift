import Foundation
import VornCore

/// A board held in memory with vornd's task rules, for previews and tests.
public actor InMemoryTaskService: TaskService {
    public private(set) var tasks: [VornTask]
    public private(set) var projects: [VornProject]
    public private(set) var mode: TaskViewMode
    public private(set) var calls: [String] = []
    private var agents: [AgentKind: Bool]
    private var listeners: [UUID: AsyncStream<Void>.Continuation] = [:]

    public init(tasks: [VornTask] = [], projects: [VornProject] = [], mode: TaskViewMode = .list,
                agents: [AgentKind: Bool] = [.claude: true, .copilot: true, .codex: true]) {
        self.tasks = tasks
        self.projects = projects
        self.mode = mode
        self.agents = agents
    }

    public func listTasks() async throws -> [VornTask] { tasks.sorted { $0.order < $1.order } }

    public func getTask(id: String) async throws -> VornTask? { tasks.first { $0.id == id } }

    public func createTask(_ d: TaskDraft) async throws -> VornTask {
        calls.append("create")
        guard projects.contains(where: { $0.name == d.projectName }) else { throw VornError.refused("No project") }
        let now = Self.now()
        let order = (tasks.filter { $0.projectName == d.projectName }.map(\.order).max() ?? -1) + 1
        var t = VornTask(id: UUID().uuidString.lowercased(), projectName: d.projectName, title: d.title,
                         description: d.description ?? "", status: d.status ?? .todo, order: order,
                         assignedAgent: d.assignedAgent, branch: d.branch, useWorktree: d.useWorktree, createdAt: now)
        if t.status.isTerminal { t.completedAt = now }
        tasks.append(t)
        changed()
        return t
    }

    public func updateTask(id: String, _ p: TaskPatch) async throws -> VornTask? {
        calls.append("update")
        guard let i = tasks.firstIndex(where: { $0.id == id }) else { throw VornError.refused("No task") }
        var t = tasks[i]
        if let v = p.projectName, v != t.projectName {
            t.projectName = v
            t.order = (tasks.filter { $0.projectName == v }.map(\.order).max() ?? -1) + 1
        }
        if let v = p.title { t.title = v }
        if let v = p.description { t.description = v }
        if let v = p.branch { t.branch = v.isEmpty ? nil : v }
        if let v = p.useWorktree { t.useWorktree = v }
        if let v = p.assignedAgent { t.assignedAgent = v } else if p.clearsAgent { t.assignedAgent = nil }
        if let s = p.status {
            if s.isTerminal && !t.status.isTerminal { t.completedAt = Self.now() }
            if !s.isTerminal && t.status.isTerminal { t.completedAt = nil; t.archivedAt = nil }
            t.status = s
        }
        t.updatedAt = Self.now()
        tasks[i] = t
        changed()
        return t
    }

    public func deleteTask(id: String) async throws {
        calls.append("delete")
        tasks.removeAll { $0.id == id }
        changed()
    }

    public func archiveTask(id: String, archived: Bool) async throws {
        calls.append(archived ? "archive" : "unarchive")
        guard let i = tasks.firstIndex(where: { $0.id == id }), tasks[i].status.isTerminal || !archived else {
            throw VornError.refused("Only done or cancelled tasks can be archived")
        }
        tasks[i].archivedAt = archived ? Self.now() : nil
        changed()
    }

    public func reorderTasks(ids: [String]) async throws {
        calls.append("reorder")
        let slots = ids.compactMap { id in tasks.first { $0.id == id }?.order }.sorted()
        for (id, order) in zip(ids, slots) {
            if let i = tasks.firstIndex(where: { $0.id == id }) { tasks[i].order = order }
        }
        changed()
    }

    public func listProjects() async throws -> [VornProject] { projects }
    public func taskViewMode() async throws -> TaskViewMode { mode }

    public func setTaskViewMode(_ mode: TaskViewMode) async throws {
        calls.append("viewMode")
        self.mode = mode
        changed()
    }

    public func installedAgents() async throws -> [AgentKind: Bool] { agents }

    public func boardChanges() async -> AsyncStream<Void> {
        let (stream, continuation) = AsyncStream<Void>.makeStream()
        let id = UUID()
        listeners[id] = continuation
        continuation.onTermination = { _ in Task { await self.drop(id) } }
        return stream
    }

    private func drop(_ id: UUID) { listeners[id] = nil }

    private func changed() {
        for c in listeners.values { c.yield(()) }
    }

    static func now() -> String {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f.string(from: Date())
    }
}

public extension InMemoryTaskService {
    /// A board with something in every column, for previews and screenshots.
    static func sample() -> InMemoryTaskService {
        let projects = [
            VornProject(name: "vorn", path: "/tmp/vorn", icon: "Rocket", iconColor: "#C9972A"),
            VornProject(name: "website", path: "/tmp/website", icon: "Globe", iconColor: "#6F8FAF"),
        ]
        func t(_ id: String, _ title: String, _ s: TaskStatus, _ o: Double, _ a: AgentKind?, _ d: String,
               archived: Bool = false) -> VornTask {
            var task = VornTask(id: id, projectName: o < 100 ? "vorn" : "website", title: title, status: s, order: o,
                                assignedAgent: a, createdAt: d)
            if s.isTerminal { task.completedAt = d }
            if archived { task.archivedAt = d }
            return task
        }
        let tasks = [
            t("a1f3c2d0", "Port the task board to the native app", .todo, 0, .claude, "2025-10-09T10:00:00.000Z"),
            t("b2e4d1c9", "Keyboard navigation between columns", .todo, 1, nil, "2025-10-10T10:00:00.000Z"),
            t("c3d5e2b8", "Archive tasks older than thirty days automatically", .todo, 2, .codex, "2025-10-11T10:00:00.000Z"),
            t("d4c6f3a7", "Stream the cell grid into the terminal view", .inProgress, 3, .claude, "2025-10-07T10:00:00.000Z"),
            t("e5b7a4f6", "Reconnect the client after vornd restarts", .inProgress, 4, .copilot, "2025-10-08T10:00:00.000Z"),
            t("f6a8b5e5", "Review the workflow runs panel", .inReview, 5, .claude, "2025-10-05T10:00:00.000Z"),
            t("07f9c6d4", "Theme tokens from theme.css", .done, 6, .codex, "2025-10-01T10:00:00.000Z"),
            t("18e0d7c3", "Lucide glyphs as SwiftUI paths", .done, 7, nil, "2025-10-02T10:00:00.000Z"),
            t("29d1e8b2", "Menu bar parity with the web client", .cancelled, 8, nil, "2025-09-28T10:00:00.000Z"),
            t("3ac2f9a1", "Landing page hero copy", .todo, 100, .gemini, "2025-10-12T10:00:00.000Z"),
        ]
        return InMemoryTaskService(tasks: tasks, projects: projects)
    }
}
