import Foundation
import VornCore

/// A workflow's steps and the edges between them: what every graph edit takes and returns.
public struct WorkflowGraph: Hashable, Sendable {
    public var nodes: [WorkflowNode]
    public var edges: [WorkflowEdge]

    public init(nodes: [WorkflowNode], edges: [WorkflowEdge]) {
        self.nodes = nodes
        self.edges = edges
    }
}

/// workflow-helpers.ts FlowRow: the trunk as rows, forks side by side, loops as one block.
indirect enum GraphRow: Equatable {
    case node(WorkflowNode)
    case fork(forkNodeId: String, branches: [[GraphRow]], joinNodeId: String?)
    case loop(WorkflowNode, body: [GraphRow])
}

/// A fresh id, like `crypto.randomUUID()`.
func newGraphId() -> String { UUID().uuidString.lowercased() }

extension WorkflowNode {
    /// A loop's `bodyNodeIds`, empty for anything else.
    var bodyNodeIds: [String] {
        config["bodyNodeIds"]?.arrayValue?.compactMap(\.stringValue) ?? []
    }

    mutating func setBodyNodeIds(_ ids: [String]) {
        config.set("bodyNodeIds", .array(ids.map(JSONValue.string)))
    }
}

/// workflow-helpers.ts and workflow-graph.ts: node factories and graph edits, each returning the next graph.
enum GraphOps {
    // MARK: Constants

    static let cardWidth: Double = 280
    static let loopWidth: Double = 312
    static let grid: Double = 8
    static let placementPitch: Double = 144

    /// Steps that can swap type in place; condition, loop, and trigger own structure.
    static let replaceableTypes: Set<WorkflowNodeType> = [
        .launchAgent, .script, .approval, .createTaskFromItem, .callConnectorAction, .httpRequest,
    ]

    static func nodeWidth(_ node: WorkflowNode?) -> Double { node?.type == .loop ? loopWidth : cardWidth }

    /// A coordinate on the 8pt drag lattice; rounds halves up like Math.round, and -0 reads as 0.
    static func snapToLattice(_ value: Double) -> Double {
        (value / grid + 0.5).rounded(.down) * grid + 0
    }

    // MARK: Factories

    static func triggerLabel(_ triggerType: String) -> String {
        switch triggerType {
        case "manual": return "Manual Trigger"
        case "once": return "Schedule (Once)"
        case "recurring": return "Schedule (Recurring)"
        case "taskCreated": return "When Task Created"
        case "taskStatusChanged": return "When Task Status Changes"
        case "connectorPoll": return "Connector Poll"
        case "webhook": return "Webhook"
        default: return "Trigger"
        }
    }

    static func createTrigger(_ config: JSONValue = .object(["triggerType": .string("manual")])) -> WorkflowNode {
        WorkflowNode(
            id: newGraphId(), type: .trigger, label: triggerLabel(config.str("triggerType") ?? ""), config: config)
    }

    private static func make(_ type: WorkflowNodeType, _ label: String, _ base: [String: JSONValue],
                             _ overrides: [String: JSONValue]) -> WorkflowNode {
        WorkflowNode(
            id: newGraphId(), type: type, label: label, slug: TemplateVars.slugify(label),
            config: .object(base.merging(overrides) { _, new in new }))
    }

    static func createLaunchAgent(_ config: [String: JSONValue] = [:]) -> WorkflowNode {
        make(.launchAgent, "Launch Agent", [
            "agentType": "claude", "projectName": "", "projectPath": "", "headless": true,
        ], config)
    }

    static func createScript(_ config: [String: JSONValue] = [:]) -> WorkflowNode {
        make(.script, "Execute Script", [
            "scriptType": "bash", "scriptContent": "# Write your script here\n", "projectName": "", "projectPath": "",
        ], config)
    }

    static func createApproval(_ config: [String: JSONValue] = [:]) -> WorkflowNode {
        make(.approval, "Approval Gate", ["message": ""], config)
    }

    static func createLoop(_ config: [String: JSONValue] = [:]) -> WorkflowNode {
        make(.loop, "Repeat Steps", ["nodeType": "loop", "bodyNodeIds": .array([]), "maxIterations": 2], config)
    }

