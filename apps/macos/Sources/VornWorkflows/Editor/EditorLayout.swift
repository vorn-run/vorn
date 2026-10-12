import CoreGraphics
import Foundation
import VornCore

/// workflow-canvas-layout.ts: the definition projected onto the canvas, cards on an 8pt lattice.
enum EditorLayout {
    static let branchGap: Double = 56
    static let rowGap: Double = 56
    static let loopHeader: Double = 41
    static let loopBodyPad: Double = 16
    static let loopFooter: Double = 18 + 22 + 40
    static let emptyBody: Double = 52
    static let loopBodyTop: Double = loopHeader + loopBodyPad
    static let straightEdgeTolerance: Double = 16
    static let openingTop: Double = 48
    static let minZoom: Double = 0.2
    static let maxZoom: Double = 1.75

    struct LoopBodyLayout {
        var positions: [String: CGPoint]
        var branchMembers: Set<String>
        var width: Double
        var height: Double
    }

    struct Placed {
        var positions: [String: CGPoint]
        var branchMembers: Set<String>
    }

    // MARK: Previews and heights

    /// node-visuals.ts stepPreview: the trimmed text a card shows under its title.
    static func stepPreview(_ node: WorkflowNode?) -> String? {
        guard let node else { return nil }
        func text(_ key: String) -> String? {
            guard let raw = node.config.str(key) else { return nil }
            let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
            return trimmed.isEmpty ? nil : trimmed
        }
        switch node.type {
        case .script: return text("scriptContent")
        case .launchAgent: return text("prompt")
        case .approval: return text("message")
        case .httpRequest: return text("body")
        case .callConnectorAction: return connectorArgsPreview(node.config["args"])
        default: return nil
        }
    }

    /// node-visuals.ts connectorArgsPreview: `key: value` pairs two spaces apart.
    static func connectorArgsPreview(_ args: JSONValue?) -> String? {
        guard case .object(let dict)? = args, !dict.isEmpty else { return nil }
        return dict.map { "\($0.key): \(jsString($0.value))" }.joined(separator: "  ")
    }

    /// `String(v)` in JS for a JSON value.
    static func jsString(_ value: JSONValue) -> String {
        switch value {
        case .string(let s): return s
        case .number(let n): return n == n.rounded() && abs(n) < 1e15 ? String(Int64(n)) : String(n)
        case .bool(let b): return b ? "true" : "false"
        case .null: return "null"
        case .array(let items): return items.map { $0 == .null ? "" : jsString($0) }.joined(separator: ",")
        case .object: return "[object Object]"
        }
    }

    static func estimateNodeHeight(_ node: WorkflowNode, _ all: [WorkflowNode], _ edges: [WorkflowEdge] = []) -> Double {
        switch node.type {
        case .loop: return loopBodyTop + layoutLoopBody(node, all, edges).height + loopFooter
        case .condition: return (node.config.str("variable") ?? "").isEmpty ? 58 : 90
        default: return stepPreview(node) == nil ? 58 : 90
        }
    }

    // MARK: Loop bodies

    static func layoutLoopBody(_ loop: WorkflowNode, _ nodes: [WorkflowNode], _ edges: [WorkflowEdge]) -> LoopBodyLayout {
        let (members, inner, _) = GraphOps.loopBodyGraph(nodes, edges, loop: loop)
        if members.isEmpty {
            return LoopBodyLayout(positions: [:], branchMembers: [], width: GraphOps.loopWidth, height: emptyBody)
        }
        let placed = layoutPositions(members, inner)
        var minX = Double.infinity
        var maxX = -Double.infinity
        var bottom: Double = 0
        for member in members {
            guard let at = placed.positions[member.id] else { continue }
            minX = min(minX, at.x)
            maxX = max(maxX, at.x + GraphOps.cardWidth)
            bottom = max(bottom, at.y + estimateNodeHeight(member, members))
        }
        let span = maxX - minX
        let width = max(GraphOps.loopWidth, span + 2 * loopBodyPad)
        let shift = (width - span) / 2 - minX
        var relative: [String: CGPoint] = [:]
        for (id, at) in placed.positions {
            relative[id] = CGPoint(
                x: GraphOps.snapToLattice(at.x + shift), y: GraphOps.snapToLattice(at.y + loopBodyTop))
        }
        return LoopBodyLayout(positions: relative, branchMembers: placed.branchMembers, width: width, height: bottom)
    }

