import SwiftUI
import VornCore
import VornUI

/// nodes/*.tsx: what each step's card says, its glyph, and its footer line.
enum CardText {
    static let operatorLabels: [String: String] = [
        "equals": "=", "notEquals": "!=", "contains": "contains", "notContains": "not contains",
        "isEmpty": "is empty", "isNotEmpty": "is not empty",
    ]

    /// node-visuals.ts truncate: three dots, not an ellipsis.
    static func truncate(_ text: String, _ max: Int) -> String {
        text.count > max ? String(text.prefix(max)) + "..." : text
    }

    /// TriggerNode.tsx getSubtitle.
    static func triggerSubtitle(_ c: JSONValue) -> String {
        func s(_ k: String) -> String? { c.str(k).flatMap { $0.isEmpty ? nil : $0 } }
        switch c.str("triggerType") {
        case "manual": return "Click to run"
        case "once":
            let at = c.str("runAt") ?? ""
            return "Run at \(WorkflowPropertiesPanel.localeString(at) ?? "Invalid Date")"
        case "recurring": return "Cron: \(c.str("cron") ?? "undefined")"
        case "taskCreated": return s("projectFilter").map { "Project: \($0)" } ?? "Any project"
        case "taskStatusChanged":
            let parts = [s("fromStatus"), s("toStatus")].compactMap { $0 }
            let transition = parts.count == 2 ? "\(parts[0]) → \(parts[1])" : parts.first ?? "Any change"
            return transition + (s("projectFilter").map { " · \($0)" } ?? "")
        case "sessionRestored":
            return (c.str("restore") == "any" ? "cold or warm" : "cold") + (s("projectFilter").map { " · \($0)" } ?? " · any project")
        case "connectorPoll": return "\(c.str("event") ?? "undefined") · \(c.str("cron") ?? "undefined")"
        case "webhook": return "\(c.str("method") ?? "undefined") · this machine"
        default: return ""
        }
    }

    /// The line under the title; `mono` is the trailing model id on an agent card.
    static func subtitle(_ node: WorkflowNode) -> (text: String, mono: String?) {
        let c = node.config
        func s(_ k: String) -> String? { c.str(k).flatMap { $0.isEmpty ? nil : $0 } }
        switch node.type {
        case .trigger: return (triggerSubtitle(c), nil)
        case .script: return ((c.str("scriptType") ?? "") + (s("projectName").map { " · \($0)" } ?? ""), nil)
        case .condition:
            let has = s("variable") != nil && s("operator") != nil
            return (has ? operatorLabels[c.str("operator") ?? ""] ?? "" : "Not configured", nil)
        case .approval:
            if let ms = c.num("timeoutMs"), ms != 0 { return ("Waits for approval · \(Int((ms / 1000).rounded()))s timeout", nil) }
            return ("Waits for approval", nil)
        case .createTaskFromItem:
            let project = c.str("project") == "fromConnection" ? "Project from connection" : c.str("project") ?? "undefined"
            return ("\(project) · initial: \(c.str("initialStatus") ?? "undefined")", nil)
        case .httpRequest:
            return (s("url").map { "\(c.str("method") ?? "undefined") \($0)" } ?? "Set URL", nil)
        case .callConnectorAction:
            return (s("actionLabel") ?? s("action") ?? "Select action", nil)
        case .launchAgent:
            let remote = s("remoteHostId") != nil
            let head = (s("projectName") ?? "No project") + (!remote ? s("branch").map { " · \($0)" } ?? "" : "")
            return (head + (s("model") != nil ? " · " : ""), s("model"))
        default: return ("", nil)
        }
    }

