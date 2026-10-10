import Foundation

/// `TaskStatus` in packages/shared/src/types.ts.
public enum TaskStatus: String, Codable, CaseIterable, Sendable, Hashable {
    case todo
    case inProgress = "in_progress"
    case inReview = "in_review"
    case done
    case cancelled

    /// Done and cancelled: work that is over, and the only states a task may be archived from.
    public var isTerminal: Bool { self == .done || self == .cancelled }
}

/// `AiAgentType`: the agents a task can be assigned to.
public enum AgentKind: String, Codable, CaseIterable, Sendable, Hashable {
    case claude, copilot, codex, opencode, gemini

    public var label: String {
        switch self {
        case .claude: "Claude"
        case .copilot: "Copilot"
        case .codex: "Codex"
        case .opencode: "OpenCode"
        case .gemini: "Gemini"
        }
    }
}

/// `TaskConfig`: one row of the task board.
public struct VornTask: Codable, Identifiable, Hashable, Sendable {
    public var id: String
    public var projectName: String
    public var title: String
    public var description: String
    public var status: TaskStatus
    public var order: Double
    public var assignedSessionId: String?
    public var assignedAgent: AgentKind?
    public var agentSessionId: String?
    public var branch: String?
    public var useWorktree: Bool?
    public var worktreePath: String?
    public var images: [String]?
    public var createdAt: String
    public var updatedAt: String
    public var completedAt: String?
    public var archivedAt: String?
    public var sourceConnectorId: String?
    public var sourceExternalId: String?
    public var sourceExternalUrl: String?

    public init(
        id: String, projectName: String, title: String, description: String = "",
        status: TaskStatus = .todo, order: Double = 0, assignedAgent: AgentKind? = nil,
        branch: String? = nil, useWorktree: Bool? = nil, createdAt: String, updatedAt: String? = nil,
        completedAt: String? = nil, archivedAt: String? = nil
    ) {
        self.id = id
        self.projectName = projectName
        self.title = title
        self.description = description
        self.status = status
        self.order = order
        self.assignedAgent = assignedAgent
        self.branch = branch
        self.useWorktree = useWorktree
        self.createdAt = createdAt
        self.updatedAt = updatedAt ?? createdAt
        self.completedAt = completedAt
        self.archivedAt = archivedAt
    }

    enum CodingKeys: String, CodingKey {
        case id, projectName, title, description, status, order, assignedSessionId, assignedAgent
        case agentSessionId, branch, useWorktree, worktreePath, images, createdAt, updatedAt
        case completedAt, archivedAt, sourceConnectorId, sourceExternalId, sourceExternalUrl
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        projectName = try c.decodeIfPresent(String.self, forKey: .projectName) ?? ""
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        description = try c.decodeIfPresent(String.self, forKey: .description) ?? ""
        status = (try? c.decode(TaskStatus.self, forKey: .status)) ?? .todo
        order = (try? c.decode(Double.self, forKey: .order)) ?? 0
        assignedSessionId = try? c.decodeIfPresent(String.self, forKey: .assignedSessionId)
        // An agent this client does not know is shown as unassigned rather than failing the board.
        assignedAgent = try? c.decodeIfPresent(AgentKind.self, forKey: .assignedAgent)
        agentSessionId = try? c.decodeIfPresent(String.self, forKey: .agentSessionId)
        branch = try? c.decodeIfPresent(String.self, forKey: .branch)
        useWorktree = try? c.decodeIfPresent(Bool.self, forKey: .useWorktree)
        worktreePath = try? c.decodeIfPresent(String.self, forKey: .worktreePath)
        images = try? c.decodeIfPresent([String].self, forKey: .images)
        createdAt = (try? c.decode(String.self, forKey: .createdAt)) ?? ""
        updatedAt = (try? c.decode(String.self, forKey: .updatedAt)) ?? createdAt
        completedAt = try? c.decodeIfPresent(String.self, forKey: .completedAt)
        archivedAt = try? c.decodeIfPresent(String.self, forKey: .archivedAt)
        sourceConnectorId = try? c.decodeIfPresent(String.self, forKey: .sourceConnectorId)
        sourceExternalId = try? c.decodeIfPresent(String.self, forKey: .sourceExternalId)
        sourceExternalUrl = try? c.decodeIfPresent(String.self, forKey: .sourceExternalUrl)
    }

    public var isArchived: Bool { archivedAt != nil }

    /// `getTaskShortId`: three letters of the project and four of the id, e.g. `VOR-1A2B`.
    public var shortId: String {
        let letters = projectName.filter { ("a"..."z").contains($0) || ("A"..."Z").contains($0) }
        let prefix = letters.prefix(3).uppercased()
        return "\(prefix.isEmpty ? "TSK" : prefix)-\(id.prefix(4).uppercased())"
    }
}

/// `ProjectConfig`, the fields a task board reads.
public struct VornProject: Codable, Identifiable, Hashable, Sendable {
    public var name: String
    public var path: String
    public var icon: String?
    public var iconColor: String?
    public var workspaceId: String?

    public var id: String { name }

    public init(name: String, path: String, icon: String? = nil, iconColor: String? = nil, workspaceId: String? = nil) {
        self.name = name
        self.path = path
        self.icon = icon
        self.iconColor = iconColor
        self.workspaceId = workspaceId
    }
}

/// `TaskViewMode`, kept in `config.defaults.taskViewMode`.
public enum TaskViewMode: String, Codable, Sendable {
    case list
    case kanban
}