    static func createCallConnectorAction(_ config: [String: JSONValue] = [:]) -> WorkflowNode {
        make(.callConnectorAction, "Connector Action", [
            "nodeType": "callConnectorAction", "connectionId": "", "action": "", "args": .object([:]),
        ], config)
    }

    static func createHttpRequest(_ config: [String: JSONValue] = [:]) -> WorkflowNode {
        make(.httpRequest, "HTTP Request", [
            "nodeType": "httpRequest", "method": "GET", "url": "", "headers": .object([:]), "body": "",
        ], config)
    }

    static func createCondition(_ config: [String: JSONValue] = [:]) -> WorkflowNode {
        make(.condition, "Condition", ["variable": "", "operator": "equals", "value": ""], config)
    }

    /// The default config for each trigger type, used by the form and the library.
    static func switchTriggerType(_ type: String, now: Date = Date()) -> JSONValue {
        switch type {
        case "once": return ["triggerType": "once", "runAt": .string(isoString(now))]
        case "recurring": return ["triggerType": "recurring", "cron": "0 9 * * *"]
        case "connectorPoll":
            return ["triggerType": "connectorPoll", "connectionId": "", "event": "", "cron": "*/5 * * * *"]
        case "webhook": return ["triggerType": "webhook", "method": "POST", "token": .string(newGraphId())]
        default: return ["triggerType": .string(type)]
        }
    }

    /// `Date.toISOString()`: UTC with milliseconds.
    static func isoString(_ date: Date) -> String {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f.string(from: date)
    }

    // MARK: Adjacency

    static func successorsMap(_ edges: [WorkflowEdge]) -> [String: [String]] {
        var map: [String: [String]] = [:]
        for e in edges { map[e.source, default: []].append(e.target) }
        return map
    }

    static func findJoinPoint(_ children: [String], _ successors: [String: [String]]) -> String? {
        guard children.count > 1 else { return nil }
        let reachable: [Set<String>] = children.map { child in
            var seen = Set<String>()
            var queue = [child]
            var i = 0
            while i < queue.count {
                let current = queue[i]
                i += 1
                if !seen.insert(current).inserted { continue }
                queue.append(contentsOf: successors[current] ?? [])
            }
            return seen
        }
        let childSet = Set(children)
        var visited = Set<String>()
        var queue = children
        var i = 0
        while i < queue.count {
            let current = queue[i]
            i += 1
            if !visited.insert(current).inserted { continue }
            if !childSet.contains(current) && reachable.allSatisfy({ $0.contains(current) }) { return current }
            queue.append(contentsOf: successors[current] ?? [])
        }
        return nil
    }

    static func reachableFrom(_ start: String, _ edges: [WorkflowEdge]) -> Set<String> {
        let successors = successorsMap(edges)
        var seen = Set<String>()
        var queue = successors[start] ?? []
        var i = 0
        while i < queue.count {
            let id = queue[i]
            i += 1
            if !seen.insert(id).inserted { continue }
            queue.append(contentsOf: successors[id] ?? [])
        }
        return seen
    }

