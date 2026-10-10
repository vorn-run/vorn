import Foundation

/// Which program a session runs. A kind this build does not know decodes as
/// `.other`, so a session is never dropped for its kind.
public enum AgentType: Codable, Hashable, Sendable, CaseIterable {
    case claude, copilot, codex, opencode, gemini, shell
    case other(String)

    public static let allCases: [AgentType] = [.claude, .copilot, .codex, .opencode, .gemini, .shell]
    /// The AI agents, in the order today's pickers list them.
    public static let agents: [AgentType] = [.claude, .copilot, .codex, .opencode, .gemini]

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
