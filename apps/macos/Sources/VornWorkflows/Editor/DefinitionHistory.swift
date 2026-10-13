import Foundation
import VornCore

/// use-definition-history.ts: undo and redo over the steps and edges, typing bursts merged into one entry.
struct DefinitionHistory {
    static let stackLimit = 100
    static let mergeWindow: TimeInterval = 0.5

    private(set) var past: [WorkflowGraph] = []
    private(set) var future: [WorkflowGraph] = []
    private(set) var current: WorkflowGraph
    private var lastPushAt: Date?
    /// The first change after a load is the load settling (slug migration, alignment), not an edit.
    private var absorbingLoad = true

    init(_ graph: WorkflowGraph, absorbing: Bool = true) {
        current = graph
        absorbingLoad = absorbing
    }

    var canUndo: Bool { !past.isEmpty }
    var canRedo: Bool { !future.isEmpty }

    /// Forgets everything, for a different workflow.
    mutating func reset(_ graph: WorkflowGraph, absorbing: Bool = true) {
        past = []
        future = []
        lastPushAt = nil
        current = graph
        absorbingLoad = absorbing
    }

    /// Notes the graph after an edit; within the merge window it joins the previous entry.
    mutating func record(_ graph: WorkflowGraph, now: Date = Date()) {
        guard graph != current else { return }
        if absorbingLoad {
            absorbingLoad = false
            current = graph
            return
        }
        let elapsed = lastPushAt.map { now.timeIntervalSince($0) } ?? .infinity
        if elapsed > Self.mergeWindow || past.isEmpty {
            past.append(current)
            if past.count > Self.stackLimit { past.removeFirst() }
        }
        lastPushAt = now
        future = []
        current = graph
    }

    /// The graph to show after undoing, or nil when there is nothing to undo.
    mutating func undo() -> WorkflowGraph? {
        guard let snapshot = past.popLast() else { return nil }
        future.append(current)
        return apply(snapshot)
    }

    mutating func redo() -> WorkflowGraph? {
        guard let snapshot = future.popLast() else { return nil }
        past.append(current)
        return apply(snapshot)
    }

    private mutating func apply(_ snapshot: WorkflowGraph) -> WorkflowGraph {
        current = snapshot
        lastPushAt = nil
        absorbingLoad = false
        return snapshot
    }
}
