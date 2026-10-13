import Foundation

/// Which program a session runs. A kind this build does not know decodes as
/// `.other`, so a session is never dropped for its kind.
public enum AgentType: Codable, Hashable, Sendable, CaseIterable {
    case claude, copilot, codex, opencode, gemini, shell
    case other(String)

    public static let allCases: [AgentType] = [.claude, .copilot, .codex, .opencode, .gemini, .shell]
    /// The AI agents, in the order today's pickers list them.
    public static let agents: [AgentType] = [.claude, .copilot, .codex, .opencode, .gemini]

    /// Whether the agent takes a model choice (agent-models.ts).
    public var choosesModel: Bool { [.claude, .copilot, .opencode, .codex].contains(self) }

    public init(rawValue: String) {
        switch rawValue {
        case "claude": self = .claude
        case "copilot": self = .copilot
        case "codex": self = .codex
        case "opencode": self = .opencode
        case "gemini": self = .gemini
        case "shell": self = .shell
        default: self = .other(rawValue)
        }
    }

    public var rawValue: String {
        switch self {
        case .claude: "claude"
        case .copilot: "copilot"
        case .codex: "codex"
        case .opencode: "opencode"
        case .gemini: "gemini"
        case .shell: "shell"
        case .other(let s): s
        }
    }

    public var displayName: String {
        switch self {
        case .claude: "Claude Code"
        case .copilot: "GitHub Copilot"
        case .codex: "Codex CLI"
        case .opencode: "OpenCode"
        case .gemini: "Gemini CLI"
        case .shell: "Shell"
        case .other(let s): s
        }
    }

    public init(from decoder: Decoder) throws {
        self.init(rawValue: try decoder.singleValueContainer().decode(String.self))
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encode(rawValue)
    }
}

public enum AgentStatus: String, Codable, Hashable, Sendable {
    case running, waiting, idle, error

    public init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = AgentStatus(rawValue: raw) ?? .idle
    }

    /// The word today's sidebar shows for it.
    public var label: String {
        switch self {
        case .running: "Running"
        case .waiting: "Waiting"
        case .idle: "Idle"
        case .error: "Error"
        }
    }
}

/// A live terminal session, as vornd reports it (`TerminalSession` in types.ts).
public struct TerminalSession: Codable, Identifiable, Hashable, Sendable {
    public var id: String
    public var agentType: AgentType
    public var projectName: String
    public var projectPath: String
    public var status: AgentStatus
    public var createdAt: Double
    public var pid: Int
    public var displayName: String?
    public var branch: String?
    public var worktreePath: String?
    public var worktreeName: String?
    public var isWorktree: Bool?
    public var remoteHostId: String?
    public var groupId: String?
    public var cols: Int?
    public var rows: Int?
    public var workspaceId: String?
    public var shellCwd: String?
    public var shellExitCode: Int?

    public init(
        id: String, agentType: AgentType, projectName: String, projectPath: String,
        status: AgentStatus = .running, createdAt: Double = 0, pid: Int = 0,
        displayName: String? = nil, branch: String? = nil, worktreePath: String? = nil,
        worktreeName: String? = nil, isWorktree: Bool? = nil, workspaceId: String? = nil
    ) {
        self.id = id
        self.agentType = agentType
        self.projectName = projectName
        self.projectPath = projectPath
        self.status = status
        self.createdAt = createdAt
        self.pid = pid
        self.displayName = displayName
        self.branch = branch
        self.worktreePath = worktreePath
        self.worktreeName = worktreeName
        self.isWorktree = isWorktree
        self.workspaceId = workspaceId
    }

    /// The name cards and the sidebar show (`getDisplayName`).
    public var title: String {
        if let name = displayName?.trimmingCharacters(in: .whitespaces), !name.isEmpty { return name }
        return projectName
    }
}

/// A project (`ProjectConfig` in types.ts).
public struct Project: Codable, Identifiable, Hashable, Sendable {
    public var name: String
    public var path: String
    public var preferredAgents: [String]
    public var icon: String?
    public var iconColor: String?
    public var hostIds: [String]?
    public var workspaceId: String?

    public var id: String { name }

    public init(name: String, path: String, preferredAgents: [String] = [], icon: String? = nil,
                iconColor: String? = nil, workspaceId: String? = nil) {
        self.name = name
        self.path = path
        self.preferredAgents = preferredAgents
        self.icon = icon
        self.iconColor = iconColor
        self.workspaceId = workspaceId
    }

    /// `workspaceId`, defaulting to the personal workspace as today.
    public var workspace: String { workspaceId ?? Workspace.personalID }

    private enum CodingKeys: String, CodingKey {
        case name, path, preferredAgents, icon, iconColor, hostIds, workspaceId
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        name = try c.decode(String.self, forKey: .name)
        path = try c.decode(String.self, forKey: .path)
        preferredAgents = try c.decodeIfPresent([String].self, forKey: .preferredAgents) ?? []
        icon = try c.decodeIfPresent(String.self, forKey: .icon)
        iconColor = try c.decodeIfPresent(String.self, forKey: .iconColor)
        hostIds = try c.decodeIfPresent([String].self, forKey: .hostIds)
        workspaceId = try c.decodeIfPresent(String.self, forKey: .workspaceId)
    }
}

