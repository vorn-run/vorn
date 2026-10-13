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

/// The edits NodeConfigPanel.tsx's own controls make.
public enum PanelEdits {
    /// WorkflowEditor.tsx handleNodeLabelChange, for this step: the label and a slug unique among the others.
    /// The editor rewrites other steps' `steps.<old>.` references when the slug moves.
    public static func setLabel(_ node: WorkflowNode, _ label: String, others: [WorkflowNode]) -> WorkflowNode {
        var n = node
        let taken = Set(others.filter { $0.id != node.id }.compactMap(\.slug).filter { !$0.isEmpty })
        n.label = label
        n.slug = TemplateVars.ensureUniqueSlug(TemplateVars.slugify(label), existing: taken)
        return n
    }

    /// `stop` is the default and is stored as absent.
    public static func setOnError(_ node: WorkflowNode, _ policy: String) -> WorkflowNode {
        var n = node
        n.onError = policy == "stop" ? nil : policy
        return n
    }

    /// handleNodeLabelChange's rename: every `steps.<old>.` in a config becomes `steps.<new>.`.
    public static func renameSlugReferences(_ nodes: [WorkflowNode], from old: String, to new: String) -> [WorkflowNode] {
        guard !old.isEmpty, old != new else { return nodes }
        return nodes.map { n in
            let text = n.config.jsonText
            guard text.contains("steps.\(old).") else { return n }
            var copy = n
            copy.config = JSONValue.parse(text.replacingOccurrences(of: "steps.\(old).", with: "steps.\(new).")) ?? n.config
            return copy
        }
    }
}

/// NodeConfigPanel.tsx: the 420pt panel that edits the selected step's label, config and error policy.
public struct NodeConfigPanel: View {
    let store: WorkflowsStore
    @Binding var node: WorkflowNode
    let context: NodeEditorContext
    let onDelete: () -> Void
    let onClose: () -> Void
    @State private var data: NodeFormsData
    private let loadsData: Bool
    @State private var menuOpen: Bool
    @Environment(\.formRenderScroll) private var renderScroll

    public init(
        store: WorkflowsStore, node: Binding<WorkflowNode>, context: NodeEditorContext, data: NodeFormsData? = nil,
        menuOpen: Bool = false, onDelete: @escaping () -> Void, onClose: @escaping () -> Void
    ) {
        self.store = store
        _node = node
        self.context = context
        self.onDelete = onDelete
        self.onClose = onClose
        _data = State(initialValue: data ?? NodeFormsData(backend: store.backend))
        loadsData = data == nil
        _menuOpen = State(initialValue: menuOpen)
    }

    private var stepContext: StepContext {
        StepContext.derive(
            node: node, nodes: context.nodes, edges: context.edges, latestRun: context.latestRun,
            lookupAction: { [data] conn, action in data.actions[conn]?.first { $0.type == action } })
    }

    public var body: some View {
        let step = stepContext
        VStack(spacing: 0) {
            header
            Scroller {
                VStack(alignment: .leading, spacing: 20) {
                    form(step)
                    if node.type != .trigger { errorSection }
                }
                .padding(20)
                .frame(maxWidth: .infinity, alignment: .leading)
                .offset(y: -renderScroll)
            }
            if let run = context.onRunToStep { runToStep(run) }
        }
        .frame(width: 420)
        .frame(maxHeight: .infinity)
        .background(Theme.surfaceNode)
        .overlay(alignment: .leading) { Hairline(opacity: 0.08, vertical: true) }
        .formPopupHost()
        .environment(\.formConnections, data.connections)
        .task {
            if loadsData { await data.load() }
        }
        .task(id: connectorConnectionIds) {
            for id in connectorConnectionIds { data.ensureActions(id) }
        }
    }

    /// The connections of every connector step, whose actions name their outputs in the picker.
    private var connectorConnectionIds: [String] {
        context.nodes.filter { $0.type == .callConnectorAction }.compactMap { $0.config.str("connectionId") }
            .filter { !$0.isEmpty }
    }

    @ViewBuilder private func form(_ step: StepContext) -> some View {
        switch node.type {
        case .trigger:
            TriggerConfigForm(store: store, data: data, config: $node.config, onOpenLibrary: context.onOpenTriggerLibrary)
        case .launchAgent:
            LaunchAgentConfigForm(store: store, data: data, config: $node.config, step: step, nodeId: node.id, nodes: context.nodes)
        case .script:
            ScriptConfigForm(store: store, data: data, config: $node.config, step: step)
        case .condition:
            ConditionConfigForm(config: $node.config, step: step)
        case .loop:
            LoopConfigForm(config: $node.config, step: step)
        case .approval:
            ApprovalConfigForm(config: $node.config, step: step, slug: node.slug)
        case .createTaskFromItem:
            CreateTaskFromItemNodeForm(store: store, config: $node.config)
        case .httpRequest:
            HttpRequestConfigForm(store: store, data: data, config: $node.config, step: step)
        case .callConnectorAction:
            CallConnectorActionNodeForm(data: data, config: $node.config, step: step)
        default:
            EmptyView()
        }
    }

