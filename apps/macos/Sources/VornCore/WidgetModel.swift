import CoreGraphics
import Foundation

/// widget-glow.ts: the halo colour around a running agent's icon, as red, green, blue (0–255) and alpha.
public struct WidgetGlow: Equatable, Sendable {
    public let red: Double
    public let green: Double
    public let blue: Double
    public let alpha: Double

    public init(_ red: Double, _ green: Double, _ blue: Double, _ alpha: Double) {
        self.red = red
        self.green = green
        self.blue = blue
        self.alpha = alpha
    }

    /// widget-glow.ts `glowColorForAgent`.
    public static func of(_ agent: AgentType) -> WidgetGlow {
        switch agent {
        case .claude: WidgetGlow(217, 119, 87, 0.45)
        case .copilot: WidgetGlow(255, 255, 255, 0.3)
        case .codex: WidgetGlow(122, 157, 255, 0.45)
        case .opencode: WidgetGlow(255, 255, 255, 0.3)
        case .gemini: WidgetGlow(49, 134, 255, 0.45)
        case .shell, .other: WidgetGlow(156, 163, 175, 0.35)
        }
    }

    /// The CSS the renderer writes, `rgba(r,g,b,a)`.
    public var css: String {
        func n(_ v: Double) -> String { v == v.rounded() ? String(Int(v)) : String(v) }
        return "rgba(\(n(red)),\(n(green)),\(n(blue)),\(n(alpha)))"
    }
}

/// Widget.tsx: one row of the widget, a live session (`WidgetAgentInfo`).
public struct WidgetAgent: Identifiable, Hashable, Sendable {
    public let id: String
    public let agentType: AgentType
    public let displayName: String?
    public let projectName: String
    public let status: AgentStatus

    public init(id: String, agentType: AgentType, displayName: String?, projectName: String, status: AgentStatus) {
        self.id = id
        self.agentType = agentType
        self.displayName = displayName
        self.projectName = projectName
        self.status = status
    }

    public init(_ session: TerminalSession) {
        self.init(id: session.id, agentType: session.agentType, displayName: session.displayName,
                  projectName: session.projectName, status: session.status)
    }

    /// `displayName || projectName`: an empty name falls back too.
    public var label: String { (displayName?.isEmpty == false ? displayName : nil) ?? projectName }
    /// The row's tooltip.
    public var tooltip: String { "\(projectName) — \(status.rawValue)" }
}

/// Widget.tsx: how many rows are running, waiting and failed, as the header and the pill count them.
public struct WidgetCounts: Equatable, Sendable {
    public var running = 0
    public var waiting = 0
    public var error = 0
    public var total = 0

    public init(_ agents: [WidgetAgent]) {
        total = agents.count
        for a in agents {
            switch a.status {
            case .running: running += 1
            case .waiting: waiting += 1
            case .error: error += 1
            case .idle: break
            }
        }
    }
}

/// index.ts: the widget's two sizes and how it moves between them.
public enum WidgetViewMode: String, Sendable {
    case full, compact

    /// index.ts `VIEW_SIZES`.
    public var size: CGSize {
        switch self {
        case .full: CGSize(width: 280, height: 400)
        case .compact: CGSize(width: 140, height: 36)
        }
    }
}

/// index.ts: where the widget sits, in a top-left origin space like the renderer's screen.
public enum WidgetPlacement {
    /// The gap kept from the work area's edges.
    public static let margin: CGFloat = 20

    /// `createWidgetWindow`: the bottom-right corner of the work area.
    public static func initial(_ size: CGSize, in area: CGRect) -> CGRect {
        CGRect(x: area.maxX - size.width - margin, y: area.maxY - size.height - margin,
               width: size.width, height: size.height)
    }

    /// `widget:set-view-mode`: the new size keeps the bottom-right corner, then stays on screen.
    public static func resized(_ frame: CGRect, to size: CGSize, in area: CGRect) -> CGRect {
        var x = frame.minX + (frame.width - size.width)
        var y = frame.minY + (frame.height - size.height)
        x = max(area.minX, min(x, area.maxX - size.width))
        y = max(area.minY, min(y, area.maxY - size.height))
        return CGRect(x: x, y: y, width: size.width, height: size.height)
    }
}

/// One question an agent asks through `AskUserQuestion`.
public struct WidgetQuestion: Hashable, Sendable {
    public let header: String?
    public let question: String
    public let options: [String]
}

/// Widget.tsx: a permission request held open by an agent (`PermissionRequestInfo`).
public struct PermissionRequest: Identifiable, Hashable, Sendable {
    public enum ContextKind: Sendable { case file, command, text }

    public let requestId: String
    public let sessionId: String?
    public let terminalId: String?
    public let toolName: String
    public let toolInput: [String: JSONValue]
    public let description: String?
    public let suggestions: [JSONValue]
    public let questions: [WidgetQuestion]

