import SwiftUI
import VornCore
import VornUI

/// What a step's form can read besides the step itself: the draft graph and the editor's hooks.
public struct NodeEditorContext {
    public var nodes: [WorkflowNode]
    public var edges: [WorkflowEdge]
    /// The freshest run of this workflow, feeding values into the variable picker.
    public var latestRun: WorkflowExecution?
    /// Opens the step library in trigger scope; set on trigger steps.
    public var onOpenTriggerLibrary: (() -> Void)?
    /// Opens the step library to swap this step in place; set on replaceable steps.
    public var onOpenReplaceLibrary: ((String) -> Void)?
    /// Runs only this step and its upstream slice; nil when the step is not a valid target.
    public var onRunToStep: ((String) -> Void)?

    public init(
        nodes: [WorkflowNode], edges: [WorkflowEdge], latestRun: WorkflowExecution? = nil,
        onOpenTriggerLibrary: (() -> Void)? = nil, onOpenReplaceLibrary: ((String) -> Void)? = nil,
        onRunToStep: ((String) -> Void)? = nil
    ) {
        self.nodes = nodes
        self.edges = edges
        self.latestRun = latestRun
        self.onOpenTriggerLibrary = onOpenTriggerLibrary
        self.onOpenReplaceLibrary = onOpenReplaceLibrary
        self.onRunToStep = onRunToStep
    }
}

/// NodeConfigPanel.tsx: the 420pt panel that edits the selected step's label, config and error policy.
struct NodeConfigPanel: View {
    let store: WorkflowsStore
    @Binding var node: WorkflowNode
    let context: NodeEditorContext
    let onDelete: () -> Void
    let onClose: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Icon(Glyph.node(node.type), size: 14).foregroundStyle(Theme.inkSecondary)
                TextField("Label", text: $node.label)
                    .textFieldStyle(.plain)
                    .font(.system(size: 13, weight: .medium))
                    .foregroundStyle(Color.white)
                IconButton(glyph: .x, size: 14, help: "Close", action: onClose)
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 12)
            .overlay(alignment: .bottom) { Hairline(opacity: 0.08) }
            Spacer()
        }
        .frame(width: 420)
        .frame(maxHeight: .infinity)
        .background(Theme.surfaceNode)
        .overlay(alignment: .leading) { Hairline(opacity: 0.08, vertical: true) }
    }
}