    /// The footer under the rule, and whether it is mono.
    static func footer(_ node: WorkflowNode) -> (text: String, mono: Bool)? {
        let c = node.config
        func s(_ k: String) -> String? { c.str(k).flatMap { $0.isEmpty ? nil : $0 } }
        switch node.type {
        case .script:
            let line = (s("scriptContent") ?? "").split(separator: "\n", omittingEmptySubsequences: false)
                .first { !$0.trimmingCharacters(in: .whitespaces).isEmpty && !$0.hasPrefix("#") }
            return line.map { (truncate($0.trimmingCharacters(in: .whitespacesAndNewlines), 50), true) }
        case .condition:
            guard let v = s("variable"), let op = s("operator") else { return nil }
            let value = op != "isEmpty" && op != "isNotEmpty" ? s("value").map { " \"\($0)\"" } ?? "" : ""
            return (truncate("\(v) \(operatorLabels[op] ?? "undefined")\(value)", 60), true)
        case .approval:
            return s("message").map { (truncate($0, 60), false) }
        case .httpRequest:
            let body = (c.str("body") ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            return body.isEmpty ? nil : (truncate(body, 50), true)
        case .callConnectorAction:
            return EditorLayout.connectorArgsPreview(c["args"]).map { (truncate($0, 50), true) }
        case .launchAgent:
            if let p = s("prompt") { return (truncate(p, 60), false) }
            if c.bool("taskFromQueue") == true { return ("Next task from queue", false) }
            if s("taskId") != nil { return ("From task", false) }
            return nil
        default:
            return nil
        }
    }

    static let triggerGlyphs: [String: LucideGlyph] = [
        "manual": .zap, "once": .calendar, "recurring": .clock, "taskCreated": .listPlus,
        "taskStatusChanged": .arrowRightLeft, "sessionRestored": .rotateCcw, "connectorPoll": .plug, "webhook": .globe,
    ]

    /// The connection a step talks to, drawn by its connector's mark (nodeConnectionId).
    static func connectionId(_ node: WorkflowNode) -> String? {
        switch node.type {
        case .callConnectorAction: return node.config.str("connectionId")
        case .trigger where node.config.str("triggerType") == "connectorPoll": return node.config.str("connectionId")
        default: return nil
        }
    }

    /// The card height as drawn: a 58pt body, 91.5 with a footer.
    static func drawnHeight(_ node: WorkflowNode) -> CGFloat {
        footer(node) == nil ? 58 : 91.5
    }
}

/// A node card's glyph: a connector's mark, an agent's brand, or the type's lucide icon.
struct CardGlyph: View {
    let node: WorkflowNode
    let directory: ConnectorDirectory

    var body: some View {
        let c = node.config
        if let look = directory.look(CardText.connectionId(node)) {
            ConnectorGlyph(connectorId: look.connectorId, icon: look.icon, size: 18)
        } else {
            switch node.type {
            case .trigger: glyph(CardText.triggerGlyphs[c.str("triggerType") ?? ""] ?? .zap)
            case .script: glyph(["python", "node"].contains(c.str("scriptType") ?? "") ? .codeXml : .terminal)
            case .condition: glyph(.gitFork)
            case .approval: glyph(.hand)
            case .createTaskFromItem: glyph(.listPlus)
            case .httpRequest: glyph(.globe)
            case .callConnectorAction: glyph(.zap)
            case .loop: glyph(.repeatIcon)
            case .launchAgent:
                if c.str("agentType") == "fromTask" {
                    LucideIcon(.clipboardList, size: 18).foregroundStyle(Theme.inkSecondary)
                } else {
                    AgentIcon(AgentType(rawValue: c.str("agentType") ?? "claude"), size: 18)
                }
            default: glyph(.zap)
            }
        }
    }

    private func glyph(_ g: LucideGlyph) -> some View {
        LucideIcon(g, size: 18).foregroundStyle(Theme.inkSecondary)
    }
}

/// NodeShell.tsx: the 280pt card every step is drawn in.
struct NodeCardView: View {
    let node: WorkflowNode
    let directory: ConnectorDirectory
    var selected = false
    var status: NodeExecutionStatus?
    @State private var hovered = false
    @State private var pulse = false

