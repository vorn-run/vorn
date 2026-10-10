import SwiftUI
import VornCore
import VornUI

/// Card geometry from workflow-helpers.ts and workflow-canvas-layout.ts.
enum CanvasMetrics {
    static let cardWidth: CGFloat = 280
    static let grid: CGFloat = 8
    static let rowGap: CGFloat = 56
    static let branchGap: CGFloat = 56
    static let openingTop: CGFloat = 48
}

/// What a canvas card says under its title, and the preview line below its rule.
enum NodeCardText {
    static func subtitle(_ node: WorkflowNode) -> String {
        let c = node.config
        func s(_ k: String) -> String? {
            guard let v = c[k]?.stringValue, !v.isEmpty else { return nil }
            return v
        }
        switch node.type {
        case .trigger:
            switch s("triggerType") {
            case "manual", nil: return "Click to run"
            case "once": return "Run at \(s("runAt").flatMap(TimeFormat.parse).map { $0.formatted(date: .numeric, time: .standard) } ?? "")"
            case "recurring": return "Cron: \(s("cron") ?? "")"
            case "taskCreated": return s("projectFilter").map { "Project: \($0)" } ?? "Any project"
            case "taskStatusChanged":
                let parts = [s("fromStatus"), s("toStatus")].compactMap { $0 }
                let transition = parts.count == 2 ? "\(parts[0]) → \(parts[1])" : parts.first ?? "Any change"
                return transition + (s("projectFilter").map { " · \($0)" } ?? "")
            case "sessionRestored":
                return (s("restore") == "any" ? "cold or warm" : "cold")
                    + (s("projectFilter").map { " · \($0)" } ?? " · any project")
            case "connectorPoll": return "\(s("event") ?? "") · \(s("cron") ?? "")"
            case "webhook": return "\(s("method") ?? "") · this machine"
            default: return ""
            }
        case .script:
            return (s("scriptType") ?? "") + (s("projectName").map { " · \($0)" } ?? "")
        case .approval:
            if let ms = c["timeoutMs"]?.numberValue, ms > 0 {
                return "Waits for approval · \(Int((ms / 1000).rounded()))s timeout"
            }
            return "Waits for approval"
        case .httpRequest:
            return s("url").map { "\(s("method") ?? "GET") \($0)" } ?? "Set URL"
        case .launchAgent:
            return (s("projectName") ?? "No project") + (s("branch").map { " · \($0)" } ?? "")
                + (s("model").map { " · \($0)" } ?? "")
        default:
            return StepVisuals.meta(node) ?? ""
        }
    }

    /// The footer line, and whether it is set in mono.
    static func footer(_ node: WorkflowNode) -> (text: String, mono: Bool)? {
        let c = node.config
        switch node.type {
        case .script:
            let line = c["scriptContent"]?.stringValue?.split(separator: "\n")
                .first { !$0.trimmingCharacters(in: .whitespaces).isEmpty && !$0.hasPrefix("#") }
            return line.map { (truncate($0.trimmingCharacters(in: .whitespaces), 50), true) }
        case .approval:
            return c["message"]?.stringValue.flatMap { $0.isEmpty ? nil : (truncate($0, 60), false) }
        case .httpRequest:
            let body = c["body"]?.stringValue?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            return body.isEmpty ? nil : (truncate(body, 50), true)
        case .launchAgent:
            return c["prompt"]?.stringValue.flatMap { $0.isEmpty ? nil : (truncate($0, 80), false) }
        default:
            return nil
        }
    }

    static func truncate(_ text: String, _ max: Int) -> String {
        text.count > max ? "\(text.prefix(max))…" : text
    }
}

/// Where each card sits: stored positions once arranged, a top-down walk otherwise.
enum CanvasLayout {
    /// The layout's estimate, which spaces the rows.
    static func height(_ node: WorkflowNode) -> CGFloat {
        if node.type == .condition { return node.config["variable"]?.stringValue?.isEmpty == false ? 90 : 58 }
        return NodeCardText.footer(node) != nil ? 90 : 58
    }