/// A workspace (`WorkspaceConfig` in types.ts).
public struct Workspace: Codable, Identifiable, Hashable, Sendable {
    public static let personalID = "personal"
    public static let personal = Workspace(id: personalID, name: "Personal", icon: "User", iconColor: "#6b7280", order: 0)

    public var id: String
    public var name: String
    public var icon: String?
    public var iconColor: String?
    public var order: Double

    public init(id: String, name: String, icon: String? = nil, iconColor: String? = nil, order: Double = 0) {
        self.id = id
        self.name = name
        self.icon = icon
        self.iconColor = iconColor
        self.order = order
    }
}

/// A git worktree of a project (`git:listWorktrees`).
public struct Worktree: Codable, Hashable, Sendable, Identifiable {
    public var path: String
    public var branch: String
    public var isMain: Bool
    public var name: String

    public var id: String { path }

    public init(path: String, branch: String, isMain: Bool, name: String) {
        self.path = path
        self.branch = branch
        self.isMain = isMain
        self.name = name
    }
}

/// The parts of `AppConfig` this app reads. The rest stays on the server.
public struct AppConfigSummary: Decodable, Sendable, Equatable {
    public struct Defaults: Decodable, Sendable, Equatable {
        public var defaultAgent: AgentType?
        public var mainViewMode: String?
        public var fontSize: Double?
        public var layoutMode: String?
        public var reopenSessions: Bool?
        public var showHeadlessAgents: Bool?
        /// Which onboarding version was dismissed; App.tsx reads it with `Number()`.
        public var hasSeenOnboarding: JSONValue?
        /// Whether the floating widget may show; index.ts only honours a literal `true`.
        public var widgetEnabled: JSONValue?

        public init(defaultAgent: AgentType? = nil, layoutMode: String? = nil, reopenSessions: Bool? = nil,
                    showHeadlessAgents: Bool? = nil) {
            self.defaultAgent = defaultAgent
            self.layoutMode = layoutMode
            self.reopenSessions = reopenSessions
            self.showHeadlessAgents = showHeadlessAgents
        }
    }

    public var defaults: Defaults?
    public var projects: [Project]?
    public var workspaces: [Workspace]?

    public init(defaults: Defaults? = nil, projects: [Project]? = nil, workspaces: [Workspace]? = nil) {
        self.defaults = defaults
        self.projects = projects
        self.workspaces = workspaces
    }
}

/// What `terminal:create` takes (`CreateTerminalPayload` in types.ts).
public struct CreateTerminalPayload: Encodable, Sendable, Equatable {
    public var agentType: AgentType
    public var projectName: String
    public var projectPath: String
    public var displayName: String?
    public var branch: String?
    public var useWorktree: Bool?
    public var existingWorktreePath: String?
    public var worktreeName: String?
    public var initialPrompt: String?
    public var resumeSessionId: String?

    public init(agentType: AgentType, projectName: String, projectPath: String,
                displayName: String? = nil, branch: String? = nil, useWorktree: Bool? = nil,
                existingWorktreePath: String? = nil, worktreeName: String? = nil,
                initialPrompt: String? = nil) {
        self.agentType = agentType
        self.projectName = projectName
        self.projectPath = projectPath
        self.displayName = displayName
        self.branch = branch
        self.useWorktree = useWorktree
        self.existingWorktreePath = existingWorktreePath
        self.worktreeName = worktreeName
        self.initialPrompt = initialPrompt
    }
}

/// One entry of `widget:status-update`.
public struct SessionStatusUpdate: Decodable, Sendable {
    public var id: String
    public var status: AgentStatus
}

/// What the server found when it checked a restored record against the tree.
public struct RestoreEnvironment: Codable, Hashable, Sendable {
    public struct Pair: Codable, Hashable, Sendable {
        public var recorded: String?
        public var actual: String?

        public init(recorded: String?, actual: String?) {
            self.recorded = recorded
            self.actual = actual
        }
    }

    public var worktree: String
    public var branch: Pair
    public var head: Pair

    public init(worktree: String, branch: Pair, head: Pair) {
        self.worktree = worktree
        self.branch = branch
        self.head = head
    }

    public var worktreeMissing: Bool { worktree == "missing" }

    /// `headMoved` in types.ts.
    public var headMoved: Bool {
        guard let r = head.recorded, let a = head.actual else { return false }
        return r != a
    }
}

/// Why a pane has nothing running behind it (`EndedSession` in the renderer).
public struct EndedInfo: Hashable, Sendable {
    public enum Reason: String, Sendable {
        case exited
        case appClosed = "app-closed"
        case machineRestarted = "machine-restarted"
        case serverStopped = "server-stopped"
    }

    public var reason: Reason
    /// Unix ms.
    public var at: Double
    public var replayed: Bool
    public var partial: Bool
    public var exitCode: Int?
    public var cwd: String?
    public var environment: RestoreEnvironment?