    var body: some View {
        let sub = CardText.subtitle(node)
        let footer = CardText.footer(node)
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 12) {
                CardGlyph(node: node, directory: directory).frame(width: 18, height: 18)
                VStack(alignment: .leading, spacing: 0) {
                    Text(node.label)
                        .font(.system(size: 13, weight: .medium))
                        .foregroundStyle(Color.white)
                        .frame(height: 19.5)
                    (Text(sub.text) + Text(sub.mono ?? "").font(.system(size: 11, design: .monospaced)))
                        .font(.system(size: 11))
                        .foregroundStyle(Theme.gray500)
                        .frame(height: 16.5)
                }
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            if let footer {
                Text(footer.text)
                    .font(.system(size: 11, design: footer.mono ? .monospaced : .default))
                    .foregroundStyle(Theme.gray600)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: .infinity, minHeight: 16.5, maxHeight: 16.5, alignment: .leading)
                    .padding(.top, 9)
                    .overlay(alignment: .top) { Rectangle().fill(Theme.white(0.06)).frame(height: 1) }
                    .padding(.top, 8)
            }
        }
        .padding(.horizontal, 13)
        .padding(.vertical, 11)
        .frame(width: GraphOps.cardWidth, height: CardText.drawnHeight(node), alignment: .topLeading)
        .background(Theme.surfaceNode)
        .background(hovered ? Theme.white(0.02) : .clear)
        .clipShape(RoundedRectangle(cornerRadius: Theme.radiusMd))
        .overlay(
            RoundedRectangle(cornerRadius: Theme.radiusMd)
                .strokeBorder(
                    selected ? Theme.white(0.4) : status == .error ? Theme.danger.opacity(0.6) : Theme.white(0.08),
                    lineWidth: 1))
        .overlay {
            if status == .running {
                RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.35), lineWidth: 1)
                    .opacity(pulse ? 0.5 : 1)
                    .onAppear {
                        withAnimation(.easeInOut(duration: 1).repeatForever(autoreverses: true)) { pulse = true }
                    }
            }
        }
        .overlay(alignment: .topTrailing) {
            if let status { StatusDot(status: WorkflowStatusKey(status), size: 6).padding(8) }
        }
        .contentShape(Rectangle())
        .onHover { hovered = $0 }
    }
}

/// AddStepNode.tsx ConnectorButton: the 22pt + that opens the step library.
struct ConnectorButton: View {
    var active = false
    let action: () -> Void

    var body: some View {
        Hovering { hovered in
            Button(action: action) {
                LucideIcon(.plus, size: 13, strokeWidth: 2.5)
                    .foregroundStyle(active || hovered ? Color.white : Theme.gray500)
                    .frame(width: 22, height: 22)
                    .background(active ? Theme.white(0.08) : Theme.surfaceNode, in: Circle())
                    .overlay(Circle().strokeBorder(active ? Theme.white(0.4) : hovered ? Theme.white(0.2) : Theme.white(0.1), lineWidth: 1))
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .help("Add a step")
            .accessibilityLabel("Add a step")
        }
    }
}

/// AddTriggerNode: the dashed spot a workflow's trigger goes.
struct AddTriggerCard: View {
    var active = false
    let action: () -> Void