    /// The card's drawn height at CSS line-height 1.5, where edges leave it.
    static func cardHeight(_ node: WorkflowNode) -> CGFloat {
        let body: CGFloat = 10 + 19.5 + 16.5 + 10
        return NodeCardText.footer(node) != nil ? body + 8 + 1 + 8 + 16.5 : body
    }

    static func snap(_ v: CGFloat) -> CGFloat { (v / CanvasMetrics.grid).rounded() * CanvasMetrics.grid }

    static func positions(_ wf: WorkflowDefinition) -> [String: CGPoint] {
        let seeded = wf.nodes.allSatisfy { $0.position.x == 0 }
        guard seeded else {
            return Dictionary(
                wf.nodes.map { ($0.id, CGPoint(x: snap($0.position.x), y: snap($0.position.y))) },
                uniquingKeysWith: { a, _ in a })
        }
        // Rank by longest path from the roots; siblings share a row side by side.
        var rank: [String: Int] = [:]
        let incoming = Set(wf.edges.map(\.target))
        var queue = wf.nodes.filter { !incoming.contains($0.id) }.map(\.id)
        for id in queue { rank[id] = 0 }
        var guardCount = 0
        while !queue.isEmpty, guardCount < 10_000 {
            guardCount += 1
            let id = queue.removeFirst()
            for e in wf.edges where e.source == id {
                let next = (rank[id] ?? 0) + 1
                if (rank[e.target] ?? -1) < next, next < wf.nodes.count {
                    rank[e.target] = next
                    queue.append(e.target)
                }
            }
        }
        for n in wf.nodes where rank[n.id] == nil { rank[n.id] = (rank.values.max() ?? -1) + 1 }
        let rows = Dictionary(grouping: wf.nodes, by: { rank[$0.id] ?? 0 }).sorted { $0.key < $1.key }
        var out: [String: CGPoint] = [:]
        var y: CGFloat = 0
        for (_, row) in rows {
            let total = CGFloat(row.count) * CanvasMetrics.cardWidth + CGFloat(row.count - 1) * CanvasMetrics.branchGap
            var x = -total / 2
            for node in row {
                out[node.id] = CGPoint(x: snap(x), y: snap(y))
                x += CanvasMetrics.cardWidth + CanvasMetrics.branchGap
            }
            y += (row.map(height).max() ?? 58) + CanvasMetrics.rowGap
        }
        return out
    }
}

/// The workflow's graph, read-only: dotted ground, step edges, one card per node.
struct WorkflowCanvasView: View {
    let workflow: WorkflowDefinition
    var latestRun: WorkflowExecution?

    var body: some View {
        let positions = CanvasLayout.positions(workflow)
        let minX = positions.values.map(\.x).min() ?? 0
        let minY = positions.values.map(\.y).min() ?? 0
        let maxX = (positions.values.map(\.x).max() ?? 0) + CanvasMetrics.cardWidth
        let maxY = workflow.nodes.map { (positions[$0.id]?.y ?? 0) + CanvasLayout.height($0) }.max() ?? 0
        let size = CGSize(width: maxX - minX, height: maxY - minY)
        GeometryReader { g in
            let originX = max(40, (g.size.width - size.width) / 2) - minX
            let originY = CanvasMetrics.openingTop - minY
            Scroller(axes: [.horizontal, .vertical]) {
                ZStack(alignment: .topLeading) {
                    Canvas { ctx, _ in
                        for edge in workflow.edges {
                            guard let s = positions[edge.source], let t = positions[edge.target],
                                  let sn = workflow.node(edge.source) else { continue }
                            let from = CGPoint(x: originX + s.x + CanvasMetrics.cardWidth / 2, y: originY + s.y + CanvasLayout.cardHeight(sn))
                            let to = CGPoint(x: originX + t.x + CanvasMetrics.cardWidth / 2, y: originY + t.y)
                            ctx.stroke(stepPath(from, to), with: .color(Theme.white(0.16)), lineWidth: 1.5)
                        }
                    }
                    ForEach(workflow.nodes) { node in
                        if let p = positions[node.id] {
                            NodeCardView(node: node, status: latestRun?.state(of: node.id)?.status)
                                .offset(x: originX + p.x, y: originY + p.y)
                        }
                    }
                }
                .frame(
                    width: max(g.size.width, originX + maxX + 40),
                    height: max(g.size.height, originY + maxY + 40), alignment: .topLeading)
            }
            .scrollIndicators(.never)
            .background(DotGrid())
        }
        .background(Theme.surfaceBase)
    }

