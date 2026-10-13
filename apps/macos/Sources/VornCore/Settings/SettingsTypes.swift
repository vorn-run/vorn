import Foundation

/// A shell found on this machine (`shell:listInstalled`).
public struct InstalledShell: Codable, Hashable, Sendable, Identifiable {
    public struct Blocks: Codable, Hashable, Sendable {
        public var level: String
        public var limitation: String?
        public init(level: String, limitation: String? = nil) {
            self.level = level
            self.limitation = limitation
        }
    }

    public var family: String
    public var name: String
    public var path: String
    public var version: String?
    public var blocks: Blocks
    public var id: String { path }

    public init(family: String, name: String, path: String, version: String? = nil, blocks: Blocks) {
        self.family = family
        self.name = name
        self.path = path
        self.version = version
        self.blocks = blocks
    }
}

/// A stored SSH key, without the secret (`credential:listKeys`).
public struct SSHKeyMeta: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var label: String
    public var keyType: String?
    public var publicKey: String?
    public var createdAt: String?

    public init(id: String, label: String, keyType: String? = nil, publicKey: String? = nil, createdAt: String? = nil) {
        self.id = id
        self.label = label
        self.keyType = keyType
        self.publicKey = publicKey
        self.createdAt = createdAt
    }
}

/// A secret a connection holds (`connection:listKeys`).
public struct ConnectorKey: Codable, Hashable, Sendable, Identifiable {
    public struct Field: Codable, Hashable, Sendable {
        public var key: String
        public var label: String
        public var readable: Bool
        public var hint: String?
        public var envNames: [String]?
        public init(key: String, label: String, readable: Bool, hint: String? = nil, envNames: [String]? = nil) {
            self.key = key
            self.label = label
            self.readable = readable
            self.hint = hint
            self.envNames = envNames
        }
    }

    public var connectionId: String
    public var name: String
    public var connectorId: String
    public var fields: [Field]
    public var usageCount: Int
    public var id: String { connectionId }

    public init(connectionId: String, name: String, connectorId: String, fields: [Field], usageCount: Int) {
        self.connectionId = connectionId
        self.name = name
        self.connectorId = connectorId
        self.fields = fields
        self.usageCount = usageCount
    }
}

/// What `core:status` reports.
public struct CoreStatus: Codable, Hashable, Sendable {
    public var loaded: Bool
    public var version: String?
    public var error: String?
    public init(loaded: Bool, version: String?, error: String? = nil) {
        self.loaded = loaded
        self.version = version
        self.error = error
    }
}

/// One vorn-sessiond vornd knows about (`/vornd/health`).
public struct SessionHolder: Codable, Hashable, Sendable {
    public var pid: Int
    public var instance: String
    public var build: String
    public var proto: Int
    public var sessions: Int?
    public var compatible: Bool

    public init(pid: Int, instance: String, build: String, proto: Int = 0, sessions: Int?, compatible: Bool) {
        self.pid = pid
        self.instance = instance
        self.build = build
        self.proto = proto
        self.sessions = sessions
        self.compatible = compatible
    }
}

public struct SessionHolders: Hashable, Sendable {
    public var current: SessionHolder?
    public var older: [SessionHolder]
    public var error: String?

    public init(current: SessionHolder?, older: [SessionHolder], error: String?) {
        self.current = current
        self.older = older
        self.error = error
    }

    /// Reads the `sessiond` object of vornd's health answer, keeping only well-formed holders.
    public static func parse(_ raw: JSONValue?) -> SessionHolders? {
        guard let raw, case .object(let r) = raw else { return nil }
        func holder(_ v: JSONValue?) -> SessionHolder? {
            guard let o = v?.objectValue, let pid = o["pid"]?.numberValue, let inst = o["instance"]?.stringValue else { return nil }
            return SessionHolder(pid: Int(pid), instance: inst, build: o["build"]?.stringValue ?? "",
                                 proto: Int(o["proto"]?.numberValue ?? 0), sessions: o["sessions"]?.numberValue.map { Int($0) },
                                 compatible: o["compatible"]?.boolValue == true)
        }
        return SessionHolders(current: holder(r["current"]), older: (r["older"]?.arrayValue ?? []).compactMap(holder),
                              error: r["error"]?.stringValue)
    }
}

// MARK: Worktree inventory

