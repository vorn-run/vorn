import Foundation
import Observation

/// The app's view of one vornd: its projects, workspaces and live sessions,
/// kept current from its notifications. Reconnects when the socket drops.
@MainActor
@Observable
public final class VornStore {
    public enum Phase: Equatable, Sendable {
        case idle
        case connecting
        case connected
        case failed(String)
    }

    public private(set) var phase: Phase = .idle
    public private(set) var endpoint: VornEndpoint?
    public private(set) var projects: [Project] = []
    public private(set) var workspaces: [Workspace] = [.personal]
    public private(set) var sessions: [TerminalSession] = []
    public private(set) var defaults: AppConfigSummary.Defaults?
    public private(set) var worktrees: [String: [Worktree]] = [:]
    public private(set) var gitRepos: [String: Bool] = [:]
    /// Sessions whose process exited on its own; their last screen stays up.
    public private(set) var endedIDs: Set<String> = []

    public var dataDir: URL { client.dataDir }
    /// Refuses every call that would change vornd's state.
    public var readOnly: Bool { client.readOnly }
    /// The connection, shared with the other views.
    public let client: VornClient

    @ObservationIgnored private let ownsClient: Bool
    @ObservationIgnored private var runTask: Task<Void, Never>?
    @ObservationIgnored private var closing: Set<String> = []

    /// The notification topics this app listens to.
    public static let topics = [
        "session:created", "session:updated", "session:reordered", "session-exit",
        "terminal:exit", "config:changed", "widget:status-update",
    ]

    public init(
        dataDir: URL = VornEndpoint.defaultDataDir(),
        readOnly: Bool = ProcessInfo.processInfo.environment["VORN_READ_ONLY"] == "1"
    ) {
        client = VornClient(dataDir: dataDir, readOnly: readOnly)
        ownsClient = true
    }

    /// A store over a connection the app shares; `stop()` leaves it open.
    public init(client: VornClient) {
        self.client = client
        ownsClient = false
    }

    /// A store holding fixed data and no connection, for previews and renders.
    public init(projects: [Project], sessions: [TerminalSession], workspaces: [Workspace] = [.personal]) {
        client = VornClient(dataDir: URL(fileURLWithPath: "/dev/null"), readOnly: true)
        ownsClient = true
        self.projects = projects
        self.sessions = sessions
        self.workspaces = workspaces
        phase = .connected
    }

    // MARK: Lifecycle

    /// Connects, and follows the connection (reloading after each reconnect) until `stop()`.
    public func start() {
        guard runTask == nil else { return }
        let client = client
        runTask = Task { [weak self] in
            let events = await client.events(Self.topics)
            let states = await client.stateUpdates()
            let applying = Task { [weak self] in
                for await event in events { self?.apply(event) }
            }
            defer { applying.cancel() }
            await client.start()
            for await state in states {
                guard let self, !Task.isCancelled else { return }
                await self.follow(state)
            }
        }
    }

    public func stop() {
        runTask?.cancel()
        runTask = nil
        if ownsClient { Task { [client] in await client.close() } }
    }

    private func follow(_ state: VornClient.State) async {
        switch state {
        case .idle: break
        case .connecting: phase = .connecting
        case .disconnected(let reason): phase = .failed(reason)
        case .connected:
            endpoint = await client.endpoint
            do {
                try await reload()
                phase = .connected
            } catch {
                phase = .failed(error.localizedDescription)
            }
        }
    }

    private func reload() async throws {
        let config = try await client.call("config:load", as: AppConfigSummary.self)
        applyConfig(config)
        projects = try await client.call("project:list", as: [Project].self)
        sessions = try await client.call("terminal:listActive", as: [TerminalSession].self)
        endedIDs.formIntersection(sessions.map(\.id))
        for project in projects { Task { await loadGitInfo(project.path) } }
    }

    private func applyConfig(_ config: AppConfigSummary) {
        defaults = config.defaults
        if let ws = config.workspaces, !ws.isEmpty { workspaces = ws.sorted { $0.order < $1.order } }
        if let p = config.projects { projects = p }
    }

    // MARK: Notifications