    /// React Flow's step edge: down, across at the midpoint, down.
    private func stepPath(_ a: CGPoint, _ b: CGPoint) -> Path {
        var p = Path()
        p.move(to: a)
        if abs(a.x - b.x) < 16 {
            p.addLine(to: CGPoint(x: a.x, y: b.y))
            return p
        }
        let midY = (a.y + b.y) / 2
        let r: CGFloat = min(5, abs(b.x - a.x) / 2, abs(midY - a.y))
        let dir: CGFloat = b.x > a.x ? 1 : -1
        p.addLine(to: CGPoint(x: a.x, y: midY - r))
        p.addQuadCurve(to: CGPoint(x: a.x + dir * r, y: midY), control: CGPoint(x: a.x, y: midY))
        p.addLine(to: CGPoint(x: b.x - dir * r, y: midY))
        p.addQuadCurve(to: CGPoint(x: b.x, y: midY + r), control: CGPoint(x: b.x, y: midY))
        p.addLine(to: b)
        return p
    }
}

/// Dots every 20pt at white .05, like the canvas background.
private struct DotGrid: View {
    var body: some View {
        Canvas { ctx, size in
            var y: CGFloat = 0
            while y < size.height {
                var x: CGFloat = 0
                while x < size.width {
                    ctx.fill(Path(ellipseIn: CGRect(x: x, y: y, width: 1, height: 1)), with: .color(Theme.white(0.05)))
                    x += 20
                }
                y += 20
            }
        }
        .allowsHitTesting(false)
    }
}

/// NodeShell: a 280pt card with the type glyph, title, subtitle and an optional footer.
struct NodeCardView: View {
    let node: WorkflowNode
    var status: NodeExecutionStatus?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 12) {
                Icon(Glyph.node(node.type), size: 18, weight: .medium)
                    .foregroundStyle(Theme.inkSecondary)
                VStack(alignment: .leading, spacing: 0) {
                    Text(node.label)
                        .font(.system(size: 13, weight: .medium))
                        .foregroundStyle(Color.white)
                        .lineLimit(1)
                        .frame(height: 19.5)
                    Text(NodeCardText.subtitle(node))
                        .font(.system(size: 11))
                        .foregroundStyle(Theme.gray500)
                        .lineLimit(1)
                        .frame(height: 16.5)
                }
                Spacer(minLength: 0)
            }
            if let footer = NodeCardText.footer(node) {
                Text(footer.text)
                    .font(.system(size: 11, design: footer.mono ? .monospaced : .default))
                    .foregroundStyle(Theme.gray600)
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, minHeight: 16.5, maxHeight: 16.5, alignment: .leading)
                    .padding(.top, 9)
                    .overlay(alignment: .top) { Hairline(opacity: 0.06) }
                    .padding(.top, 8)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .frame(width: CanvasMetrics.cardWidth, height: CanvasLayout.cardHeight(node), alignment: .topLeading)
        .background(Theme.surfaceNode, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
        .overlay(
            RoundedRectangle(cornerRadius: Theme.radiusMd)
                .strokeBorder(status == .error ? Theme.danger.opacity(0.6) : Theme.white(0.08), lineWidth: 1))
        .overlay(alignment: .topTrailing) {
            if let status, status != .pending {
                StatusDot(status: WorkflowStatusKey(status), size: 6).padding(8)
            }
        }
    }
}