    public init(reason: Reason, at: Double, replayed: Bool, partial: Bool = false, exitCode: Int? = nil,
                cwd: String? = nil, environment: RestoreEnvironment? = nil) {
        self.reason = reason
        self.at = at
        self.replayed = replayed
        self.partial = partial
        self.exitCode = exitCode
        self.cwd = cwd
        self.environment = environment
    }

    /// The words after "Ended ·" (EndedStrip.tsx).
    public var cause: String {
        switch reason {
        case .exited: exitCode.map { "exited with \($0)" } ?? "exited"
        case .appClosed: "Vorn was closed"
        case .machineRestarted: "the machine restarted"
        case .serverStopped: "the server stopped unexpectedly"
        }
    }
}

/// A session a previous run left behind (`sessions:restored`).
public struct RestoredSession: Decodable, Hashable, Sendable {
    public var session: TerminalSession
    public var endedAt: Double
    public var replayable: Bool
    public var partial: Bool
    public var closedCleanly: Bool
    public var rebooted: Bool
    public var environment: RestoreEnvironment?

    public init(session: TerminalSession, endedAt: Double, replayable: Bool = true, partial: Bool = false,
                closedCleanly: Bool = true, rebooted: Bool = false, environment: RestoreEnvironment? = nil) {
        self.session = session
        self.endedAt = endedAt
        self.replayable = replayable
        self.partial = partial
        self.closedCleanly = closedCleanly
        self.rebooted = rebooted
        self.environment = environment
    }

    /// `endedFromRestored`: one reading of a record, so the board and the strip agree.
    public var ended: EndedInfo {
        EndedInfo(reason: closedCleanly ? .appClosed : rebooted ? .machineRestarted : .serverStopped,
                  at: endedAt, replayed: replayable, partial: partial, cwd: session.shellCwd,
                  environment: environment)
    }
}

/// What `sessions:resume` answers.
public struct ResumeResult: Decodable, Sendable {
    public var ok: Bool
    public var session: TerminalSession?
    public var boundTo: String?
    public var reason: String?
    public var message: String?
}

/// An agent conversation on disk that can be resumed (`sessions:getRecent`).
public struct RecentSession: Decodable, Identifiable, Hashable, Sendable {
    public var sessionId: String
    public var agentType: AgentType
    public var display: String
    public var projectPath: String
    public var timestamp: Double
    public var activityCount: Int
    public var activityLabel: String
    public var canResumeExact: Bool

    public var id: String { sessionId }

    public init(sessionId: String, agentType: AgentType, display: String, projectPath: String, timestamp: Double,
                activityCount: Int, activityLabel: String, canResumeExact: Bool) {
        self.sessionId = sessionId
        self.agentType = agentType
        self.display = display
        self.projectPath = projectPath
        self.timestamp = timestamp
        self.activityCount = activityCount
        self.activityLabel = activityLabel
        self.canResumeExact = canResumeExact
    }

    /// `formatRecentSessionActivity`: "3 prompts", "1 reply".
    public var activity: String {
        var label = activityLabel
        if activityCount != 1 { label = label.hasSuffix("y") ? String(label.dropLast()) + "ies" : label + "s" }
        return "\(activityCount) \(label)"
    }
}

/// An agent running without a terminal (`headless:list`).
public struct HeadlessSession: Decodable, Identifiable, Hashable, Sendable {
    public var id: String
    public var pid: Int
    public var agentType: AgentType
    public var projectName: String
    public var projectPath: String
    public var displayName: String?
    public var branch: String?
    public var status: String
    public var exitCode: Int?
    public var startedAt: Double
    public var endedAt: Double?
    public var workflowId: String?
    public var workflowName: String?

    public init(id: String, agentType: AgentType, projectName: String, projectPath: String = "",
                status: String = "running", exitCode: Int? = nil, startedAt: Double, endedAt: Double? = nil,
                displayName: String? = nil, branch: String? = nil) {
        self.id = id
        self.pid = 0
        self.agentType = agentType
        self.projectName = projectName
        self.projectPath = projectPath
        self.status = status
        self.exitCode = exitCode
        self.startedAt = startedAt
        self.endedAt = endedAt
        self.displayName = displayName
        self.branch = branch
    }

    public var isRunning: Bool { status == "running" }
    public var isError: Bool { status == "exited" && exitCode != 0 }

    /// The status the grid's filter reads it as.
    public var filterStatus: AgentStatus { isRunning ? .running : isError ? .error : .idle }
}

/// Uncommitted changes in a tree (`git:diffStat`).
public struct DiffStat: Codable, Hashable, Sendable {
    public var filesChanged: Int
    public var insertions: Int
    public var deletions: Int

    public init(filesChanged: Int, insertions: Int, deletions: Int) {
        self.filesChanged = filesChanged
        self.insertions = insertions
        self.deletions = deletions
    }
}

/// A repo's local branches (`git:listBranches`).
public struct BranchList: Decodable, Hashable, Sendable {
    public var local: [String]
    public var current: String?
    public var isGitRepo: Bool?
}