    // MARK: Positions

    /// The tidy layout: rows down the trunk, fork branches side by side, loops as one block.
    static func layoutPositions(_ nodes: [WorkflowNode], _ edges: [WorkflowEdge]) -> Placed {
        let rows = GraphOps.computeFlowLayout(nodes, GraphOps.collapseLoopBodies(nodes, edges))
        var positions: [String: CGPoint] = [:]
        var branchMembers = Set<String>()
        let bodySet = Set(GraphOps.loopBodyOwners(nodes).keys)

        func rowWidth(_ row: GraphRow) -> Double {
            switch row {
            case .loop(let loop, _): return layoutLoopBody(loop, nodes, edges).width
            case .node: return GraphOps.cardWidth
            case .fork(_, let branches, _):
                let widths = branches.map(branchWidth)
                return widths.reduce(0, +) + Double(widths.count - 1) * branchGap
            }
        }
        func branchWidth(_ branch: [GraphRow]) -> Double {
            branch.isEmpty ? GraphOps.cardWidth : branch.map(rowWidth).max() ?? GraphOps.cardWidth
        }
        func place(_ rows: [GraphRow], _ xCenter: Double, _ y: Double, _ insideBranch: Bool) -> Double {
            var cursor = y
            for row in rows {
                switch row {
                case .node(let node):
                    if bodySet.contains(node.id) { continue }
                    positions[node.id] = CGPoint(
                        x: GraphOps.snapToLattice(xCenter - GraphOps.cardWidth / 2), y: GraphOps.snapToLattice(cursor))
                    if insideBranch { branchMembers.insert(node.id) }
                    cursor += estimateNodeHeight(node, nodes) + rowGap
                case .loop(let loop, _):
                    positions[loop.id] = CGPoint(
                        x: GraphOps.snapToLattice(xCenter - layoutLoopBody(loop, nodes, edges).width / 2),
                        y: GraphOps.snapToLattice(cursor))
                    if insideBranch { branchMembers.insert(loop.id) }
                    cursor += estimateNodeHeight(loop, nodes, edges) + rowGap
                case .fork(_, let branches, _):
                    let widths = branches.map(branchWidth)
                    let total = widths.reduce(0, +) + Double(widths.count - 1) * branchGap
                    var left = xCenter - total / 2
                    var deepest = cursor
                    for (i, branch) in branches.enumerated() {
                        deepest = max(deepest, place(branch, left + widths[i] / 2, cursor, true))
                        left += widths[i] + branchGap
                    }
                    cursor = deepest
                }
            }
            return cursor
        }
        _ = place(rows, 0, 0, false)
        return Placed(positions: positions, branchMembers: branchMembers)
    }

    static func latticePosition(_ node: WorkflowNode) -> WorkflowNodePosition {
        WorkflowNodePosition(x: GraphOps.snapToLattice(node.position.x), y: GraphOps.snapToLattice(node.position.y))
    }

    static func positionsAreAligned(_ nodes: [WorkflowNode]) -> Bool {
        nodes.allSatisfy { latticePosition($0) == $0.position }
    }

    /// Stored positions brought onto the lattice; unchanged when seed or already aligned, so a save writes nothing new.
    static func alignedNodes(_ nodes: [WorkflowNode]) -> [WorkflowNode] {
        if GraphOps.positionsAreSeed(nodes) || positionsAreAligned(nodes) { return nodes }
        return nodes.map { var n = $0; n.position = latticePosition($0); return n }
    }

    /// Positions written by Tidy up: the computed layout stored on every top-level node.
    static func tidied(_ nodes: [WorkflowNode], _ edges: [WorkflowEdge]) -> [WorkflowNode] {
        let placed = layoutPositions(nodes, edges).positions
        return nodes.map { n in
            guard let at = placed[n.id] else { return n }
            var copy = n
            copy.position = WorkflowNodePosition(x: at.x, y: at.y)
            return copy
        }
    }

    // MARK: Canvas elements

    struct Card: Identifiable, Equatable {
        var id: String
        var node: WorkflowNode
        /// Relative to the parent loop for body members, else canvas coordinates.
        var position: CGPoint
        var absolute: CGPoint
        var width: Double
        var height: Double
        var parentId: String?