    /// Folds one notification into the state.
    public func apply(_ event: VornEvent) {
        guard case .notification(let method, let params) = event else { return }
        switch method {
        case "session:created", "session:updated":
            guard let s = try? params.decode(TerminalSession.self) else { return }
            upsert(s)
        case "session:reordered":
            guard let ids = try? params.decode([String].self) else { return }
            let rank = Dictionary(uniqueKeysWithValues: ids.enumerated().map { ($1, $0) })
            sessions.sort { (rank[$0.id] ?? Int.max) < (rank[$1.id] ?? Int.max) }
        case "widget:status-update":
            guard let updates = try? params.decode([SessionStatusUpdate].self) else { return }
            for u in updates {
                if let i = sessions.firstIndex(where: { $0.id == u.id }), sessions[i].status != u.status {
                    sessions[i].status = u.status
                }
            }
        case "terminal:exit":
            guard let id = params["id"]?.stringValue else { return }
            exited(id)
        case "session-exit":
            guard let s = try? params.decode(TerminalSession.self) else { return }
            exited(s.id)
        case "config:changed":
            guard let config = try? params.decode(AppConfigSummary.self) else { return }
            applyConfig(config)
        default:
            break
        }
    }

    private func upsert(_ s: TerminalSession) {
        if let i = sessions.firstIndex(where: { $0.id == s.id }) {
            if sessions[i] != s { sessions[i] = s }
        } else {
            sessions.append(s)
        }
    }

    /// A session's process ended: gone if this app closed it, else marked ended.
    private func exited(_ id: String) {
        if closing.remove(id) != nil {
            sessions.removeAll { $0.id == id }
            endedIDs.remove(id)
        } else if let i = sessions.firstIndex(where: { $0.id == id }) {
            sessions[i].status = .idle
            endedIDs.insert(id)
        }
    }

    // MARK: Actions

    /// Opens a plain shell in `cwd` (`shell:create`).
    @discardableResult
    public func createShell(cwd: String?, project: Project? = nil, worktree: Worktree? = nil) async throws -> TerminalSession {
        var s = try await client.call("shell:create", cwd.map { .string($0) }).decode(TerminalSession.self)
        if let project {
            s.projectName = project.name
            s.projectPath = project.path
        }
        if let worktree {
            s.worktreePath = worktree.path
            s.worktreeName = worktree.name
            s.branch = worktree.branch
            s.isWorktree = worktree.path != (project?.path ?? s.projectPath)
        }
        upsert(s)
        return s
    }

    /// Starts an agent session (`terminal:create`).
    @discardableResult
    public func createSession(_ payload: CreateTerminalPayload) async throws -> TerminalSession {
        let s = try await client.call("terminal:create", payload, as: TerminalSession.self)
        upsert(s)
        return s
    }

    /// Closes a session (`terminal:kill`).
    public func close(_ id: String) async throws {
        closing.insert(id)
        do {
            try await client.call("terminal:kill", .string(id))
        } catch {
            closing.remove(id)
            throw error
        }
        if endedIDs.contains(id) { exited(id) }
    }

    public func rename(_ id: String, to name: String) async throws {
        try await client.call("terminal:rename", .object(["id": .string(id), "displayName": .string(name)]))
        if let i = sessions.firstIndex(where: { $0.id == id }) { sessions[i].displayName = name }
    }

    /// Learns whether `path` is a git repository and, if so, its worktrees.
    public func loadGitInfo(_ path: String) async {
        let isRepo = (try? await client.call("git:isGitRepo", .string(path)).decode(Bool.self)) ?? false
        gitRepos[path] = isRepo
        guard isRepo else { return }
        if let wts = try? await client.call("git:listWorktrees", .string(path)).decode([Worktree].self) {
            worktrees[path] = wts
        }
    }

    // MARK: Queries

    public func session(_ id: String) -> TerminalSession? { sessions.first { $0.id == id } }

    public func project(named name: String) -> Project? { projects.first { $0.name == name } }

    public func sessions(in project: Project) -> [TerminalSession] {
        sessions.filter { $0.projectPath == project.path || $0.projectName == project.name }
    }
}
