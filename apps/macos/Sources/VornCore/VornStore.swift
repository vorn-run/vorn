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
    /// Panes with nothing running behind them, and why; their last screen stays up.
    public private(set) var ended: [String: EndedInfo] = [:]
    /// What previous runs left behind and the server still holds (`sessions:restored`).
    public private(set) var restored: [RestoredSession] = []
    /// Agents running without a terminal (`headless:list`).
    public private(set) var headless: [HeadlessSession] = []
    /// Headless sessions this window chose to hide.
    public var dismissedHeadless: Set<String> = []
    /// Agents found on this machine (`agent:detectInstalled`).
    public private(set) var installedAgents: [AgentType] = []
    /// Uncommitted changes per tree, refreshed while a card shows it.
    public private(set) var diffStats: [String: DiffStat] = [:]

    /// When each session last changed status (unix ms), the Mac's stand-in for the renderer's last output time.
    public private(set) var lastActivity: [String: Double] = [:]

    public var endedIDs: Set<String> { Set(ended.keys) }

    public var dataDir: URL { client.dataDir }
    /// Refuses every call that would change vornd's state.
    public var readOnly: Bool { client.readOnly }
    /// The connection, shared with the other views.
    public let client: VornClient

    @ObservationIgnored private let ownsClient: Bool
    @ObservationIgnored private var runTask: Task<Void, Never>?
    @ObservationIgnored private var closing: Set<String> = []
    @ObservationIgnored private var gitLoads: [String: Int] = [:]

    /// The notification topics this app listens to.
    public static let topics = [
        "session:created", "session:updated", "session:reordered", "session-exit",
        "terminal:exit", "config:changed", "widget:status-update", "headless:exit", "headless:data",
        "worktree:confirmCleanup", "terminal:bell", "terminal:notify",
    ]

    /// The last session in a worktree ended (`worktree:confirmCleanup`): project path, worktree path.
    @ObservationIgnored public var onWorktreeCleanup: (String, String) -> Void = { _, _ in }
    /// A terminal rang its bell or a program in it asked to notify.
    @ObservationIgnored public var onNotice: (AppNotice) -> Void = { _ in }
    /// Why the last load after connecting failed; the app shows its error screen over it.
    public private(set) var loadError: String?

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
            await load()
        }
    }

    private func load() async {
        do {
            try await reload()
            loadError = nil
            phase = .connected
        } catch {
            loadError = String(describing: error)
            phase = .failed(error.localizedDescription)
        }
    }

    /// The error screen's Reload: reads everything again over the open connection.
    public func retryLoad() async {
        loadError = nil
        await load()
    }

    private func reload() async throws {
        let config = try await client.call("config:load", as: AppConfigSummary.self)
        applyConfig(config)
        projects = try await client.call("project:list", as: [Project].self)
        let active = try await client.call("terminal:listActive", as: [TerminalSession].self)
        let carried = (try? await client.call("sessions:restored", as: [RestoredSession].self)) ?? []
        restored = carried
        // Panes this store holds for sessions the server no longer runs, then the ones ended before it connected.
        let live = Set(active.map(\.id))
        let held = Dictionary(carried.map { ($0.session.id, $0) }, uniquingKeysWith: { a, _ in a })
        var board = active
        // Only what this pass found stopped is started again, never a session that stopped on its own.
        var justEnded: [String] = []
        for s in sessions where !live.contains(s.id) {
            board.append(s)
            if ended[s.id] == nil {
                justEnded.append(s.id)
                ended[s.id] = held[s.id]?.ended ?? EndedInfo(reason: .serverStopped, at: Date().timeIntervalSince1970 * 1000,
                                                             replayed: false, cwd: s.shellCwd)
            }
        }
        let reopen = defaults?.reopenSessions ?? true
        if reopen {
            for one in carried where !live.contains(one.session.id) && !board.contains(where: { $0.id == one.session.id }) {
                board.append(one.session)
                ended[one.session.id] = one.ended
                justEnded.append(one.session.id)
            }
        }
        sessions = board
        ended = ended.filter { id, _ in board.contains { $0.id == id } }
        for id in live { ended[id] = nil }
        if reopen && !readOnly && !justEnded.isEmpty {
            Task { await resumeAll(justEnded) }
        }
        for project in projects { Task { await loadGitInfo(project.path) } }
        Task { await reloadHeadless() }
        if installedAgents.isEmpty, let found = try? await client.call("agent:detectInstalled", as: [String: Bool].self) {
            installedAgents = AgentType.agents.filter { found[$0.rawValue] == true }
        }
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
            guard var s = try? params.decode(TerminalSession.self) else { return }
            // As the renderer does, a broadcast keeps the project and worktree this app filed a shell under.
            if let held = session(s.id) {
                s.projectName = held.projectName
                s.projectPath = held.projectPath
                if s.worktreePath == nil {
                    s.worktreePath = held.worktreePath
                    s.isWorktree = held.isWorktree
                }
                s.worktreeName = s.worktreeName ?? held.worktreeName
                s.branch = s.branch ?? held.branch
            }
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
                    lastActivity[u.id] = Date().timeIntervalSince1970 * 1000
                }
            }
        case "terminal:exit":
            guard let id = params["id"]?.stringValue else { return }
            exited(id)
        case "session-exit":
            guard let s = try? params.decode(TerminalSession.self) else { return }
            exited(s.id)
        case "headless:exit":
            guard let id = params["id"]?.stringValue, let i = headless.firstIndex(where: { $0.id == id }) else {
                Task { await reloadHeadless() }
                return
            }
            headless[i].status = "exited"
            headless[i].exitCode = params["exitCode"]?.numberValue.map { Int($0) }
            headless[i].endedAt = Date().timeIntervalSince1970 * 1000
        case "headless:data":
            if let id = params["id"]?.stringValue, !headless.contains(where: { $0.id == id }) {
                Task { await reloadHeadless() }
            }
        case "config:changed":
            guard let config = try? params.decode(AppConfigSummary.self) else { return }
            applyConfig(config)
        case "terminal:bell":
            guard let id = params["id"]?.stringValue, let s = session(id) else { return }
            onNotice(.agent(s, .bell))
        case "terminal:notify":
            guard let id = params["id"]?.stringValue, let s = session(id) else { return }
            onNotice(.program(s, title: params["title"]?.stringValue ?? "", body: params["body"]?.stringValue ?? ""))
        case "worktree:confirmCleanup":
            guard let p = params["projectPath"]?.stringValue, let w = params["worktreePath"]?.stringValue else { return }
            onWorktreeCleanup(p, w)
        default:
            break
        }
    }

    private func upsert(_ s: TerminalSession) {
        ended[s.id] = nil
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
            ended[id] = nil
        } else if let i = sessions.firstIndex(where: { $0.id == id }) {
            sessions[i].status = .idle
            let s = sessions[i]
            ended[id] = EndedInfo(reason: .exited, at: Date().timeIntervalSince1970 * 1000, replayed: false,
                                  exitCode: s.shellExitCode, cwd: s.shellCwd)
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
        if ended[id] != nil { exited(id) }
    }

    /// Starts an ended session again, in the pane it was sitting in (`resumeEndedSession`).
    public func resume(_ id: String) async throws {
        let result = try await client.call("sessions:resume", .object(["id": .string(id)]), as: ResumeResult.self)
        guard result.ok, let s = result.session else {
            if result.reason == "gone" {
                sessions.removeAll { $0.id == id }
                ended[id] = nil
                throw VornStoreError("That session was resumed somewhere else")
            }
            throw VornStoreError(result.message ?? "Could not resume that session")
        }
        restored.removeAll { $0.session.id == id }
        ended[id] = nil
        if result.boundTo != nil, s.id != id, sessions.contains(where: { $0.id == s.id }) {
            sessions.removeAll { $0.id == id }
        } else if let i = sessions.firstIndex(where: { $0.id == id }) {
            sessions[i] = s
        } else {
            upsert(s)
        }
    }

    /// One at a time, so they come back in order; a failure leaves the pane's strip as the offer to retry.
    private func resumeAll(_ ids: [String]) async {
        for id in ids where ended[id] != nil {
            try? await resume(id)
        }
    }

    /// Puts an ended session from a previous run on screen without starting anything (`showEndedSession`).
    public func showRestored(_ id: String) {
        guard let one = restored.first(where: { $0.session.id == id }) else { return }
        if !sessions.contains(where: { $0.id == id }) { sessions.append(one.session) }
        ended[id] = one.ended
    }

    /// Restored records with no pane yet: what the banner offers.
    public var coldRestored: [RestoredSession] {
        restored.filter { one in !sessions.contains { $0.id == one.session.id } }
    }

    /// Conversations on disk that can be resumed, newest first (`sessions:getRecent`).
    public func recentSessions(projectPath: String?) async throws -> [RecentSession] {
        try await client.call("sessions:getRecent", projectPath.map { .string($0) } ?? .null, as: [RecentSession].self)
    }

    public func reloadHeadless() async {
        if let list = try? await client.call("headless:list", as: [HeadlessSession].self) { headless = list }
    }

    /// Asks vornd to end every session and exit; it may drop the socket before it answers.
    public func stopServer() async throws {
        let client = client
        _ = try? await withThrowingTaskGroup(of: Void.self) { group in
            group.addTask { _ = try await client.call("server:shutdown") }
            group.addTask { try await Task.sleep(for: .seconds(5)) }
            try await group.next()
            group.cancelAll()
        }
    }

    public func killHeadless(_ id: String) async throws {
        try await client.call("headless:kill", .string(id))
    }

    /// Re-reads a tree's uncommitted change counts.
    /// Re-reads a tree's uncommitted change counts, and the branch its sessions are on (useGitDiffPolling).
    public func refreshDiffStat(_ path: String) async {
        if let stat = try? await client.call("git:diffStat", .string(path)).decode(DiffStat?.self) {
            if diffStats[path] != stat { diffStats[path] = stat }
        }
        if let branch = try? await client.call("git:getBranch", .string(path)).stringValue,
           sessions.contains(where: { ($0.worktreePath ?? $0.projectPath) == path && $0.branch != branch }) {
            setBranch(branch, forCwd: path)
        }
    }

    /// setBranchForCwd: every session in that tree is now on `branch`.
    public func setBranch(_ branch: String, forCwd cwd: String) {
        for i in sessions.indices where (sessions[i].worktreePath ?? sessions[i].projectPath) == cwd {
            sessions[i].branch = branch
        }
    }

    public func listBranches(_ path: String) async throws -> BranchList {
        try await client.call("git:listBranches", .string(path), as: BranchList.self)
    }

    /// Creates a worktree off `branch` and reloads the project's list.
    @discardableResult
    public func createWorktree(projectPath: String, branch: String, name: String? = nil) async throws -> Worktree {
        var params: [String: JSONValue] = ["projectPath": .string(projectPath), "branch": .string(branch)]
        if let name { params["worktreeName"] = .string(name) }
        let r = try await client.call("git:createWorktree", .object(params))
        let wt = Worktree(path: r["worktreePath"]?.stringValue ?? "", branch: r["branch"]?.stringValue ?? branch,
                          isMain: false, name: r["name"]?.stringValue ?? name ?? branch)
        await loadGitInfo(projectPath)
        return wt
    }

    public func renameWorktree(projectPath: String, worktreePath: String, to name: String) async throws {
        try await client.call("git:renameWorktree", .object(["worktreePath": .string(worktreePath), "newName": .string(name)]))
        await loadGitInfo(projectPath)
    }

    public func removeWorktree(projectPath: String, worktreePath: String, force: Bool = false) async throws {
        try await client.call("git:removeWorktree", .object([
            "projectPath": .string(projectPath), "worktreePath": .string(worktreePath), "force": .bool(force),
        ]))
        await loadGitInfo(projectPath)
    }

    /// Saves the full config after `change` edits it, as the renderer's `patchConfig` does.
    public func updateConfig(_ change: (inout [String: JSONValue]) -> Void) async throws {
        guard case .object(var config) = try await client.call("config:load") else {
            throw VornStoreError("The config could not be read")
        }
        change(&config)
        try await client.call("config:save", .object(config))
        if let summary = try? JSONValue.object(config).decode(AppConfigSummary.self) { applyConfig(summary) }
    }

    /// Adds a project to the config (`addProject`).
    public func addProject(_ project: Project) async throws {
        try await updateConfig { config in
            config["projects"] = .array((config["projects"]?.arrayValue ?? []) + [.object(Self.entry(project))])
        }
        await loadGitInfo(project.path)
    }

    /// Replaces the project named `originalName` (`updateProject`), keeping keys the form does not edit.
    public func updateProject(_ originalName: String, _ project: Project) async throws {
        try await updateConfig { config in
            config["projects"] = .array((config["projects"]?.arrayValue ?? []).map { old in
                guard old["name"]?.stringValue == originalName else { return old }
                return .object((old.objectValue ?? [:]).merging(Self.entry(project)) { $1 })
            })
        }
        await loadGitInfo(project.path)
    }

    private static func entry(_ project: Project) -> [String: JSONValue] {
        var entry: [String: JSONValue] = [
            "name": .string(project.name), "path": .string(project.path),
            "preferredAgents": .array(project.preferredAgents.map { .string($0) }),
        ]
        if let icon = project.icon { entry["icon"] = .string(icon) }
        if let color = project.iconColor { entry["iconColor"] = .string(color) }
        if let hosts = project.hostIds { entry["hostIds"] = .array(hosts.map { .string($0) }) }
        if let ws = project.workspaceId { entry["workspaceId"] = .string(ws) }
        return entry
    }

    /// Whether a worktree has uncommitted changes; nil when git could not tell.
    public func worktreeDirty(_ path: String) async -> Bool? {
        try? await client.call("git:worktreeDirty", .string(path)).decode(Bool.self)
    }

    public func removeProject(named name: String) async throws {
        try await updateConfig { config in
            config["projects"] = .array((config["projects"]?.arrayValue ?? []).filter { $0["name"]?.stringValue != name })
        }
    }

    /// Writes one key of `defaults`.
    public func setDefault(_ key: String, _ value: JSONValue) async throws {
        try await updateConfig { config in
            var defaults = config["defaults"]?.objectValue ?? [:]
            defaults[key] = value
            config["defaults"] = .object(defaults)
        }
    }

    /// Persists a manual order (`terminal:reorder`).
    public func reorder(_ ids: [String]) async throws {
        let rank = Dictionary(ids.enumerated().map { ($1, $0) }, uniquingKeysWith: { a, _ in a })
        sessions.sort { (rank[$0.id] ?? Int.max) < (rank[$1.id] ?? Int.max) }
        try await client.call("terminal:reorder", .array(ids.map { .string($0) }))
    }

    /// Types `data` into a session's terminal (`terminal:write`).
    public func write(_ id: String, _ data: String) async throws {
        try await client.notify("terminal:write", .object(["id": .string(id), "data": .string(data)]))
    }

    public func rename(_ id: String, to name: String) async throws {
        try await client.call("terminal:rename", .object(["id": .string(id), "displayName": .string(name)]))
        if let i = sessions.firstIndex(where: { $0.id == id }) { sessions[i].displayName = name }
    }

    /// Learns whether `path` is a git repository and, if so, its worktrees.
    public func loadGitInfo(_ path: String) async {
        gitLoads[path, default: 0] += 1
        let load = gitLoads[path]
        let isRepo = (try? await client.call("git:isGitRepo", .string(path)).decode(Bool.self)) ?? false
        guard gitLoads[path] == load else { return }
        gitRepos[path] = isRepo
        guard isRepo else { return }
        let wts = try? await client.call("git:listWorktrees", .string(path)).decode([Worktree].self)
        // A newer load for the same path wins, so a slow stale answer cannot bring back a removed worktree.
        if let wts, gitLoads[path] == load { worktrees[path] = wts }
    }

    /// Fixture state for renders and tests that have no server to ask.
    public func stage(ended: [String: EndedInfo]? = nil, restored: [RestoredSession]? = nil,
                      headless: [HeadlessSession]? = nil, diffStats: [String: DiffStat]? = nil,
                      installedAgents: [AgentType]? = nil) {
        if let ended { self.ended = ended }
        if let restored { self.restored = restored }
        if let headless { self.headless = headless }
        if let diffStats { self.diffStats = diffStats }
        if let installedAgents { self.installedAgents = installedAgents }
    }

    // MARK: Queries

    public func session(_ id: String) -> TerminalSession? { sessions.first { $0.id == id } }

    public func project(named name: String) -> Project? { projects.first { $0.name == name } }

    public func sessions(in project: Project) -> [TerminalSession] {
        sessions.filter { $0.projectPath == project.path || $0.projectName == project.name }
    }
}


public struct VornStoreError: LocalizedError, Sendable {
    public let message: String
    public init(_ message: String) { self.message = message }
    public var errorDescription: String? { message }
}