    /// Steps downstream of `nodeId`, triggers excluded (nodesAfter).
    static func nodesAfter(_ nodes: [WorkflowNode], _ edges: [WorkflowEdge], _ nodeId: String) -> [WorkflowNode] {
        let byId = Dictionary(nodes.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        return bfsOrder(from: nodeId, edges).compactMap { byId[$0] }.filter { $0.type != .trigger }
    }

    private static func bfsOrder(from start: String, _ edges: [WorkflowEdge]) -> [String] {
        let successors = successorsMap(edges)
        var seen: Set<String> = [start]
        var order: [String] = []
        var queue = successors[start] ?? []
        var i = 0
        while i < queue.count {
            let id = queue[i]
            i += 1
            if !seen.insert(id).inserted { continue }
            order.append(id)
            queue.append(contentsOf: successors[id] ?? [])
        }
        return order
    }

    // MARK: Loops (workflow-graph.ts)

    /// Which loop owns each body step; ids that no longer exist are left out, the first claim wins.
    static func loopBodyOwners(_ nodes: [WorkflowNode]) -> [String: String] {
        let exists = Set(nodes.map(\.id))
        var owners: [String: String] = [:]
        for n in nodes where n.type == .loop {
            for id in n.bodyNodeIds where exists.contains(id) && id != n.id && owners[id] == nil {
                owners[id] = n.id
            }
        }
        return owners
    }

    /// The run graph as the scheduler sees it: each loop stands in for its body.
    static func collapseLoopBodies(_ nodes: [WorkflowNode], _ edges: [WorkflowEdge]) -> [WorkflowEdge] {
        let owners = loopBodyOwners(nodes)
        var seen = Set<String>()
        var collapsed: [WorkflowEdge] = []
        for edge in edges {
            if owners[edge.target] != nil { continue }
            let next = owners[edge.source].map {
                WorkflowEdge(id: "\(edge.id):via-loop", source: $0, target: edge.target)
            } ?? edge
            if next.source == next.target { continue }
            let key = "\(next.source)->\(next.target):\(next.conditionBranch ?? "")"
            if !seen.insert(key).inserted { continue }
            collapsed.append(next)
        }
        return collapsed
    }

    /// A loop's body as a graph of its own; a body without inner edges chains in `bodyNodeIds` order.
    static func loopBodyGraph(_ nodes: [WorkflowNode], _ edges: [WorkflowEdge], loop: WorkflowNode)
        -> (members: [WorkflowNode], edges: [WorkflowEdge], entries: [String])
    {
        let owners = loopBodyOwners(nodes)
        let byId = Dictionary(nodes.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        let members = loop.bodyNodeIds.filter { owners[$0] == loop.id }.compactMap { byId[$0] }
        let ids = Set(members.map(\.id))
        var inner = edges.filter { ids.contains($0.source) && ids.contains($0.target) }
        if inner.isEmpty && members.count > 1 {
            inner = (1..<members.count).map {
                WorkflowEdge(id: "\(members[$0 - 1].id)->\(members[$0].id):chain", source: members[$0 - 1].id,
                             target: members[$0].id)
            }
        }
        let fed = Set(inner.map(\.target))
        return (members, inner, members.filter { !fed.contains($0.id) }.map(\.id))
    }

    /// Adds a step into a loop's body, writing the edge and the membership together.
    static func appendToLoopBody(_ g: WorkflowGraph, loopId: String, _ newNode: WorkflowNode, after afterId: String? = nil)
        -> WorkflowGraph
    {
        guard let loop = g.nodes.first(where: { $0.id == loopId }), loop.type == .loop else { return g }
        let body = loop.bodyNodeIds
        let lastId = afterId.flatMap { body.contains($0) ? $0 : nil } ?? body.last ?? loopId
        let onward = g.edges.filter { $0.source == lastId }
        let nextEdges = g.edges.filter { $0.source != lastId }
            + [WorkflowEdge(id: newGraphId(), source: lastId, target: newNode.id)]
            + onward.map { var e = $0; e.id = newGraphId(); e.source = newNode.id; return e }
        let nextNodes = g.nodes.map { n -> WorkflowNode in
            guard n.id == loopId else { return n }
            var copy = n
            copy.setBodyNodeIds(body + [newNode.id])
            return copy
        }
        return WorkflowGraph(nodes: placeNewNodes(g.nodes, nextNodes + [newNode], nextEdges), edges: nextEdges)
    }

    /// The loop that owns an insertion point inside a rail: the loop itself, or the loop listing that step.
    static func loopOwningInsertPoint(_ nodes: [WorkflowNode], _ afterNodeId: String) -> WorkflowNode? {
        nodes.first { $0.id == afterNodeId && $0.type == .loop }
            ?? nodes.first { $0.type == .loop && $0.bodyNodeIds.contains(afterNodeId) }
    }

    /// Brings newly inserted steps into the loop body they were inserted in.
    static func adoptIntoLoopBody(before: [WorkflowNode], _ result: WorkflowGraph, afterNodeId: String,
                                  beforeNodeId: String?) -> WorkflowGraph
    {
        let owners = loopBodyOwners(before)
        func ownerOf(_ id: String?) -> WorkflowNode? {
            guard let id, let owner = owners[id] else { return nil }
            return before.first { $0.id == owner }
        }
        let afterOwner = ownerOf(afterNodeId)
        let beforeOwner = ownerOf(beforeNodeId)
        let loop: WorkflowNode?
        if beforeNodeId == EditorAnchor.loopBody {
            loop = loopOwningInsertPoint(before, afterNodeId)
        } else if let beforeOwner, beforeOwner.id == afterOwner?.id || beforeOwner.id == afterNodeId {
            loop = beforeOwner
        } else if beforeNodeId == nil {
            loop = afterOwner
        } else {
            loop = nil
        }
        guard let loop else { return result }
        let existing = Set(before.map(\.id))
        let added = result.nodes.filter { !existing.contains($0.id) }.map(\.id)
        guard !added.isEmpty else { return result }
        var out = result
        out.nodes = result.nodes.map { n in
            guard n.id == loop.id else { return n }
            var copy = n
            let body = n.bodyNodeIds
            copy.setBodyNodeIds(body + added.filter { !body.contains($0) })
            return copy
        }
        return out
    }

    private static func withoutBodyMembers(_ nodes: [WorkflowNode], _ removed: Set<String>) -> [WorkflowNode] {
        nodes.map { n in
            guard n.type == .loop else { return n }
            let body = n.bodyNodeIds
            guard body.contains(where: removed.contains) else { return n }
            var copy = n
            copy.setBodyNodeIds(body.filter { !removed.contains($0) })
            return copy
        }
    }

    // MARK: Inserts

    /// A condition after `afterNodeId`; with a downstream step, True/False placeholder scripts rejoin at it.
    static func insertConditionBetween(_ g: WorkflowGraph, after afterNodeId: String, before beforeNodeId: String?)
        -> WorkflowGraph
    {
        let condition = createCondition()
        var newNodes = g.nodes + [condition]
        var newEdges = g.edges
        if let beforeNodeId {
            var trueBranch = createScript(["scriptContent": "# True branch\nexit 0\n"])
            trueBranch.label = "True Branch"
            trueBranch.slug = TemplateVars.slugify("True Branch")
            var falseBranch = createScript(["scriptContent": "# False branch\nexit 0\n"])
            falseBranch.label = "False Branch"
            falseBranch.slug = TemplateVars.slugify("False Branch")
            newNodes += [trueBranch, falseBranch]
            newEdges.removeAll { $0.source == afterNodeId && $0.target == beforeNodeId }
            newEdges += [
                WorkflowEdge(id: newGraphId(), source: afterNodeId, target: condition.id),
                WorkflowEdge(id: newGraphId(), source: condition.id, target: trueBranch.id, conditionBranch: "true"),
                WorkflowEdge(id: newGraphId(), source: condition.id, target: falseBranch.id, conditionBranch: "false"),
                WorkflowEdge(id: newGraphId(), source: trueBranch.id, target: beforeNodeId),
                WorkflowEdge(id: newGraphId(), source: falseBranch.id, target: beforeNodeId),
            ]
        } else {
            newEdges.append(WorkflowEdge(id: newGraphId(), source: afterNodeId, target: condition.id))
        }
        return WorkflowGraph(nodes: placeNewNodes(g.nodes, newNodes, newEdges), edges: newEdges)
    }

    /// Splits an edge around a new step; the branch tag rides the first half.
    static func insertNodeBetween(_ g: WorkflowGraph, edgeId: String, _ newNode: WorkflowNode) -> WorkflowGraph {
        guard let edge = g.edges.first(where: { $0.id == edgeId }) else { return g }
        var newEdges = g.edges.filter { $0.id != edgeId }
        newEdges.append(WorkflowEdge(
            id: newGraphId(), source: edge.source, target: newNode.id, conditionBranch: edge.conditionBranch))
        newEdges.append(WorkflowEdge(id: newGraphId(), source: newNode.id, target: edge.target))
        return WorkflowGraph(nodes: placeNewNodes(g.nodes, g.nodes + [newNode], newEdges), edges: newEdges)
    }

    /// Appends after the last step with nothing below it.
    static func appendNode(_ g: WorkflowGraph, _ newNode: WorkflowNode) -> WorkflowGraph {
        let withOutgoing = Set(g.edges.map(\.source))
        var newEdges = g.edges
        if let last = g.nodes.last(where: { !withOutgoing.contains($0.id) }) {
            newEdges.append(WorkflowEdge(id: newGraphId(), source: last.id, target: newNode.id))
        }
        return WorkflowGraph(nodes: placeNewNodes(g.nodes, g.nodes + [newNode], newEdges), edges: newEdges)
    }

    static func appendNodeAfter(_ g: WorkflowGraph, after afterNodeId: String, _ newNode: WorkflowNode) -> WorkflowGraph {
        let newEdges = g.edges + [WorkflowEdge(id: newGraphId(), source: afterNodeId, target: newNode.id)]
        return WorkflowGraph(nodes: placeNewNodes(g.nodes, g.nodes + [newNode], newEdges), edges: newEdges)
    }

    /// A step between a fork and its branches: it takes over every edge leaving the fork.
    static func insertBeforeFork(_ g: WorkflowGraph, forkNodeId: String, _ newNode: WorkflowNode) -> WorkflowGraph {
        var newEdges = g.edges.map { e in
            e.source == forkNodeId ? WorkflowEdge(id: newGraphId(), source: newNode.id, target: e.target) : e
        }
        newEdges.append(WorkflowEdge(id: newGraphId(), source: forkNodeId, target: newNode.id))
        return WorkflowGraph(nodes: placeNewNodes(g.nodes, g.nodes + [newNode], newEdges), edges: newEdges)
    }

    /// A new branch from `forkFromId`, rejoining where its sibling chain does.
    static func addParallelBranch(_ g: WorkflowGraph, forkFrom forkFromId: String, _ newNode: WorkflowNode)
        -> WorkflowGraph
    {
        let successors = successorsMap(g.edges)
        let children = successors[forkFromId] ?? []
        var newEdges = g.edges + [WorkflowEdge(id: newGraphId(), source: forkFromId, target: newNode.id)]
        if children.count == 1 {
            var joinTarget: String?
            var current = children[0]
            while true {
                let succs = successors[current] ?? []
                if succs.isEmpty { break }
                if succs.count == 1 {
                    joinTarget = succs[0]
                    break
                }
                guard let jp = findJoinPoint(succs, successors) else { break }
                current = jp
            }
            if let joinTarget { newEdges.append(WorkflowEdge(id: newGraphId(), source: newNode.id, target: joinTarget)) }
        } else if children.count > 1, let join = findJoinPoint(children, successors) {
            newEdges.append(WorkflowEdge(id: newGraphId(), source: newNode.id, target: join))
        }
        return WorkflowGraph(nodes: placeNewNodes(g.nodes, g.nodes + [newNode], newEdges), edges: newEdges)
    }

    // MARK: Remove

    /// Removes a step, reconnecting around it; a condition takes both branches with it up to the join.
    static func removeNode(_ g: WorkflowGraph, _ nodeId: String) -> WorkflowGraph {
        let target = g.nodes.first { $0.id == nodeId }
        if target?.type == .condition {
            let successors = successorsMap(g.edges)
            let starts = successors[nodeId] ?? []
            let join = findJoinPoint(starts, successors)
            var toRemove = collectBranchSubtree(starts, join, successors)
            toRemove.insert(nodeId)
            let predecessors = g.edges.filter { $0.target == nodeId }.map(\.source)
            var remaining = g.edges.filter { !toRemove.contains($0.source) && !toRemove.contains($0.target) }
            if let join {
                for p in predecessors { remaining.append(WorkflowEdge(id: newGraphId(), source: p, target: join)) }
            }
            let nodes = withoutBodyMembers(g.nodes.filter { !toRemove.contains($0.id) }, toRemove)
            return WorkflowGraph(nodes: placeNewNodes(g.nodes, nodes, remaining), edges: remaining)
        }
        let incoming = g.edges.filter { $0.target == nodeId }
        let outgoing = g.edges.filter { $0.source == nodeId }
        var newEdges = g.edges.filter { $0.source != nodeId && $0.target != nodeId }
        for i in incoming {
            for o in outgoing { newEdges.append(WorkflowEdge(id: newGraphId(), source: i.source, target: o.target)) }
        }
        let nodes = withoutBodyMembers(g.nodes.filter { $0.id != nodeId }, [nodeId])
        return WorkflowGraph(nodes: placeNewNodes(g.nodes, nodes, newEdges), edges: newEdges)
    }

    private static func collectBranchSubtree(_ starts: [String], _ join: String?, _ successors: [String: [String]])
        -> Set<String>
    {
        var collected = Set<String>()
        var queue = starts
        var i = 0
        while i < queue.count {
            let current = queue[i]
            i += 1
            if current == join || !collected.insert(current).inserted { continue }
            for next in successors[current] ?? [] where next != join { queue.append(next) }
        }
        return collected
    }

    // MARK: Placement

    /// Whether every stored position is still the untouched seed column (x = 0 everywhere).
    static func positionsAreSeed(_ nodes: [WorkflowNode]) -> Bool { nodes.allSatisfy { $0.position.x == 0 } }

    /// Places only the new nodes under their parents, shifting downstream steps out of the way.
    static func placeNewNodes(_ prev: [WorkflowNode], _ next: [WorkflowNode], _ edges: [WorkflowEdge]) -> [WorkflowNode] {
        if positionsAreSeed(prev) { return autoLayoutNodes(next, edges) }
        let prevIds = Set(prev.map(\.id))
        var placed: [String: WorkflowNodePosition] = [:]
        var placedOrder: [String] = []
        for n in next where prevIds.contains(n.id) {
            placed[n.id] = n.position
            placedOrder.append(n.id)
        }
        var siblingCount: [String: Int] = [:]
        var remaining = next.filter { !prevIds.contains($0.id) }
        while !remaining.isEmpty {
            let ready = remaining.firstIndex { n in
                guard let incoming = edges.first(where: { $0.target == n.id }) else { return true }
                return placed[incoming.source] != nil
            }
            let node = remaining.remove(at: ready ?? 0)
            let incoming = edges.first { $0.target == node.id }
            let base = incoming.flatMap { placed[$0.source] } ?? WorkflowNodePosition(x: 0, y: 0)
            let sibling = incoming.map { siblingCount[$0.source] ?? 0 } ?? 0
            if let incoming { siblingCount[incoming.source] = sibling + 1 }
            let parent = incoming.flatMap { inc in next.first { $0.id == inc.source } }
            let centering = (nodeWidth(parent) - nodeWidth(node)) / 2
            var candidate = WorkflowNodePosition(x: base.x + centering + Double(sibling) * 320, y: base.y + placementPitch)
            let downstream = reachableFrom(node.id, edges)
            func occupied(_ p: WorkflowNodePosition) -> Bool {
                placedOrder.contains { id in
                    guard !downstream.contains(id), let o = placed[id] else { return false }
                    return abs(o.x - p.x) < 300 && abs(o.y - p.y) < 120
                }
            }
            var tries = 0
            while occupied(candidate) && tries < 50 {
                candidate.y += placementPitch
                tries += 1
            }
            placed[node.id] = candidate
            placedOrder.append(node.id)
        }
        let newIds = Set(next.filter { !prevIds.contains($0.id) }.map(\.id))
        var shifted = Set<String>()
        for newId in next.map(\.id) where newIds.contains(newId) {
            guard let newPos = placed[newId] else { continue }
            for id in reachableFrom(newId, edges) where !newIds.contains(id) {
                if let pos = placed[id], pos.y <= newPos.y + 100 { shifted.insert(id) }
            }
        }
        return next.map { n in
            var copy = n
            if newIds.contains(n.id), let p = placed[n.id] {
                copy.position = p
            } else if shifted.contains(n.id) {
                copy.position.y += placementPitch
            }
            return copy
        }
    }

    /// The seed column: breadth-first from the trigger (or the roots), one pitch apart.
    static func autoLayoutNodes(_ nodes: [WorkflowNode], _ edges: [WorkflowEdge]) -> [WorkflowNode] {
        guard !nodes.isEmpty else { return nodes }
        let hasIncoming = Set(edges.map(\.target))
        var ordered: [WorkflowNode] = nodes.first(where: { $0.type == .trigger }).map { [$0] }
            ?? nodes.filter { !hasIncoming.contains($0.id) }
        let children = successorsMap(edges)
        let byId = Dictionary(nodes.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        var visited = Set(ordered.map(\.id))
        var queue = ordered.map(\.id)
        var i = 0
        while i < queue.count {
            let current = queue[i]
            i += 1
            for child in children[current] ?? [] where !visited.contains(child) {
                visited.insert(child)
                queue.append(child)
                if let n = byId[child] { ordered.append(n) }
            }
        }
        for n in nodes where !visited.contains(n.id) { ordered.append(n) }
        return ordered.enumerated().map { index, n in
            var copy = n
            copy.position = WorkflowNodePosition(x: 0, y: Double(index) * placementPitch)
            return copy
        }
    }

    // MARK: Flow layout

    static func computeFlowLayout(_ nodes: [WorkflowNode], _ edges: [WorkflowEdge]) -> [GraphRow] {
        guard !nodes.isEmpty else { return [] }
        let byId = Dictionary(nodes.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        let successors = successorsMap(edges)
        var rows: [GraphRow] = []
        if let trigger = nodes.first(where: { $0.type == .trigger }) {
            var seen = Set<String>()
            rows += buildFlow(from: trigger.id, stopBefore: nil, byId, successors, &seen)
        } else {
            let hasIncoming = Set(edges.map(\.target))
            var seen = Set<String>()
            for root in nodes where !hasIncoming.contains(root.id) {
                rows += buildFlow(from: root.id, stopBefore: nil, byId, successors, &seen)
            }
        }
        let visited = collectNodeIds(rows)
        for n in nodes where !visited.contains(n.id) { rows.append(.node(n)) }
        return rows
    }

    private static func collectNodeIds(_ rows: [GraphRow]) -> Set<String> {
        var ids = Set<String>()
        for row in rows {
            switch row {
            case .node(let n): ids.insert(n.id)
            case .loop(let n, let body):
                ids.insert(n.id)
                ids.formUnion(collectNodeIds(body))
            case .fork(_, let branches, _):
                for b in branches { ids.formUnion(collectNodeIds(b)) }
            }
        }
        return ids
    }

    private static func buildFlow(from startId: String, stopBefore: String?, _ byId: [String: WorkflowNode],
                                  _ successors: [String: [String]], _ seen: inout Set<String>) -> [GraphRow]
    {
        var rows: [GraphRow] = []
        var currentId: String? = startId
        while let id = currentId {
            if id == stopBefore || seen.contains(id) { break }
            seen.insert(id)
            guard let node = byId[id] else { break }
            let succ = successors[id] ?? []
            if node.type == .loop {
                let bodyIds = node.bodyNodeIds.filter { byId[$0] != nil }
                rows.append(.loop(node, body: bodyIds.compactMap { byId[$0].map(GraphRow.node) }))
                seen.formUnion(bodyIds)
                var next = succ.first
                while let n = next, bodyIds.contains(n) { next = successors[n]?.first }
                currentId = next
                continue
            }
            rows.append(.node(node))
            if succ.count <= 1 {
                currentId = succ.first
            } else {
                let join = findJoinPoint(succ, successors)
                let branches = succ.map { child -> [GraphRow] in
                    var branchSeen = seen
                    return buildFlow(from: child, stopBefore: join, byId, successors, &branchSeen)
                }
                rows.append(.fork(forkNodeId: id, branches: branches, joinNodeId: join))
                currentId = join
            }
        }
        return rows
    }
}

/// The anchor ids the canvas hands the editor (WorkflowCanvas.tsx).
enum EditorAnchor {
    static let loopBody = "__LOOP_BODY__"
    static let fork = "__FORK__"
    static let trigger = "__TRIGGER__"
}
