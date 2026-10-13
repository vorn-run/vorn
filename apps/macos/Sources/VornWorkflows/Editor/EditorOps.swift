import Foundation
import VornCore

/// WorkflowEditor.tsx: the edits the editor composes from the graph helpers, kept pure so tests can drive them.
enum EditorOps {
    /// Slugs made unique and filled in for steps saved before slugs existed (load-time migration).
    static func migrateSlugs(_ nodes: [WorkflowNode]) -> [WorkflowNode] {
        var used = Set<String>()
        return nodes.map { n in
            var copy = n
            if let slug = n.slug, !slug.isEmpty {
                if used.contains(slug) { copy.slug = TemplateVars.ensureUniqueSlug(slug, existing: used) }
                used.insert(copy.slug ?? slug)
                return copy
            }
            if n.type == .trigger { return n }
            let slug = TemplateVars.ensureUniqueSlug(TemplateVars.slugify(n.label), existing: used)
            used.insert(slug)
            copy.slug = slug
            return copy
        }
    }

    /// A step of `type` with a slug no other step uses; agents start on the first project.
    static func createNode(_ type: AddableNodeType, in nodes: [WorkflowNode], excluding: String? = nil,
                           project: Project? = nil) -> WorkflowNode
    {
        var node: WorkflowNode
        switch type {
        case .condition: node = GraphOps.createCondition()
        case .approval: node = GraphOps.createApproval()
        case .loop: node = GraphOps.createLoop()
        case .script: node = GraphOps.createScript()
        case .connectorAction: node = GraphOps.createCallConnectorAction()
        case .httpRequest: node = GraphOps.createHttpRequest()
        case .agent:
            node = GraphOps.createLaunchAgent(
                project.map { ["projectName": .string($0.name), "projectPath": .string($0.path)] } ?? [:])
        }
        if let slug = node.slug {
            let existing = Set(nodes.filter { $0.id != excluding }.compactMap(\.slug))
            node.slug = TemplateVars.ensureUniqueSlug(slug, existing: existing)
        }
        return node
    }

    /// Renames a step, re-slugging it and rewriting `steps.<old>.` references elsewhere.
    static func relabel(_ nodes: [WorkflowNode], nodeId: String, label: String) -> [WorkflowNode] {
        let oldSlug = nodes.first { $0.id == nodeId }?.slug
        let existing = Set(nodes.filter { $0.id != nodeId }.compactMap(\.slug))
        let newSlug = TemplateVars.ensureUniqueSlug(TemplateVars.slugify(label), existing: existing)
        return nodes.map { n in
            var copy = n
            if n.id == nodeId {
                copy.label = label
                copy.slug = newSlug
            } else if let oldSlug, !oldSlug.isEmpty, oldSlug != newSlug {
                copy.config = replacing(n.config, "steps.\(oldSlug).", with: "steps.\(newSlug).")
            }
            return copy
        }
    }

    /// Every string and key in a JSON value with `old` replaced, like the TS stringify round trip.
    static func replacing(_ value: JSONValue, _ old: String, with new: String) -> JSONValue {
        switch value {
        case .string(let s): return .string(s.replacingOccurrences(of: old, with: new))
        case .array(let items): return .array(items.map { replacing($0, old, with: new) })
        case .object(let dict):
            return .object(Dictionary(
                dict.map { ($0.key.replacingOccurrences(of: old, with: new), replacing($0.value, old, with: new)) },
                uniquingKeysWith: { a, _ in a }))
        default: return value
        }
    }

    /// handleInsertNode: a new step at an anchor; returns the graph and the step to select.
    static func insert(_ g: WorkflowGraph, after afterNodeId: String, before beforeNodeId: String?,
                       type: AddableNodeType, preset: [String: JSONValue]? = nil, project: Project? = nil)
        -> (graph: WorkflowGraph, selected: String?)
    {
        if type == .condition {
            let result = GraphOps.adoptIntoLoopBody(
                before: g.nodes,
                GraphOps.insertConditionBetween(
                    g, after: afterNodeId, before: beforeNodeId == EditorAnchor.loopBody ? nil : beforeNodeId),
                afterNodeId: afterNodeId, beforeNodeId: beforeNodeId)
            let known = Set(g.nodes.map(\.id))
            return (result, result.nodes.first { $0.type == .condition && !known.contains($0.id) }?.id)
        }
        var newNode = createNode(type, in: g.nodes, project: project)
        if let preset { newNode.config = newNode.config.merged(preset.mapValues { Optional($0) }) }
        var result: WorkflowGraph
        if beforeNodeId == EditorAnchor.loopBody {
            if let loop = GraphOps.loopOwningInsertPoint(g.nodes, afterNodeId) {
                result = GraphOps.appendToLoopBody(
                    g, loopId: loop.id, newNode, after: afterNodeId == loop.id ? nil : afterNodeId)
            } else {
                result = GraphOps.appendNodeAfter(g, after: afterNodeId, newNode)
            }
        } else if beforeNodeId == EditorAnchor.fork {
            result = GraphOps.insertBeforeFork(g, forkNodeId: afterNodeId, newNode)
        } else if let beforeNodeId,
                  let edge = g.edges.first(where: { $0.source == afterNodeId && $0.target == beforeNodeId })
        {
            result = GraphOps.insertNodeBetween(g, edgeId: edge.id, newNode)
        } else {
            result = GraphOps.appendNodeAfter(g, after: afterNodeId, newNode)
        }
        result = GraphOps.adoptIntoLoopBody(before: g.nodes, result, afterNodeId: afterNodeId, beforeNodeId: beforeNodeId)
        return (result, newNode.id)
    }

