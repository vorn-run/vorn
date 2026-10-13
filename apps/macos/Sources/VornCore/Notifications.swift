import Foundation

/// `defaults.notifications` in the config: what the user wants to hear about.
public struct NotificationPrefs: Decodable, Sendable, Equatable {
    public var enabled: Bool?
    public var onWaiting: Bool?
    public var onError: Bool?
    public var onBell: Bool?
    public var soundEnabled: Bool?
    public var soundVolume: Double?

    public init(enabled: Bool? = nil, onWaiting: Bool? = nil, onError: Bool? = nil, onBell: Bool? = nil,
                soundEnabled: Bool? = nil, soundVolume: Double? = nil) {
        self.enabled = enabled
        self.onWaiting = onWaiting
        self.onError = onError
        self.onBell = onBell
        self.soundEnabled = soundEnabled
        self.soundVolume = soundVolume
    }

    private enum Keys: String, CodingKey { case enabled, onWaiting, onError, onBell, soundEnabled, soundVolume }

    /// A mistyped field reads as unset rather than failing the whole config.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        enabled = try? c.decodeIfPresent(Bool.self, forKey: .enabled)
        onWaiting = try? c.decodeIfPresent(Bool.self, forKey: .onWaiting)
        onError = try? c.decodeIfPresent(Bool.self, forKey: .onError)
        onBell = try? c.decodeIfPresent(Bool.self, forKey: .onBell)
        soundEnabled = try? c.decodeIfPresent(Bool.self, forKey: .soundEnabled)
        soundVolume = try? c.decodeIfPresent(Double.self, forKey: .soundVolume)
    }

    /// A bell or a program's own notification (shouldNotifyBell).
    public var wantsBell: Bool { enabled == true && onBell != false }
    /// A workflow gate or sign-in that started waiting.
    public var wantsWorkflowWait: Bool { enabled == true && onWaiting != false }
    public var volume: Double { soundVolume ?? 0.5 }
}

/// Which tone a notification plays (notifications.ts NotificationReason).
public enum NotificationReason: String, Sendable, CaseIterable {
    case waiting, error, bell

    /// The sine tone's pitch and length (SOUND_PROFILES).
    public var tone: (frequency: Double, duration: Double) {
        switch self {
        case .waiting: (880, 0.15)
        case .error: (330, 0.3)
        case .bell: (660, 0.2)
        }
    }
}

/// One notification to show: what it says, how it sounds, and where a click goes.
public struct AppNotice: Sendable, Equatable {
    public enum Target: Sendable, Equatable {
        case session(String)
        case run(String)
        case none
    }

    public var reason: NotificationReason
    /// Repeats under one key inside the cooldown are not shown again.
    public var key: String
    public var title: String
    public var body: String
    public var target: Target

    public init(reason: NotificationReason, key: String, title: String, body: String, target: Target) {
        self.reason = reason
        self.key = key
        self.title = title
        self.body = body
        self.target = target
    }

    /// An agent's own notice, in today's words (sendAgentNotification).
    public static func agent(_ session: TerminalSession, _ reason: NotificationReason) -> AppNotice {
        let agent = session.agentType.displayName
        let name = session.title
        let (title, body): (String, String) = switch reason {
        case .waiting: ("\(agent) needs input", "\(name) is waiting for your response")
        case .error: ("\(agent) error", "\(name) encountered an error")
        case .bell: ("\(agent) notification", "\(name) is requesting your attention")
        }
        return AppNotice(reason: reason, key: session.id, title: title, body: body, target: .session(session.id))
    }

    /// What a program in the terminal asked to show (sendProgramNotification).
    public static func program(_ session: TerminalSession, title: String, body: String) -> AppNotice {
        AppNotice(reason: .bell, key: "notify:\(session.id)", title: title.isEmpty ? session.title : title,
                  body: body, target: .session(session.id))
    }
}

/// Holds back a repeat of the same notice for ten seconds (COOLDOWN_MS).
public struct NoticeCooldown: Sendable {
    public static let interval: TimeInterval = 10
    private var last: [String: Date] = [:]

    public init() {}

    /// Whether `key` may show now; a yes starts its cooldown.
    public mutating func admit(_ key: String, at now: Date = Date()) -> Bool {
        if let at = last[key], now.timeIntervalSince(at) < Self.interval { return false }
        last[key] = now
        return true
    }
}
