import SwiftUI
import VornCore
import VornUI

/// What picking a template puts on the canvas (template-requirements.ts templateSeed).
public struct WorkflowSeed: Sendable, Hashable {
    public var name: String
    public var icon: String?
    public var iconColor: String?
    public var nodes: [WorkflowNode]
    public var edges: [WorkflowEdge]
    /// The requirements the seed could not bind, worded for the "still needs" toast.
    public var unresolved: [String]

    public init(name: String, icon: String? = nil, iconColor: String? = nil, nodes: [WorkflowNode],
                edges: [WorkflowEdge], unresolved: [String] = []) {
        self.name = name
        self.icon = icon
        self.iconColor = iconColor
        self.nodes = nodes
        self.edges = edges
        self.unresolved = unresolved
    }
}

/// StartFromPanel.tsx: the templates a new workflow can start from.
struct StartFromPanel: View {
    let store: WorkflowsStore
    let onPick: (WorkflowSeed) -> Void
    let onClose: () -> Void

    var body: some View {
        Color.clear.frame(width: 340)
    }
}