    // MARK: Header

    private var header: some View {
        HStack(spacing: 8) {
            headerIcon.foregroundStyle(Theme.inkSecondary)
            LabelInput(
                text: Binding(
                    get: { node.label }, set: { node = PanelEdits.setLabel(node, $0, others: context.nodes) }))
            HStack(spacing: 2) {
                headerButton(help: "More node actions", active: menuOpen) {
                    LucideIcon(.ellipsis, size: 14)
                } action: {
                    menuOpen.toggle()
                }
                .formPopup(isPresented: $menuOpen, minWidth: 160, trailing: true) { moreMenu }
                headerButton(help: "Close node config") { LucideIcon(.x, size: 14) } action: { onClose() }
            }
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 12)
        .overlay(alignment: .bottom) { Hairline(opacity: 0.08) }
    }

    @ViewBuilder private var headerIcon: some View {
        if node.type == .callConnectorAction, let conn = data.connection(node.config.str("connectionId")) {
            ConnectorMark(connectorId: conn.realConnectorId, size: 14)
        } else {
            NodeTypeIcon(type: node.type, size: 14)
        }
    }

    private func headerButton<L: View>(
        help: String, active: Bool = false, @ViewBuilder label: () -> L, action: @escaping () -> Void
    ) -> some View {
        let label = label()
        return Hovering { hovered in
            Button(action: action) {
                label
                    .foregroundStyle(hovered || active ? Color.white : Theme.gray500)
                    .padding(4)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(help)
        }
    }

    private var moreMenu: some View {
        DropdownPanel(minWidth: 160, verticalPadding: 4) {
            if let replace = context.onOpenReplaceLibrary, FormHelpers.replaceable.contains(node.type) {
                PanelMenuRow(glyph: .lucide(.replace), label: "Replace step") {
                    menuOpen = false
                    replace(node.id)
                }
            }
            PanelMenuRow(
                glyph: .lucide(.trash2), label: node.type == .trigger ? "Remove trigger" : "Remove action", danger: true
            ) {
                menuOpen = false
                onDelete()
            }
        }
        .fixedSize()
    }

    // MARK: Error policy and footer

    private var errorSection: some View {
        let policy = node.onError ?? "stop"
        return VStack(alignment: .leading, spacing: 0) {
            Text("If this step fails").font(.system(size: 11)).foregroundStyle(Theme.gray500).padding(.bottom, 6)
            FormSelect(
                value: policy, options: [SelectOption("stop", "Stop the run"), SelectOption("continue", "Carry on anyway")],
                tone: .quiet
            ) { node = PanelEdits.setOnError(node, $0) }
            FormHint(
                policy == "stop"
                    ? "Everything downstream is skipped and the run ends as failed."
                    : "Later steps run as if this one had succeeded, and the run still counts as successful — for a step whose failure does not invalidate the rest.",
                top: 6, color: Theme.gray600
            )
            .font(.system(size: 10))
        }
        .padding(.top, 16)
        .overlay(alignment: .top) { Hairline(opacity: 0.08) }
    }

    private func runToStep(_ run: @escaping (String) -> Void) -> some View {
        Hovering { hovered in
            Button {
                run(node.id)
            } label: {
                HStack(spacing: 8) {
                    LucideIcon(.stepForward, size: 12, strokeWidth: 2)
                    Text("Run to this step").font(.system(size: 12))
                }
                .foregroundStyle(hovered ? Color.white : Theme.gray300)
                .frame(maxWidth: .infinity, minHeight: 18)
                .padding(.horizontal, 13)
                .padding(.vertical, 7)
                .overlay(
                    RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(hovered ? 0.2 : 0.12), lineWidth: 1))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .padding(12)
        .overlay(alignment: .top) { Hairline(opacity: 0.08) }
    }
}

/// The header's label input: transparent until hovered or focused.
private struct LabelInput: View {
    @Binding var text: String
    @FocusState private var focused: Bool

    var body: some View {
        Hovering { hovered in
            BareTextInput(placeholder: "Label", text: $text, size: 13)
                .fontWeight(.medium)
                .focused($focused)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(
                    Theme.white(focused ? 0.06 : hovered ? 0.04 : 0), in: RoundedRectangle(cornerRadius: Theme.radius))
                .padding(.leading, -6)
        }
    }
}

/// NodeConfigPanel.tsx's menu row: `px-3 py-2 text-[12px]`, a 12pt glyph.
private struct PanelMenuRow: View {
    let glyph: FormGlyph
    let label: String
    var danger = false
    let action: () -> Void

    var body: some View {
        Hovering { hovered in
            Button(action: action) {
                HStack(spacing: 8) {
                    FormIcon(glyph, size: 12)
                    Text(label).font(.system(size: 12))
                    Spacer(minLength: 0)
                }
                .foregroundStyle(
                    danger ? Theme.danger.opacity(hovered ? 1 : 0.8) : hovered ? Color.white : Theme.gray300)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(hovered ? Theme.white(0.06) : .clear)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }
}