    var body: some View {
        Hovering { hovered in
            Button(action: action) {
                HStack(spacing: 8) {
                    LucideIcon(.zap, size: 14)
                    Text("Add a trigger").font(.system(size: 13))
                }
                .foregroundStyle(active ? Color.white : hovered ? Theme.gray300 : Theme.gray500)
                .frame(width: GraphOps.cardWidth, height: 58)
                .overlay(
                    RoundedRectangle(cornerRadius: Theme.radiusLg)
                        .strokeBorder(
                            active ? Theme.white(0.4) : hovered ? Theme.white(0.3) : Theme.white(0.15),
                            style: StrokeStyle(lineWidth: 1, dash: [3, 3])))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }
}

/// WorkflowCanvas.tsx HANDLE_CLASS: a 7pt port on the card's edge.
struct PortDot: View {
    var body: some View {
        Circle().fill(Theme.surfaceBase)
            .overlay(Circle().strokeBorder(Theme.white(0.35), lineWidth: 1))
            .frame(width: 7, height: 7)
    }
}

/// LoopNode: the frame a loop's body steps are drawn inside.
struct LoopFrameView: View {
    let node: WorkflowNode
    let width: CGFloat
    let bodyHeight: CGFloat
    let empty: Bool
    var selected = false
    var status: NodeExecutionStatus?
    var anchorActive = false
    let onSelect: () -> Void
    let onAdd: () -> Void
    @State private var headerHovered = false

    var body: some View {
        let c = node.config
        let forEach = c.str("mode") == "forEach"
        let untilVar = c["until"]?.str("variable").flatMap { $0.isEmpty ? nil : $0 }
        let until = untilVar.map {
            " · until \($0) \(c["until"]?.str("operator") ?? "undefined") \(c["until"].map { EditorLayout.jsString($0["value"] ?? .null) } ?? "")"
        } ?? ""
        let items = (c.str("items") ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        let footer = forEach ? "for each in \(items.isEmpty ? "a list not chosen yet" : items)\(until)"
            : !until.isEmpty ? String(until.dropFirst(3)) : "runs every pass"
        let maxIter = c.num("maxIterations").map { EditorLayout.jsString(.number($0)) } ?? "1"
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                LucideIcon(.repeatIcon, size: 13).foregroundStyle(Theme.inkSecondary)
                Text(node.label).font(.system(size: 12.5, weight: .semibold)).foregroundStyle(Color.white)
                    .lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
                Text(forEach ? "each" : "max \(maxIter)")
                    .font(.system(size: 10, design: .monospaced)).foregroundStyle(Theme.gray400)
                    .padding(.horizontal, 6).padding(.vertical, 2)
                    .background(Theme.white(0.06), in: RoundedRectangle(cornerRadius: Theme.radius))
                if let status { StatusDot(status: WorkflowStatusKey(status), size: 6) }
            }
            .padding(.horizontal, 16)
            .frame(height: EditorLayout.loopHeader - 1)
            .background(headerHovered ? Theme.white(0.02) : .clear)
            .overlay(alignment: .bottom) { Rectangle().fill(Theme.white(0.06)).frame(height: 1) }
            .contentShape(Rectangle())
            .onHover { headerHovered = $0 }
            .onTapGesture(perform: onSelect)
            .padding(.top, 1)
            VStack(spacing: 0) {
                if empty {
                    Text("No steps yet — add one below")
                        .font(.system(size: 11)).foregroundStyle(Theme.gray500)
                        .frame(maxWidth: .infinity).frame(height: EditorLayout.emptyBody)
                        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd)
                            .strokeBorder(Theme.white(0.12), style: StrokeStyle(lineWidth: 1, dash: [3, 3])))
                } else {
                    Color.clear.frame(height: bodyHeight)
                }
                Rectangle().fill(Theme.white(0.08)).frame(width: 1, height: 18)
                ConnectorButton(active: anchorActive, action: onAdd)
            }
            .padding(.horizontal, 16)
            .padding(.top, EditorLayout.loopBodyPad)
            Text("↻ \(footer)")
                .font(.system(size: 10, design: .monospaced)).foregroundStyle(Theme.gray500)
                .lineLimit(1).truncationMode(.tail)
                .padding(.horizontal, 16)
                .frame(maxWidth: .infinity)
                .frame(height: 40)
        }
        .frame(width: width, height: EditorLayout.loopHeader + EditorLayout.loopBodyPad + (empty ? EditorLayout.emptyBody : bodyHeight) + 80, alignment: .top)
        .background(Theme.surfaceNode, in: RoundedRectangle(cornerRadius: Theme.radiusLg))
        .overlay(
            RoundedRectangle(cornerRadius: Theme.radiusLg)
                .strokeBorder(
                    selected ? Theme.white(0.4) : status == .error ? Theme.danger.opacity(0.6) : Theme.white(0.08),
                    lineWidth: 1))
    }
}

/// NodeHoverToolbar: run to here, replace, delete, beside a hovered card.
struct NodeHoverToolbar: View {
    let node: WorkflowNode
    let inBody: Bool
    let onRunToStep: (() -> Void)?
    let onReplace: () -> Void
    let onDelete: () -> Void

    var body: some View {
        let isTrigger = node.type == .trigger
        VStack(spacing: 4) {
            if let onRunToStep, !isTrigger, !inBody { button(.stepForward, "Run to this step", onRunToStep) }
            if GraphOps.replaceableTypes.contains(node.type) || isTrigger { button(.replace, "Replace step", onReplace) }
            button(.trash2, "Delete step", onDelete)
        }
    }

    private func button(_ glyph: LucideGlyph, _ help: String, _ action: @escaping () -> Void) -> some View {
        Hovering { hovered in
            Button(action: action) {
                LucideIcon(glyph, size: 12)
                    .foregroundStyle(hovered ? Color.white : Theme.gray400)
                    .padding(6)
                    .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                    .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(hovered ? 0.2 : 0.12), lineWidth: 1))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(help)
            .accessibilityLabel(help)
        }
    }
}
