import SwiftUI
import VornCore
import VornUI

/// The landing's controls for the shell's top bar: tabs, the status filter, refresh.
public struct WorkflowsHeader: View {
    @Bindable var store: WorkflowsStore
    @State private var filterFrame = CGRect.zero

    public init(store: WorkflowsStore) {
        self.store = store
    }

    public var body: some View {
        HStack(spacing: 4) {
            tab(.runs, "All runs")
            tab(.review, "Needs review", count: store.waitingCount)
            Rectangle().fill(Theme.white(0.06)).frame(width: 1, height: 16).padding(.horizontal, 4)
            if store.tab == .runs { filterButton }
            Hovering { hovered in
                Button { Task { await store.reload() } } label: {
                    Spinning(active: store.isLoading) { Icon(.refresh, size: 14, weight: .light) }
                        .foregroundStyle(hovered ? Theme.ink : Theme.inkFaint)
                        .padding(4)
                        .background(hovered ? Theme.white(0.06) : .clear, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("Refresh")
            }
        }
    }

    private func tab(_ t: WorkflowsTab, _ label: String, count: Int = 0) -> some View {
        let active = store.tab == t
        return Hovering { hovered in
            Button { store.tab = t } label: {
                HStack(spacing: 6) {
                    Text(label).font(.system(size: 12))
                    if count > 0 {
                        Text("\(count)")
                            .font(.system(size: 10, design: .monospaced))
                            .monospacedDigit()
                            .foregroundStyle(Theme.bronzo)
                    }
                }
                .foregroundStyle(active || hovered ? Theme.ink : Theme.inkFaint)
                .padding(.horizontal, 10)
                .padding(.vertical, 4)
                .background(active || hovered ? Theme.white(0.06) : .clear, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }

    private var filterButton: some View {
        let open = store.menu?.kind == .runFilter
        let active = store.runFilter != .all
        return Hovering { hovered in
            Button {
                store.menu = open ? nil : WorkflowsMenu(kind: .runFilter, anchor: filterFrame, edge: .trailing)
            } label: {
                Icon(.sliders, size: 16, weight: .light)
                    .foregroundStyle(open || active || hovered ? Color.white : Theme.gray400)
                    .padding(4)
                    .background(
                        open ? Theme.white(0.1) : active ? Theme.white(hovered ? 0.12 : 0.08) : hovered ? Theme.white(0.06) : .clear,
                        in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                    .overlay(alignment: .topTrailing) {
                        if active && !open {
                            Circle().fill(Theme.inkSecondary).frame(width: 6, height: 6).padding(2)
                        }
                    }
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("View options")
            .trackFrame($filterFrame)
        }
    }
}

/// The sidebar's Workflows section: header actions, All runs, one row per workflow.
public struct WorkflowsSidebarSection: View {
    @Bindable var store: WorkflowsStore
    @State private var filterFrame = CGRect.zero

    public init(store: WorkflowsStore) {
        self.store = store
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            if store.sidebarSectionOpen {
                VStack(alignment: .leading, spacing: 2) {
                    AllRunsRow(store: store)
                    let list = store.sidebarWorkflows
                    if list.isEmpty {
                        Text("No workflows")
                            .font(.system(size: 13))
                            .foregroundStyle(Theme.gray600)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 4)
                    }
                    ForEach(list) { wf in WorkflowRow(store: store, workflow: wf) }
                }
            }
        }
    }

    private var header: some View {
        HStack(spacing: 0) {
            Hovering { hovered in
                Button { store.sidebarSectionOpen.toggle() } label: {
                    HStack(spacing: 6) {
                        Icon(.chevronRight, size: 10, weight: .semibold)
                            .foregroundStyle(Theme.gray600)
                            .rotationEffect(.degrees(store.sidebarSectionOpen ? 90 : 0))
                        Text("WORKFLOWS")
                            .font(.system(size: 11, weight: .medium))
                            .tracking(0.55)
                            .foregroundStyle(hovered ? Theme.gray300 : Theme.gray500)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            Spacer(minLength: 0)
            HStack(spacing: 2) {
                let filterOpen = store.menu?.kind == .sidebarFilter
                headerAction(.filterList, help: "Filter workflows", lit: filterOpen || store.sidebarFilter != .all) {
                    store.menu = filterOpen ? nil : WorkflowsMenu(kind: .sidebarFilter, anchor: filterFrame, edge: .trailing)
                }
                .trackFrame($filterFrame)
                headerAction(.download, help: "Import workflow file", disabled: true) {}
                headerAction(.workflow, help: "New workflow", disabled: true) {}
            }
        }
        .padding(.top, 12)
        .padding(.bottom, 6)
    }

    private func headerAction(
        _ glyph: Glyph, help: String, lit: Bool = false, disabled: Bool = false, action: @escaping () -> Void
    ) -> some View {
        Hovering { hovered in
            let hot = hovered && !disabled
            Button(action: action) {
                Icon(glyph, size: 13, weight: .light)
                    .foregroundStyle(hot || lit ? Color.white : Theme.gray600)
                    .padding(2)
                    .background(hot || lit ? Theme.white(0.08) : .clear, in: RoundedRectangle(cornerRadius: Theme.radius))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(disabled)
            .help(help)
        }
    }
}

/// px-2 py-1.5 rounded-md 13px row with the white selection bar, shared by All runs and workflows.
private struct SidebarRow<Content: View>: View {
    let selected: Bool
    var dimmed = false
    @ViewBuilder let content: (Bool) -> Content
    @State private var hovered = false

    var body: some View {
        HStack(spacing: 8) { content(hovered) }
            .font(.system(size: 13))
            .foregroundStyle(selected || hovered ? Color.white : Theme.gray300)
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .background(!selected && hovered ? Theme.white(0.04) : .clear, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
            .overlay(alignment: .leading) {
                if selected { Capsule().fill(Color.white).frame(width: 1).padding(.vertical, 4) }
            }
            .opacity(dimmed ? 0.4 : 1)
            .contentShape(Rectangle())
            .onHover { hovered = $0 }
    }
}

private struct AllRunsRow: View {
    let store: WorkflowsStore

    var body: some View {
        SidebarRow(selected: store.editingWorkflowId == nil) { _ in
            Icon(.activity, size: 14, weight: .light).foregroundStyle(Theme.gray400)
            Text("All runs").lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
            if store.waitingCount > 0 {
                Text("\(store.waitingCount)")
                    .font(.system(size: 10, design: .monospaced))
                    .monospacedDigit()
                    .foregroundStyle(Theme.bronzo)
            }
        }
        .onTapGesture { store.showAllRuns() }
        .help("All runs")
    }
}

private struct WorkflowRow: View {
    let store: WorkflowsStore
    let workflow: WorkflowDefinition
    @State private var rowFrame = CGRect.zero
    @State private var moreFrame = CGRect.zero

    private enum Dot { case waiting, scheduled, disabled, error }

    private var dot: Dot? {
        if store.runs.contains(where: { $0.workflowId == workflow.id && $0.waitingStep != nil }) { return .waiting }
        if workflow.isScheduled { return workflow.enabled ? .scheduled : .disabled }
        return workflow.lastRunStatus == "error" ? .error : nil
    }

    var body: some View {
        let menuOpen = store.menu?.kind == .workflow(workflow.id)
        SidebarRow(
            selected: store.editingWorkflowId == workflow.id, dimmed: workflow.isScheduled && !workflow.enabled
        ) { hovered in
            ZStack(alignment: .bottomTrailing) {
                Icon(symbol: WorkflowIcons.symbolOrWorkflow(workflow.icon), size: 14, weight: .light)
                    .foregroundStyle(WorkflowIcons.color(workflow.iconColor))
                if let dot { dotView(dot).offset(x: 2, y: 2) }
            }
            Text(workflow.name).lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
            Group {
                rowAction(.play, size: 11, weight: .semibold, help: "Run") { store.runNow(workflow) }
                rowAction(.more, size: 12, weight: .semibold, help: "More") {
                    store.menu = menuOpen ? nil : WorkflowsMenu(kind: .workflow(workflow.id), anchor: moreFrame, edge: .leading)
                }
                .trackFrame($moreFrame)
            }
            .opacity(hovered || menuOpen ? 1 : 0)
        }
        .trackFrame($rowFrame)
        .onTapGesture { store.openWorkflow(workflow.id) }
        .contextMenu {
            Button("Edit Workflow") { store.openWorkflow(workflow.id) }
            if workflow.isScheduled {
                Button(workflow.enabled ? "Disable Schedule" : "Enable Schedule") {
                    store.setEnabled(workflow, !workflow.enabled)
                }
            }
            Button("Export as file…") {}.disabled(true)
            Divider()
            Button("Delete Workflow", role: .destructive) { store.delete(workflow) }
        }
    }

    @ViewBuilder private func dotView(_ dot: Dot) -> some View {
        Group {
            switch dot {
            case .waiting: StatusDot(status: .waiting, size: 8)
            case .error: Circle().fill(Theme.danger)
            case .scheduled: Circle().fill(Theme.inkSecondary)
            case .disabled: Circle().fill(Theme.inkFaint)
            }
        }
        .frame(width: 8, height: 8)
        .overlay(Circle().strokeBorder(Theme.surfaceBase, lineWidth: 1))
    }

    private func rowAction(
        _ glyph: Glyph, size: CGFloat, weight: Font.Weight, help: String, action: @escaping () -> Void
    ) -> some View {
        Hovering { hovered in
            Button(action: action) {
                Icon(glyph, size: size, weight: weight)
                    .foregroundStyle(hovered ? Color.white : Theme.gray500)
                    .padding(2)
                    .background(hovered ? Theme.white(0.08) : .clear, in: RoundedRectangle(cornerRadius: Theme.radius))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(help)
        }
    }
}