        var frame: CGRect { CGRect(x: absolute.x, y: absolute.y, width: width, height: height) }
        var isLoop: Bool { node.type == .loop }
    }

    /// CanvasEdgeData: where an insert on this edge lands.
    struct EdgeData: Equatable {
        var afterNodeId: String
        var beforeNodeId: String
        var conditionBranch: String?
        var insideBranch: Bool
        var bodyOf: String?
    }

    struct Edge: Identifiable, Equatable {
        var id: String
        var source: String
        var fromBodyHandle: Bool
        var target: String
        var label: String?
        var data: EdgeData?
    }

    struct AddStep: Identifiable, Equatable {
        var id: String
        var afterNodeId: String
        var insideBranch: Bool
        var position: CGPoint
    }

    struct Elements: Equatable {
        var cards: [Card]
        var edges: [Edge]
        var addSteps: [AddStep]
        var addTrigger: CGPoint?
        var branchMembers: Set<String>

        func card(_ id: String) -> Card? { cards.first { $0.id == id } }

        /// Everything drawn, for fit and the minimap.
        var bounds: CGRect {
            var rect = cards.filter { $0.parentId == nil }.map(\.frame).reduce(CGRect.null) { $0.union($1) }
            for add in addSteps { rect = rect.union(CGRect(origin: add.position, size: CGSize(width: 22, height: 22))) }
            if let t = addTrigger { rect = rect.union(CGRect(origin: t, size: CGSize(width: GraphOps.cardWidth, height: 58))) }
            return rect.isNull ? .zero : rect
        }
    }

