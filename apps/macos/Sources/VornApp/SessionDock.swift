import SwiftUI
import VornCore
import VornUI
import VornWorkflows

/// One thing parked in the dock: a gate waiting on the user, or a minimized session.
enum DockItem: Identifiable {
    case waiting(WaitingApproval)
    case minimized(TerminalSession)

    enum Kind { case waiting, minimized }

    var kind: Kind {
        switch self {
        case .waiting: .waiting
        case .minimized: .minimized
        }
    }

    var id: String {
        switch self {
        case .waiting(let w): "wait-\(w.id)"
        case .minimized(let s): "min-\(s.id)"
        }
    }

    /// Waiting gates first, then minimized sessions when this layout docks them.
    @MainActor static func items(_ model: AppModel, _ store: VornStore, _ features: FeatureStores,
                                 includeMinimized: Bool) -> [DockItem] {
        var items = features.workflows.waitingApprovals.map(DockItem.waiting)
        if includeMinimized {
            items += model.visibleSessions(store, includeMinimized: true)
                .filter { model.minimizedIDs.contains($0.id) }
                .map(DockItem.minimized)
        }
        return items
    }

    /// Runs of the same kind, so a divider can stand between them.
    static func grouped(_ items: [DockItem]) -> [(kind: Kind, items: [DockItem])] {
        var groups: [(kind: Kind, items: [DockItem])] = []
        for item in items {
            if let last = groups.indices.last, groups[last].kind == item.kind {
                groups[last].items.append(item)
            } else {
                groups.append((item.kind, [item]))
            }
        }
        return groups
    }

    /// "2 waiting approvals · 1 minimized".
    static func summary(_ items: [DockItem]) -> String {
        let waiting = items.filter { $0.kind == .waiting }.count
        let minimized = items.count - waiting
        var parts: [String] = []
        if waiting > 0 { parts.append("\(waiting) waiting approval\(waiting == 1 ? "" : "s")") }
        if minimized > 0 { parts.append("\(minimized) minimized") }
        return parts.joined(separator: " · ")
    }
}

/// Gates waiting on approval and minimized sessions, beside the nav (SessionDock.tsx).
struct SessionDock: View {
    static let maxInline = 4

    @Bindable var model: AppModel
    let store: VornStore
    let features: FeatureStores
    /// The tabs layout has nothing minimized, so its dock holds only gates.
    let includeMinimized: Bool

    var body: some View {
        let items = DockItem.items(model, store, features, includeMinimized: includeMinimized)
        if !items.isEmpty {
            if model.sessionDockCollapsed {
                collapsed(items)
            } else {
                expanded(items)
            }
        }
    }