    public var id: String { requestId }

    public init?(_ raw: JSONValue) {
        guard let id = raw["requestId"]?.stringValue else { return nil }
        requestId = id
        sessionId = raw["sessionId"]?.stringValue
        terminalId = raw["terminalId"]?.stringValue
        toolName = raw["toolName"]?.stringValue ?? "unknown"
        toolInput = raw["toolInput"]?.objectValue ?? [:]
        description = raw["description"]?.stringValue
        suggestions = raw["permissionSuggestions"]?.arrayValue ?? []
        questions = (raw["questions"]?.arrayValue ?? []).map { q in
            WidgetQuestion(header: q["header"]?.stringValue, question: q["question"]?.stringValue ?? "",
                           options: (q["options"]?.arrayValue ?? []).compactMap { $0["label"]?.stringValue })
        }
    }

    /// Widget.tsx: a question card, rather than Allow and Deny.
    public var isQuestion: Bool { toolName == "AskUserQuestion" && !questions.isEmpty }

    /// Widget.tsx `PermissionCard`: the file it touches, the command it runs, or what it says it does.
    public var context: (kind: ContextKind, text: String)? {
        if let path = toolInput["file_path"]?.stringValue, !path.isEmpty {
            let last = path.split(separator: "/", omittingEmptySubsequences: false).last.map(String.init) ?? ""
            return (.file, last.isEmpty ? path : last)
        }
        if let command = toolInput["command"]?.stringValue, !command.isEmpty {
            return (.command, command.count > 64 ? String(command.prefix(61)) + "…" : command)
        }
        if let description, !description.isEmpty { return (.text, description) }
        return nil
    }

    /// Widget.tsx `labelSuggestion`: what the button for one of the agent's suggested rules says.
    public static func label(suggestion s: JSONValue, toolName: String) -> String {
        let type = s["type"]?.stringValue
        switch type {
        case "addRules":
            let first = s["rules"]?.arrayValue?.first
            let name = first?["toolName"]?.stringValue ?? toolName
            let rule = first?["ruleContent"]?.stringValue ?? ""
            if rule.contains("**") {
                let trimmed = rule.replacingOccurrences(of: "/**", with: "")
                let dir = trimmed.split(separator: "/", omittingEmptySubsequences: false).last.map(String.init) ?? ""
                return "Allow \(name) in \(dir.isEmpty ? rule : dir)/"
            }
            if !rule.isEmpty {
                let short = rule.count > 30 ? String(rule.prefix(27)) + "…" : rule
                return "Always allow `\(short)`"
            }
            return "Always allow \(name)"
        case "setMode":
            switch s["mode"]?.stringValue {
            case "acceptEdits": return "Auto-accept edits"
            case "plan": return "Switch to plan mode"
            case let mode?: return mode
            case nil: return "Set mode"
            }
        default:
            return s["label"]?.stringValue ?? type ?? ""
        }
    }

    /// Widget.tsx `AskQuestionCard.submit`: the tool input with each answered question's choice under `answers`.
    public func answering(_ picks: [Int: String]) -> JSONValue {
        var answers: [String: JSONValue] = [:]
        for (i, label) in picks where !label.isEmpty { answers[String(i)] = .string(label) }
        var input = toolInput
        input["answers"] = .object(answers)
        return .object(input)
    }
}

/// Widget.tsx: the requests waiting on a person, in arrival order.
public struct PermissionQueue: Equatable, Sendable {
    public private(set) var requests: [PermissionRequest] = []

    public init() {}

    /// `onPermissionRequest`: a request already held is not added twice.
    public mutating func add(_ request: PermissionRequest) {
        guard !requests.contains(where: { $0.requestId == request.requestId }) else { return }
        requests.append(request)
    }

    /// `onPermissionCancelled`, and after an answer.
    public mutating func remove(_ requestId: String) {
        requests.removeAll { $0.requestId == requestId }
    }

    /// The requests shown under a row; one with no terminal shows nowhere, as in the renderer.
    public func forTerminal(_ id: String) -> [PermissionRequest] {
        requests.filter { $0.terminalId == id }
    }

    /// `permission:resolve`'s params.
    public static func resolveParams(_ requestId: String, allow: Bool, updatedPermissions: [JSONValue]? = nil,
                                     updatedInput: JSONValue? = nil) -> JSONValue {
        var o: [String: JSONValue] = ["requestId": .string(requestId), "allow": .bool(allow)]
        if let updatedPermissions { o["updatedPermissions"] = .array(updatedPermissions) }
        if let updatedInput { o["updatedInput"] = updatedInput }
        return .object(o)
    }
}

extension AppConfigSummary.Defaults {
    /// index.ts: whether the widget may show; only a literal `true` turns it on.
    public var isWidgetEnabled: Bool { widgetEnabled == .bool(true) }
}