    static func toCanvasElements(_ nodes: [WorkflowNode], _ edges: [WorkflowEdge]) -> Elements {
        let owners = GraphOps.loopBodyOwners(nodes)
        let placed = layoutPositions(nodes, edges)
        var branchMembers = placed.branchMembers
        let useComputed = GraphOps.positionsAreSeed(nodes)
        let byId = Dictionary(nodes.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        var cards: [Card] = []
        for node in nodes where owners[node.id] == nil {
            let position: CGPoint = useComputed
                ? placed.positions[node.id] ?? .zero
                : CGPoint(x: latticePosition(node).x, y: latticePosition(node).y)
            let body = node.type == .loop ? layoutLoopBody(node, nodes, edges) : nil
            let height = estimateNodeHeight(node, nodes, edges)
            cards.append(Card(id: node.id, node: node, position: position, absolute: position,
                              width: body?.width ?? GraphOps.cardWidth, height: height))
            guard let body else { continue }
            branchMembers.formUnion(body.branchMembers)
            for id in node.bodyNodeIds {
                guard let at = body.positions[id], let member = byId[id] else { continue }
                cards.append(Card(
                    id: id, node: member, position: at, absolute: CGPoint(x: position.x + at.x, y: position.y + at.y),
                    width: GraphOps.cardWidth, height: estimateNodeHeight(member, nodes, edges), parentId: node.id))
            }
        }

        func branchLabel(_ e: WorkflowEdge) -> String? {
            e.conditionBranch == "true" ? "True" : e.conditionBranch == "false" ? "False" : nil
        }
        var exitsTo: [String: [String]] = [:]
        for edge in edges {
            if let so = owners[edge.source], owners[edge.target] == nil { exitsTo["\(so)->\(edge.target)", default: []].append(edge.id) }
        }
        var out: [Edge] = []
        var seen = Set<String>()
        for edge in edges {
            guard seen.insert(edge.id).inserted else { continue }
            let sourceOwner = owners[edge.source]
            if let targetOwner = owners[edge.target] {
                let entry = edge.source == targetOwner
                if !entry && sourceOwner != targetOwner { continue }
                out.append(Edge(
                    id: edge.id, source: edge.source, fromBodyHandle: entry, target: edge.target, label: branchLabel(edge),
                    data: EdgeData(afterNodeId: edge.source, beforeNodeId: edge.target, conditionBranch: edge.conditionBranch,
                                   insideBranch: edge.conditionBranch != nil || branchMembers.contains(edge.target),
                                   bodyOf: targetOwner)))
                continue
            }
            if let sourceOwner {
                let exits = exitsTo["\(sourceOwner)->\(edge.target)"] ?? []
                if exits.first != edge.id { continue }
                out.append(Edge(
                    id: edge.id, source: sourceOwner, fromBodyHandle: false, target: edge.target, label: nil,
                    data: exits.count == 1
                        ? EdgeData(afterNodeId: edge.source, beforeNodeId: edge.target,
                                   insideBranch: branchMembers.contains(sourceOwner) || branchMembers.contains(edge.target))
                        : nil))
                continue
            }
            out.append(Edge(
                id: edge.id, source: edge.source, fromBodyHandle: false, target: edge.target, label: branchLabel(edge),
                data: EdgeData(afterNodeId: edge.source, beforeNodeId: edge.target, conditionBranch: edge.conditionBranch,
                               insideBranch: branchMembers.contains(edge.source) || branchMembers.contains(edge.target)
                                   || edge.conditionBranch != nil)))
        }

        let hasOutgoing = Set(out.filter { $0.data?.bodyOf == nil }.map(\.source))
        var adds: [AddStep] = []
        for node in nodes where owners[node.id] == nil && !hasOutgoing.contains(node.id) {
            guard let anchor = cards.first(where: { $0.id == node.id }) else { continue }
            let x: Double = anchor.absolute.x + anchor.width / 2 - 11
            let y: Double = anchor.absolute.y + anchor.height + 18
            adds.append(AddStep(
                id: "add:\(node.id)", afterNodeId: node.id, insideBranch: branchMembers.contains(node.id),
                position: CGPoint(x: x, y: y)))
        }

        var addTrigger: CGPoint?
        if !nodes.contains(where: { $0.type == .trigger }) {
            let top = cards.filter { $0.parentId == nil }.min { $0.absolute.y < $1.absolute.y }
            addTrigger = top.map {
                CGPoint(x: $0.absolute.x + $0.width / 2 - GraphOps.cardWidth / 2, y: $0.absolute.y - 58 - rowGap)
            } ?? .zero
        }
        return Elements(cards: cards, edges: out, addSteps: adds, addTrigger: addTrigger, branchMembers: branchMembers)
    }

    // MARK: Connections

    /// Whether a hand-drawn edge is allowed: no cycles, no edges into triggers, nothing out of a condition by hand.
    static func canConnect(_ nodes: [WorkflowNode], _ edges: [WorkflowEdge], source: String, target: String,
                           fromBodyHandle: Bool = false) -> Bool
    {
        if source == target { return false }
        guard let s = nodes.first(where: { $0.id == source }), let t = nodes.first(where: { $0.id == target }) else {
            return false
        }
        if t.type == .trigger || s.type == .condition { return false }
        let owners = GraphOps.loopBodyOwners(nodes)
        let targetOwner = owners[target]
        if fromBodyHandle {
            if targetOwner != source { return false }
        } else if let targetOwner, owners[source] != targetOwner {
            return false
        }
        if edges.contains(where: { $0.source == source && $0.target == target }) { return false }
        return !GraphOps.reachableFrom(target, edges).contains(source) && target != source
    }

    // MARK: Viewport

    struct Viewport: Equatable {
        var x: Double
        var y: Double
        var zoom: Double
    }

    /// Zoom 1, centred, the first step near the top.
    static func openingViewport(_ elements: Elements, width: Double) -> Viewport {
        let steps = elements.cards.filter { $0.parentId == nil }.map(\.frame)
        let drawn = steps.isEmpty ? (elements.addTrigger.map { [CGRect(origin: $0, size: CGSize(width: 280, height: 58))] } ?? []) : steps
        guard let first = drawn.first else { return Viewport(x: width / 2, y: openingTop, zoom: 1) }
        let rect = drawn.dropFirst().reduce(first) { $0.union($1) }
        return placeAtTop(CGRect(x: rect.minX, y: rect.minY, width: rect.width, height: 0), width: width, zoom: 1)
    }

    static func topAlignedFit(_ bounds: CGRect, width: Double, height: Double) -> Viewport {
        let fits = min(width * 0.9 / max(bounds.width, 1), (height - 2 * openingTop) / max(bounds.height, 1))
        return placeAtTop(bounds, width: width, zoom: min(1, max(minZoom, fits)))
    }

    private static func placeAtTop(_ bounds: CGRect, width: Double, zoom: Double) -> Viewport {
        Viewport(x: width / 2 - (bounds.minX + bounds.width / 2) * zoom, y: openingTop - bounds.minY * zoom, zoom: zoom)
    }
}