public struct WorktreeInventory: Codable, Hashable, Sendable {
    public var projects: [WorktreeProjectInventory]
    public var scannedAt: String?
    public init(projects: [WorktreeProjectInventory], scannedAt: String? = nil) {
        self.projects = projects
        self.scannedAt = scannedAt
    }
}

public struct WorktreeProjectInventory: Codable, Hashable, Sendable {
    public var projectPath: String
    public var projectName: String
    public var defaultBranch: String?
    public var remoteHostId: String?
    public var entries: [WorktreeInventoryEntry]
    public var staleBranches: [StaleBranch]
    public var error: String?

    public init(projectPath: String, projectName: String, defaultBranch: String?, remoteHostId: String? = nil,
                entries: [WorktreeInventoryEntry], staleBranches: [StaleBranch], error: String? = nil) {
        self.projectPath = projectPath
        self.projectName = projectName
        self.defaultBranch = defaultBranch
        self.remoteHostId = remoteHostId
        self.entries = entries
        self.staleBranches = staleBranches
        self.error = error
    }
}

public struct WorktreeVerdict: Codable, Hashable, Sendable {
    public var level: String
    public var freesBytes: Double
    public var reasons: [String]
    public var autoSelect: Bool
    public init(level: String, freesBytes: Double = 0, reasons: [String] = [], autoSelect: Bool = false) {
        self.level = level
        self.freesBytes = freesBytes
        self.reasons = reasons
        self.autoSelect = autoSelect
    }
}

public struct WorktreeInventoryEntry: Codable, Hashable, Sendable, Identifiable {
    public var path: String
    public var name: String
    public var projectPath: String
    public var projectName: String
    public var kind: String
    public var branch: String?
    public var isMain: Bool
    public var sizeBytes: Double
    public var artifactBytes: Double
    public var sizeMeasured: Bool
    public var idleDays: Int?
    public var isDirty: Bool
    public var isMerged: Bool
    public var hasUpstream: Bool
    public var activeSessionIds: [String]
    public var verdict: WorktreeVerdict
    public var id: String { path }

    public var isSelectable: Bool { !isMain && activeSessionIds.isEmpty }

    public init(path: String, name: String, projectPath: String, projectName: String, kind: String = "registered",
                branch: String?, isMain: Bool = false, sizeBytes: Double, artifactBytes: Double = 0,
                sizeMeasured: Bool = true, idleDays: Int? = nil, isDirty: Bool = false, isMerged: Bool = false,
                hasUpstream: Bool = false, activeSessionIds: [String] = [], verdict: WorktreeVerdict) {
        self.path = path
        self.name = name
        self.projectPath = projectPath
        self.projectName = projectName
        self.kind = kind
        self.branch = branch
        self.isMain = isMain
        self.sizeBytes = sizeBytes
        self.artifactBytes = artifactBytes
        self.sizeMeasured = sizeMeasured
        self.idleDays = idleDays
        self.isDirty = isDirty
        self.isMerged = isMerged
        self.hasUpstream = hasUpstream
        self.activeSessionIds = activeSessionIds
        self.verdict = verdict
    }
}

public struct StaleBranch: Codable, Hashable, Sendable {
    public var name: String
    public var isMerged: Bool
    public var hasUpstream: Bool
    public init(name: String, isMerged: Bool, hasUpstream: Bool = false) {
        self.name = name
        self.isMerged = isMerged
        self.hasUpstream = hasUpstream
    }
}

public struct WorktreeActionResult: Codable, Hashable, Sendable {
    public struct Failure: Codable, Hashable, Sendable {
        public var path: String
        public var error: String
        public init(path: String, error: String) {
            self.path = path
            self.error = error
        }
    }

    public var succeeded: [String]
    public var failed: [Failure]
    public var freedBytes: Double
    public var deletedBranches: [String]

    public init(succeeded: [String] = [], failed: [Failure] = [], freedBytes: Double = 0, deletedBranches: [String] = []) {
        self.succeeded = succeeded
        self.failed = failed
        self.freedBytes = freedBytes
        self.deletedBranches = deletedBranches
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        succeeded = try c.decodeIfPresent([String].self, forKey: .succeeded) ?? []
        failed = try c.decodeIfPresent([Failure].self, forKey: .failed) ?? []
        freedBytes = try c.decodeIfPresent(Double.self, forKey: .freedBytes) ?? 0
        deletedBranches = try c.decodeIfPresent([String].self, forKey: .deletedBranches) ?? []
    }
}

