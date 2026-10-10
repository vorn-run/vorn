import SwiftUI
import VornCore
import VornUI

/// StatusPicker: the status as an icon and word that opens the five states.
struct StatusPicker: View {
    let store: TasksStore
    let current: TaskStatus
    var disabled = false
    /// Store mode moves an existing task by the rules; otherwise the choice is handed back.
    var taskId: String?
    var onChange: ((TaskStatus) -> Void)?
    @State private var frame: CGRect = .zero
    @State private var hovered = false

    var body: some View {
        Button {
            guard !disabled else { return }
            store.toggle("status", anchor: frame) { menu }
        } label: {
            HStack(spacing: 6) {
                LucideIcon(current.glyph, size: 13)
                Text(current.label).font(.system(size: 12))
            }
            .foregroundStyle(current.tint)
            .padding(.horizontal, 6).padding(.vertical, 2)
            .background(hovered && !disabled ? Theme.white(0.04) : .clear, in: RoundedRectangle(cornerRadius: Theme.radius))
            .padding(.horizontal, -6)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovered = $0 }
        .globalFrame($frame)
    }

    private var menu: some View {
        MenuSurface {
            ForEach(TaskStatus.boardOrder, id: \.self) { status in
                let blocked = onChange == nil && status == .inProgress && current != .todo
                MenuRow(status.label, textColor: blocked ? Theme.inkFaint : Theme.inkSecondary,
                        checked: status == current, disabled: blocked) {
                    LucideIcon(status.glyph, size: 14).foregroundStyle(status.tint)
                } action: {
                    store.closePopup()
                    choose(status)
                }
                .help(blocked ? "Move to Todo first, then start the task" : "")
            }
        }
    }

    private func choose(_ status: TaskStatus) {
        guard status != current else { return }
        if let onChange { return onChange(status) }
        guard let id = taskId else { return }
        switch status {
        case .todo: store.reopen(id)
        case .inProgress: if current == .todo { store.setStatus(id, .inProgress) }
        case .inReview: store.moveToReview(id)
        case .done: store.complete(id)
        case .cancelled: store.cancel(id)
        }
    }
}

/// ProjectPicker, compact variant.
struct ProjectPicker: View {
    let store: TasksStore
    let current: String
    let projects: [VornProject]
    let onChange: (String) -> Void
    @State private var frame: CGRect = .zero
    @State private var hovered = false

    var body: some View {
        let project = projects.first { $0.name == current } ?? store.projects.first { $0.name == current }
        Button {
            store.toggle("project", anchor: frame) { menu }
        } label: {
            HStack(spacing: 6) {
                ProjectIcon(icon: project?.icon, color: project?.iconColor)
                Text(current.isEmpty ? "Select project..." : current)
                    .foregroundStyle(current.isEmpty ? Theme.gray600 : Theme.gray300)
                LucideIcon(.chevronDown, size: 11).foregroundStyle(Theme.gray500)
            }
            .font(.system(size: 12))
            .padding(.horizontal, 6).padding(.vertical, 2)
            .background(hovered ? Theme.white(0.04) : .clear, in: RoundedRectangle(cornerRadius: Theme.radius))
            .padding(.horizontal, -6)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovered = $0 }
        .globalFrame($frame)
    }

    private var menu: some View {
        MenuSurface(minWidth: max(180, frame.width)) {
            ForEach(projects) { p in
                MenuRow(p.name, textColor: Theme.gray300, checked: p.name == current) {
                    ProjectIcon(icon: p.icon, color: p.iconColor)
                } action: {
                    store.closePopup()
                    if p.name != current { onChange(p.name) }
                }
            }
        }
    }
}

/// AgentPicker, compact variant with a "None" row.
struct AgentPicker: View {
    let store: TasksStore
    let current: AgentKind?
    let onChange: (AgentKind?) -> Void
    @State private var frame: CGRect = .zero
    @State private var hovered = false

    var body: some View {
        Button {
            store.toggle("agent", anchor: frame) { menu }
        } label: {
            HStack(spacing: 6) {
                if let current {
                    AgentIcon(current, size: 14)
                } else {
                    LucideIcon(.bot, size: 14).foregroundStyle(Theme.gray500)
                }
                Text(current?.label ?? "Unassigned").foregroundStyle(current == nil ? Theme.gray600 : Theme.gray300)
                LucideIcon(.chevronDown, size: 11).foregroundStyle(Theme.gray500)
            }
            .font(.system(size: 12))
            .padding(.horizontal, 6).padding(.vertical, 2)
            .background(hovered ? Theme.white(0.04) : .clear)
            .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(hovered ? 0.25 : 0.12)))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovered = $0 }
        .globalFrame($frame)
    }

    private var menu: some View {
        MenuSurface(minWidth: max(180, frame.width)) {
            MenuRow("None", textColor: Theme.gray500, italic: true, checked: current == nil) {
                LucideIcon(.bot, size: 14).foregroundStyle(Theme.gray600)
            } action: {
                store.closePopup()
                if current != nil { onChange(nil) }
            }
            ForEach(AgentKind.allCases, id: \.self) { agent in
                let installed = store.installedAgents[agent] ?? false
                MenuRow(agent.label, textColor: installed ? Theme.gray300 : Theme.gray600,
                        checked: installed && agent == current,
                        trailingNote: installed ? nil : "Not installed", disabled: !installed) {
                    AgentIcon(agent, size: 14)
                } action: {
                    store.closePopup()
                    if agent != current { onChange(agent) }
                }
                .help(installed ? "" : "\(agent.rawValue) is not installed")
            }
        }
    }
}

