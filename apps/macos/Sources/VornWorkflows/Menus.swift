import SwiftUI
import VornCore
import VornUI

/// Which dropdown is open, and the global frame of the control it hangs from.
public struct WorkflowsMenu: Equatable, Sendable {
    public enum Kind: Equatable, Sendable {
        case runFilter
        case sidebarFilter
        case workflow(String)
    }

    public enum Edge: Sendable { case leading, trailing }

    public var kind: Kind
    public var anchor: CGRect
    public var edge: Edge
    /// A fixed top-left corner, kept inside the window like WorkflowContextMenu's clampToViewport.
    public var at: CGPoint?

    public init(kind: Kind, anchor: CGRect, edge: Edge) {
        self.kind = kind
        self.anchor = anchor
        self.edge = edge
    }

    public init(kind: Kind, at: CGPoint) {
        self.kind = kind
        self.anchor = CGRect(origin: at, size: .zero)
        self.edge = .leading
        self.at = at
    }

    /// WorkflowContextMenu.tsx clampToViewport: 180 wide, up to 176 tall, 8 from every edge.
    static func clamped(_ p: CGPoint, in size: CGSize) -> CGPoint {
        var x = p.x, y = p.y
        if x + 180 + 8 > size.width { x = size.width - 180 - 8 }
        if y + 176 + 8 > size.height { y = size.height - 176 - 8 }
        return CGPoint(x: max(x, 8), y: max(y, 8))
    }
}

extension View {
    /// Draws the Workflows dropdowns over this view; mount once at the window root.
    public func workflowsMenuHost(_ store: WorkflowsStore) -> some View {
        overlay { MenuHost(store: store) }
    }

    /// Reports this control's global frame to `frame`, for anchoring a dropdown.
    func trackFrame(_ frame: Binding<CGRect>) -> some View {
        background(
            GeometryReader { g in
                Color.clear
                    .onAppear { frame.wrappedValue = g.frame(in: .global) }
                    .onChange(of: g.frame(in: .global)) { _, f in frame.wrappedValue = f }
            })
    }
}

private struct MenuHost: View {
    let store: WorkflowsStore

    var body: some View {
        GeometryReader { g in
            if let menu = store.menu {
                let origin = g.frame(in: .global).origin
                ZStack(alignment: .topLeading) {
                    Color.black.opacity(0.001)
                        .onTapGesture { store.menu = nil }
                    let fixed = menu.at.map {
                        WorkflowsMenu.clamped(CGPoint(x: $0.x - origin.x, y: $0.y - origin.y), in: g.size)
                    }
                    content(menu)
                        .fixedSize()
                        .alignmentGuide(.leading) { d in
                            if let fixed { return -fixed.x }
                            let x = menu.edge == .leading ? menu.anchor.minX : menu.anchor.maxX - d.width
                            return -(x - origin.x)
                        }
                        .alignmentGuide(.top) { _ in
                            if let fixed { return -fixed.y }
                            return -(menu.anchor.maxY + 4 - origin.y)
                        }
                }
                .frame(width: g.size.width, height: g.size.height, alignment: .topLeading)
                .onExitCommand { store.menu = nil }
            }
        }
    }

    @ViewBuilder private func content(_ menu: WorkflowsMenu) -> some View {
        switch menu.kind {
        case .runFilter:
            DropdownPanel(width: 200, verticalPadding: 6) {
                MenuLabel("Status")
                ForEach(RunBucket.allCases, id: \.self) { bucket in
                    OptionRow(
                        label: bucket.filterLabel, selected: store.runFilter == bucket,
                        dot: bucket.dotColor
                    ) {
                        store.runFilter = bucket
                        store.menu = nil
                    }
                }
            }
        case .sidebarFilter:
            DropdownPanel(width: 160, verticalPadding: 6) {
                MenuLabel("Filter")
                ForEach(SidebarWorkflowFilter.allCases, id: \.self) { f in
                    OptionRow(label: f.label, selected: store.sidebarFilter == f, dot: nil) {
                        store.sidebarFilter = f
                        store.menu = nil
                    }
                }
            }
        case .workflow(let id):
            if let wf = store.workflow(id) { WorkflowContextMenu(store: store, workflow: wf) }
        }
    }
}

extension RunBucket {
    var filterLabel: String {
        switch self {
        case .all: return "All"
        case .running: return "Running"
        case .waiting: return "Waiting"
        case .error: return "Failed"
        case .success: return "Succeeded"
        }
    }

    var dotColor: Color {
        switch self {
        case .all: return Theme.inkSecondary
        case .running: return StatusDot.color(.running)
        case .waiting: return StatusDot.color(.waiting)
        case .error: return StatusDot.color(.error)
        case .success: return StatusDot.color(.success)
        }
    }
}

/// surface-overlay, white .08 hairline, rounded-lg, shadow.
struct DropdownPanel<Content: View>: View {
    var width: CGFloat?
    var minWidth: CGFloat?
    var verticalPadding: CGFloat = 4
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) { content }
            .padding(.vertical, verticalPadding)
            .frame(width: width)
            .frame(minWidth: minWidth, alignment: .leading)
            .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusLg))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.08), lineWidth: 1))
            .shadow(color: .black.opacity(0.5), radius: 16, y: 8)
    }
}

struct MenuLabel: View {
    let text: String

    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text.uppercased())
            .font(.system(size: 10))
            .tracking(0.5)
            .foregroundStyle(Theme.gray500)
            .padding(.horizontal, 12)
            .padding(.vertical, 4)
    }
}

/// OptionRow.tsx: a check (or its space), an optional dot, the label.
struct OptionRow: View {
    let label: String
    let selected: Bool
    let dot: Color?
    let action: () -> Void

    var body: some View {
        Hovering { hovered in
            Button(action: action) {
                HStack(spacing: 8) {
                    Group {
                        if selected { Icon(.check, size: 11, weight: .semibold) } else { Color.clear }
                    }
                    .frame(width: 11, height: 11)
                    if let dot { Circle().fill(dot).frame(width: 6, height: 6) }
                    Text(label)
                    Spacer(minLength: 0)
                }
                .font(.system(size: 12))
                .foregroundStyle(selected || hovered ? Color.white : Theme.gray300)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .background(selected ? Theme.white(0.06) : hovered ? Theme.white(0.04) : .clear)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }
}

struct MenuDivider: View {
    var body: some View { Hairline(opacity: 0.06).padding(.vertical, 4) }
}

/// The sidebar row's menu (WorkflowContextMenu.tsx); no confirm, like today.
struct WorkflowContextMenu: View {
    let store: WorkflowsStore
    let workflow: WorkflowDefinition

    var body: some View {
        DropdownPanel(minWidth: 180) {
            EditorMenuRow(glyph: .pencil, label: "Edit Workflow") {
                store.menu = nil
                store.openWorkflow(workflow.id)
            }
            if workflow.isScheduled {
                EditorMenuRow(glyph: .power, label: workflow.enabled ? "Disable Schedule" : "Enable Schedule") {
                    store.menu = nil
                    store.setEnabled(workflow, !workflow.enabled)
                }
            }
            EditorMenuRow(glyph: .upload, label: "Export as file…") {
                store.menu = nil
                store.exportWorkflow(workflow)
            }
            EditorMenuRow(glyph: .trash2, label: "Delete Workflow", danger: true) {
                store.menu = nil
                store.delete(workflow)
            }
        }
    }
}