    /// handleLibraryPick: applies a pick at an anchor; nil when nothing changes.
    static func applyPick(_ pick: LibraryPick, at anchor: InsertAnchor, to g: WorkflowGraph, project: Project? = nil)
        -> (graph: WorkflowGraph, selected: String?)?
    {
        switch pick {
        case .triggerType, .connectorTrigger:
            let config: JSONValue
            if case .triggerType(let type) = pick {
                config = GraphOps.switchTriggerType(type)
            } else if case .connectorTrigger(let connectionId, let event) = pick {
                config = ["triggerType": "connectorPoll", "connectionId": .string(connectionId),
                          "event": .string(event), "cron": "*/5 * * * *"]
            } else {
                return nil
            }
            if let existing = g.nodes.first(where: { $0.type == .trigger }) {
                let cur = existing.config
                let same: Bool
                if case .triggerType(let type) = pick {
                    same = cur.str("triggerType") == type
                } else {
                    same = cur.str("triggerType") == "connectorPoll" && cur.str("connectionId") == config.str("connectionId")
                        && cur.str("event") == config.str("event")
                }
                if same { return nil }
                var fresh = GraphOps.createTrigger(config)
                fresh.id = existing.id
                fresh.position = existing.position
                fresh.slug = existing.slug
                var next = g
                next.nodes = g.nodes.map { $0.id == existing.id ? fresh : $0 }
                return (next, existing.id)
            }
            let trigger = GraphOps.createTrigger(config)
            let bodySet = Set(GraphOps.loopBodyOwners(g.nodes).keys)
            let hasIncoming = Set(g.edges.map(\.target))
            let links = g.nodes.filter { !hasIncoming.contains($0.id) && !bodySet.contains($0.id) }
                .map { WorkflowEdge(id: newGraphId(), source: trigger.id, target: $0.id) }
            return (WorkflowGraph(nodes: g.nodes + [trigger], edges: g.edges + links), trigger.id)
        default:
            break
        }
        if let replaceId = anchor.replaceNodeId {
            guard let target = g.nodes.first(where: { $0.id == replaceId }) else { return nil }
            var fresh: WorkflowNode
            switch pick {
            case .connectorAction(let connectionId, let action, let actionLabel):
                fresh = createNode(.connectorAction, in: g.nodes, excluding: target.id, project: project)
                fresh.config = fresh.config.merged([
                    "connectionId": .string(connectionId), "action": .string(action),
                    "actionLabel": .string(actionLabel),
                ])
            case .type(let type) where type != .condition && type != .loop:
                fresh = createNode(type, in: g.nodes, excluding: target.id, project: project)
            default:
                return nil
            }
            fresh.id = target.id
            fresh.position = target.position
            fresh.slug = target.slug ?? fresh.slug
            var next = g
            next.nodes = g.nodes.map { $0.id == target.id ? fresh : $0 }
            return (next, target.id)
        }
        guard g.nodes.contains(where: { $0.id == anchor.afterNodeId }) else { return nil }
        if let before = anchor.beforeNodeId, before != EditorAnchor.fork, before != EditorAnchor.loopBody,
           !g.nodes.contains(where: { $0.id == before })
        {
            return nil
        }
        if pick == .parallel {
            let newNode = createNode(.agent, in: g.nodes, project: project)
            return (GraphOps.addParallelBranch(g, forkFrom: anchor.afterNodeId, newNode), newNode.id)
        }
        guard let step = pick.step else { return nil }
        return insert(g, after: anchor.afterNodeId, before: anchor.beforeNodeId, type: step.type, preset: step.config,
                      project: project)
    }

    /// handleDeleteNode.
    static func delete(_ g: WorkflowGraph, nodeId: String) -> WorkflowGraph { GraphOps.removeNode(g, nodeId) }

    /// Whether "Run to here" applies: not a trigger and not inside a loop body.
    static func runToStepEligible(_ nodes: [WorkflowNode], _ nodeId: String) -> Bool {
        guard let node = nodes.first(where: { $0.id == nodeId }) else { return false }
        return node.type != .trigger && GraphOps.loopBodyOwners(nodes)[nodeId] == nil
    }
}