/// TaskToolbar's panel: status, source, archived, and list or board.
struct ViewOptionsPanel: View {
    @Bindable var store: TasksStore

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            section(top: false) {
                heading("Status")
                OptionRow(label: "All", dot: Theme.inkSecondary, selected: store.statusFilter == .all) {
                    store.statusFilter = .all
                }
                ForEach(TaskStatus.boardOrder, id: \.self) { s in
                    OptionRow(label: s.label, dot: s.dot, selected: store.statusFilter == .status(s)) {
                        store.statusFilter = .status(s)
                    }
                }
            }
            let connectors = store.connectorIds
            if !connectors.isEmpty {
                section {
                    heading("Source")
                    OptionRow(label: "All Sources", dot: Theme.inkSecondary, selected: store.sourceFilter == .all) {
                        store.sourceFilter = .all
                    }
                    OptionRow(label: "Local Only", dot: Theme.inkFaint, selected: store.sourceFilter == .local) {
                        store.sourceFilter = .local
                    }
                    ForEach(connectors, id: \.self) { c in
                        OptionRow(label: c.prefix(1).uppercased() + c.dropFirst(), dot: nil,
                                  selected: store.sourceFilter == .connector(c), secondary: true) {
                            store.sourceFilter = .connector(c)
                        }
                    }
                }
            }
            section {
                HStack(spacing: 8) {
                    LucideIcon(.archive, size: 12).foregroundStyle(Theme.inkFaint)
                    Text("Include archived").frame(maxWidth: .infinity, alignment: .leading)
                    ToggleSwitch(isOn: $store.includeArchived)
                }
                .font(.system(size: 12))
                .foregroundStyle(Theme.inkSecondary)
                .padding(.horizontal, 12).padding(.vertical, 6)
            }
            section {
                heading("View")
                HStack(spacing: 4) {
                    modeButton(.list, glyph: .viewList, "List")
                    modeButton(.kanban, glyph: .viewKanban, "Board")
                }
                .padding(.horizontal, 12).padding(.vertical, 4)
            }
        }
        .frame(width: 200)
        .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusLg))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.08)))
        .clipShape(RoundedRectangle(cornerRadius: Theme.radiusLg))
        .floatingShadow()
    }

    private func section<C: View>(top: Bool = true, @ViewBuilder _ content: () -> C) -> some View {
        VStack(alignment: .leading, spacing: 0) { content() }
            .padding(.vertical, 6)
            .overlay(alignment: .top) {
                if top { Rectangle().fill(Theme.white(0.06)).frame(height: 1) }
            }
    }

    private func heading(_ s: String) -> some View {
        Text(s.uppercased())
            .font(.system(size: 10)).tracking(0.5).foregroundStyle(Theme.inkFaint)
            .padding(.horizontal, 12).padding(.vertical, 4)
    }

    private func modeButton(_ mode: TaskViewMode, glyph: LucideGlyph, _ label: String) -> some View {
        let on = store.viewMode == mode
        return Button { store.setViewMode(mode) } label: {
            HStack(spacing: 6) {
                LucideIcon(glyph, size: 12)
                Text(label).font(.system(size: 11))
            }
            .foregroundStyle(on ? Color.white : Theme.inkFaint)
            .padding(.horizontal, 10).padding(.vertical, 4)
            .background(Theme.white(on ? 0.1 : 0.04), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
        }
        .buttonStyle(.plain)
    }
}

/// OptionRow: a check, a tone dot and a label.
private struct OptionRow: View {
    let label: String
    let dot: Color?
    let selected: Bool
    var secondary = false
    let action: () -> Void
    @State private var hovered = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 8) {
                if selected {
                    LucideIcon(.optionCheck, size: 11, strokeWidth: 3)
                } else {
                    Color.clear.frame(width: 11, height: 11)
                }
                if let dot { Circle().fill(dot).frame(width: 6, height: 6) }
                Text(label)
                Spacer(minLength: 0)
            }
            .font(.system(size: 12))
            .foregroundStyle(selected || hovered ? Color.white : (secondary ? Theme.inkSecondary : Theme.gray300))
            .padding(.horizontal, 12).padding(.vertical, 6)
            .background(selected ? Theme.white(0.06) : hovered ? Theme.white(0.04) : .clear)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovered = $0 }
    }
}

/// The top bar's view options button; ⌘J toggles it. The panel itself is drawn by `TasksView`.
public struct TaskViewOptionsButton: View {
    @Bindable var store: TasksStore
    @State private var hovered = false

    public init(store: TasksStore) {
        self.store = store
    }

    public var body: some View {
        let open = store.viewOptionsOpen
        let active = store.hasActiveFilters
        Button {
            store.viewOptionsOpen.toggle()
        } label: {
            LucideIcon(.slidersHorizontal, size: 16, strokeWidth: 1.5)
                .foregroundStyle(open || active || hovered ? Color.white : Theme.inkSecondary)
                .padding(4)
                .background(
                    Theme.white(open ? 0.1 : active ? (hovered ? 0.12 : 0.08) : (hovered ? 0.06 : 0)),
                    in: RoundedRectangle(cornerRadius: Theme.radiusMd)
                )
                .overlay(alignment: .topTrailing) {
                    if active && !open {
                        Circle().fill(Theme.inkSecondary).frame(width: 6, height: 6).padding(2)
                    }
                }
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovered = $0 }
        .keyboardShortcut("j", modifiers: .command)
        .help("View options (⌘J)")
        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { store.viewOptionsAnchor = $0 }
    }
}