    private func collapsed(_ items: [DockItem]) -> some View {
        let summary = DockItem.summary(items)
        return Hovering { hover in
            Button { model.toggle(.dock) } label: {
                HStack(spacing: 6) {
                    LucideIcon(.layers, size: 11, strokeWidth: 1.5)
                    Text("\(items.count)").font(.system(size: 11, weight: .medium, design: .monospaced))
                }
                .foregroundStyle(hover ? Color.white : Theme.gray300)
                .padding(.horizontal, 8)
                .frame(height: 26)
                .background(Theme.surfaceRaised, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd)
                    .strokeBorder(hover ? Theme.white(0.12) : Theme.hairline, lineWidth: 1))
                .overlay(alignment: .topTrailing) {
                    if items.contains(where: { $0.kind == .waiting }) {
                        Circle().fill(Theme.bronzo).frame(width: 6, height: 6).offset(x: 2, y: -2)
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .tooltip(summary)
        .accessibilityLabel(summary)
        .controlAnchor("dock")
    }

    private func expanded(_ items: [DockItem]) -> some View {
        let overflow = items.count - Self.maxInline
        return HStack(spacing: 6) {
            let groups = DockItem.grouped(Array(items.prefix(Self.maxInline)))
            ForEach(Array(groups.enumerated()), id: \.offset) { i, group in
                if i > 0 { Rectangle().fill(Theme.white(0.08)).frame(width: 1, height: 16) }
                HStack(spacing: 6) {
                    ForEach(group.items) { DockPill(item: $0, model: model, features: features) }
                }
            }
            if overflow > 0 {
                Hovering { hover in
                    Button { model.toggle(.dock) } label: {
                        Text("+\(overflow)")
                            .font(.ui(11, weight: .medium))
                            .foregroundStyle(hover ? Color.white : Theme.gray400)
                            .padding(.horizontal, 6)
                            .frame(minWidth: 28, minHeight: 26)
                            .background(Theme.surfaceRaised, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                            .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd)
                                .strokeBorder(hover ? Theme.white(0.12) : Theme.hairline, lineWidth: 1))
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
                .tooltip("\(overflow) more in dock")
                .accessibilityLabel("\(overflow) more dock items")
                .controlAnchor("dock")
            }
            IconButton(.chevronsLeft, size: 12, strokeWidth: 1.75, padding: 5, radius: Theme.radius, color: Theme.gray500,
                       hoverFill: Theme.white(0.06), help: "Collapse dock") {
                model.overlay = model.overlay == .dock ? nil : model.overlay
                model.sessionDockCollapsed = true
            }
            .accessibilityLabel("Collapse session dock")
        }
    }
}

/// A dock item as its pill.
struct DockPill: View {
    let item: DockItem
    @Bindable var model: AppModel
    let features: FeatureStores

    var body: some View {
        switch item {
        case .waiting(let w):
            WaitingApprovalPill(approval: w) {
                model.overlay = nil
                features.showRun(w.run.runId, model: model)
            } decide: { decision in
                Task { await features.workflows.resolveGate(w.run, nodeId: w.state.nodeId, decision: decision) }
            }
        case .minimized(let s):
            MinimizedPill(session: s) {
                model.overlay = nil
                model.minimizedIDs.remove(s.id)
                model.showTab(s.id)
            }
        }
    }
}

/// A gate waiting on the user, answerable in place (WaitingApprovalPill.tsx).
struct WaitingApprovalPill: View {
    let approval: WaitingApproval
    let open: () -> Void
    let decide: (GateDecision) -> Void

    var body: some View {
        Hovering { hover in
            HStack(spacing: 6) {
                // No halo: a gate is waiting rather than working, and the bronzo is what calls you to it.
                Circle().fill(Theme.bronzo).frame(width: 6, height: 6)
                WorkflowIconView(name: approval.workflow?.icon, color: approval.workflow?.iconColor, size: 12)
                Text(approval.workflow?.name ?? "Workflow")
                    .font(.ui(11, weight: .medium))
                    .foregroundStyle(Theme.gray200)
                    .lineLimit(1)
                    .frame(maxWidth: 140)
                    .fixedSize(horizontal: true, vertical: false)
                if let message = approval.message, !message.isEmpty {
                    Text("·").font(.ui(10)).foregroundStyle(Theme.gray600)
                    Text(message)
                        .font(.ui(10))
                        .foregroundStyle(Theme.gray500)
                        .lineLimit(1)
                        .frame(maxWidth: 230)
                        .fixedSize(horizontal: true, vertical: false)
                }
                HStack(spacing: 2) {
                    GateButton(glyph: .check, label: "Approve", color: Theme.inkSecondary, hoverColor: Theme.ink,
                               hoverFill: Theme.white(0.08)) { decide(.approve) }
                    GateButton(glyph: .x, label: "Reject", color: Theme.inkFaint, hoverColor: Theme.danger,
                               hoverFill: Theme.danger.opacity(0.1)) { decide(.reject) }
                }
                .padding(.leading, 4)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 4)
            .background(Theme.surfaceRaised, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd)
                .strokeBorder(Theme.bronzo.opacity(hover ? 0.7 : 0.4), lineWidth: 1))
            .contentShape(Rectangle())
            .onTapGesture(perform: open)
        }
    }
}

private struct GateButton: View {
    let glyph: LucideGlyph
    let label: String
    let color: Color
    let hoverColor: Color
    let hoverFill: Color
    let action: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                LucideIcon(glyph, size: 12, strokeWidth: 2.5)
                    .foregroundStyle(hover ? hoverColor : color)
                    .padding(2)
                    .background(RoundedRectangle(cornerRadius: Theme.radius).fill(hover ? hoverFill : .clear))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .tooltip(label)
        .accessibilityLabel(label)
    }
}

/// The dock's popover: everything when collapsed, under an Expand footer; else what did not fit.
struct DockPanel: View {
    @Bindable var model: AppModel
    let store: VornStore
    let features: FeatureStores
    let includeMinimized: Bool

    var body: some View {
        let all = DockItem.items(model, store, features, includeMinimized: includeMinimized)
        let shown = model.sessionDockCollapsed ? all : Array(all.dropFirst(SessionDock.maxInline))
        let groups = DockItem.grouped(shown)
        VStack(alignment: .leading, spacing: 8) {
            if model.sessionDockCollapsed {
                HStack {
                    Spacer(minLength: 0)
                    Hovering { hover in
                        Button {
                            model.overlay = nil
                            model.sessionDockCollapsed = false
                        } label: {
                            HStack(spacing: 4) {
                                LucideIcon(.chevronsRight, size: 11, strokeWidth: 1.75)
                                Text("Expand").font(.ui(10))
                            }
                            .foregroundStyle(hover ? Color.white : Theme.gray400)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                    .tooltip("Expand dock")
                }
                .padding(.horizontal, 4)
                .padding(.bottom, 4)
                .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }
            }
            ForEach(Array(groups.enumerated()), id: \.offset) { _, group in
                VStack(alignment: .leading, spacing: 4) {
                    if groups.count > 1 {
                        Text(group.kind == .waiting ? "WAITING APPROVAL" : "MINIMIZED")
                            .font(.ui(9, weight: .medium))
                            .tracking(0.45)
                            .foregroundStyle(group.kind == .waiting ? Theme.inkSecondary : Theme.gray500)
                            .padding(.horizontal, 4)
                    }
                    FlowLayout(spacing: 6) {
                        ForEach(group.items) { DockPill(item: $0, model: model, features: features) }
                    }
                }
            }
        }
        .padding(6)
        .frame(minWidth: 240, alignment: .leading)
        .fixedSize()
        .background(Theme.surfaceRaised, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.08), lineWidth: 1))
        .shadow(color: .black.opacity(0.3), radius: 8, y: 4)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Dock items")
    }
}