/// format-bytes.ts.
public func formatBytes(_ bytes: Double) -> String {
    guard bytes.isFinite, bytes > 0 else { return "0 B" }
    let units = ["B", "KB", "MB", "GB", "TB"]
    var value = bytes
    var unit = 0
    while value >= 1024 && unit < units.count - 1 {
        value /= 1024
        unit += 1
    }
    let decimals = unit <= 1 ? 0 : value < 10 ? 1 : 0
    return String(format: "%.\(decimals)f %@", value, units[unit])
}

// MARK: Updates

/// The updater's state (`UpdateStatus` in types.ts).
public enum UpdateStatus: Hashable, Sendable {
    case idle(lastCheckedAt: Double?)
    case checking
    case available(version: String)
    case downloading(version: String?, percent: Int)
    case ready(version: String)
    case error(String)
    case unsupported

    public var isChecking: Bool { if case .checking = self { true } else { false } }
    public var isUnsupported: Bool { if case .unsupported = self { true } else { false } }
    public var isPending: Bool { if case .ready = self { true } else { false } }
}

/// update-status.ts: what a status says, and the one action worth offering.
public struct UpdateStatusView: Hashable, Sendable {
    public enum Tone: Sendable { case blocked, broken, live, settled, idle }
    public enum Action: Sendable { case restart, download, retry }

    public var label: String
    public var shortLabel: String
    public var detail: String?
    public var tone: Tone
    public var percent: Int?
    public var action: Action?

    public static func lastChecked(_ at: Double?, now: Double = Date().timeIntervalSince1970 * 1000) -> String? {
        guard let at else { return nil }
        let seconds = max(0, Int((now - at) / 1000))
        if seconds < 60 { return "checked just now" }
        let minutes = seconds / 60
        if minutes < 60 { return "checked \(minutes) minute\(minutes == 1 ? "" : "s") ago" }
        let hours = minutes / 60
        if hours < 24 { return "checked \(hours) hour\(hours == 1 ? "" : "s") ago" }
        let days = hours / 24
        return "checked \(days) day\(days == 1 ? "" : "s") ago"
    }

    public init(_ status: UpdateStatus, channel: String = "stable", now: Double = Date().timeIntervalSince1970 * 1000) {
        switch status {
        case .idle(let at):
            self.init(label: "Up to date", shortLabel: "Up to date",
                      detail: Self.lastChecked(at, now: now) ?? "\(channel) channel", tone: .idle)
        case .checking:
            self.init(label: "Checking for updates…", shortLabel: "Checking…", detail: "\(channel) channel", tone: .live)
        case .available(let v):
            self.init(label: "Version \(v) is available", shortLabel: "v\(v) available", detail: "not downloaded yet",
                      tone: .idle, action: .download)
        case .downloading(let v, let p):
            self.init(label: v.map { "Downloading \($0)" } ?? "Downloading update", shortLabel: "Downloading \(p)%",
                      detail: nil, tone: .live, percent: p)
        case .ready(let v):
            self.init(label: "Version \(v) is ready to install", shortLabel: "v\(v) ready",
                      detail: "Vorn closes for a few seconds and reopens itself", tone: .blocked, action: .restart)
        case .error(let m):
            self.init(label: "Couldn't update", shortLabel: "Update failed", detail: m, tone: .broken, action: .retry)
        case .unsupported:
            self.init(label: "Updates are off in development", shortLabel: "Updates off", detail: "packaged builds only", tone: .idle)
        }
    }

    init(label: String, shortLabel: String, detail: String?, tone: Tone, percent: Int? = nil, action: Action? = nil) {
        self.label = label
        self.shortLabel = shortLabel
        self.detail = detail
        self.tone = tone
        self.percent = percent
        self.action = action
    }
}

/// update-cost.ts: what restarting to update does to the sessions on screen.
public func updateCostLine(sessionCount: Int, aTurnIsRunning: Bool, sessionsSurvive: Bool) -> String? {
    guard sessionCount > 0 else { return nil }
    let one = sessionCount == 1
    let subject = one ? "Your session" : "Your \(sessionCount) sessions"
    let s = one ? "s" : ""
    let fate = sessionsSurvive ? "keep\(s) running through the update" : "end\(s) with the update"
    let turn = !aTurnIsRunning ? "" : sessionsSurvive ? " The turn in flight continues." : " The turn in flight is cut short."
    return "\(subject) \(fate).\(turn)"
}
