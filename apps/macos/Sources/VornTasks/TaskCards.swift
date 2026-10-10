import SwiftUI
import VornCore
import VornUI

/// KanbanCardMenu: the actions a card offers, then a delete confirmation in place.
struct CardMenu: View {
    let task: VornTask
    let store: TasksStore
    @State private var confirmingDelete = false

    private struct Item {
        let glyph: LucideGlyph
        let label: String
        var iconColor: Color = Theme.inkFaint
        var separator = false
        let action: () -> Void
    }

    private var items: [Item] {
        var items: [Item] = []
        let id = task.id
        let live = store.isSessionLive(task)
        if task.status == .inProgress, let open = store.openSession {
            items.append(Item(glyph: live ? .terminal : .play, label: live ? "Focus session" : "Resume session",
                              iconColor: live ? Theme.ink : Theme.inkSecondary) { open(task) })
        }
        if task.status == .inReview {
            items.append(Item(glyph: .fileCode, label: "Review diff", iconColor: Theme.inkSecondary) { store.selectedTaskId = id })
            items.append(Item(glyph: .circleCheck, label: "Mark as done", iconColor: Theme.inkSecondary) { store.complete(id) })
        }
        if task.status == .cancelled {
            items.append(Item(glyph: .rotateCcw, label: "Reopen task", iconColor: Theme.inkSecondary) { store.reopen(id) })
        }
        items.append(Item(glyph: .pencil, label: "Edit") { store.selectedTaskId = id })
        let terminal = task.status.isTerminal
        let cancelShown = !terminal
        let archiveShown = !task.isArchived && terminal
        let unarchiveShown = task.isArchived
        if cancelShown {
            items.append(Item(glyph: .circleX, label: "Cancel task", iconColor: Theme.danger, separator: true) { store.cancel(id) })
        }
        if archiveShown {
            items.append(Item(glyph: .archive, label: "Archive", separator: true) { store.archive(id) })
        }
        if unarchiveShown {
            items.append(Item(glyph: .archiveRestore, label: "Unarchive", separator: true) { store.unarchive(id) })
        }
        return items
    }

    var body: some View {
        MenuSurface {
            if confirmingDelete {
                VStack(alignment: .leading, spacing: 10) {
                    Text("Delete this task permanently?").font(.system(size: 12)).foregroundStyle(Theme.inkSecondary)
                    HStack(spacing: 8) {
                        Spacer(minLength: 0)
                        Button { confirmingDelete = false } label: {
                            Text("Cancel").font(.system(size: 11)).foregroundStyle(Theme.inkSecondary)
                                .padding(.horizontal, 8).padding(.vertical, 4)
                                .background(Theme.white(0.04), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                        }
                        .buttonStyle(.plain)
                        Button {
                            store.closePopup()
                            store.delete(task.id)
                        } label: {
                            Text("Delete").font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.danger)
                                .padding(.horizontal, 8).padding(.vertical, 4)
                                .background(Theme.danger.opacity(0.1), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                                .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.danger.opacity(0.2)))
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .frame(width: 180)
            } else {
                let items = items
                ForEach(items.indices, id: \.self) { i in
                    let item = items[i]
                    if item.separator { MenuSeparator() }
                    MenuRow(item.label) {
                        LucideIcon(item.glyph, size: 14).foregroundStyle(item.iconColor)
                    } action: {
                        store.closePopup()
                        item.action()
                    }
                }
                let anySeparator = items.contains { $0.separator }
                if !anySeparator { MenuSeparator() }
                MenuRow("Delete") {
                    LucideIcon(.trash2, size: 14).foregroundStyle(Theme.danger)
                } action: {
                    confirmingDelete = true
                }
            }
        }
    }
}

/// The `#123` reference a task imported from a connector carries.
struct SourceBadge: View {
    let task: VornTask
    var size: CGFloat = 10

    var body: some View {
        if task.sourceConnectorId != nil, let ext = task.sourceExternalId {
            Text("#\(ext)").font(.system(size: size)).foregroundStyle(Theme.inkFaint)
        }
    }
}

/// TaskCard, kanban variant.
struct KanbanCard: View {
    let task: VornTask
    let store: TasksStore
    @State private var hovered = false
    @State private var menuFrame: CGRect = .zero

    var body: some View {
        let dimmed = task.status == .cancelled || task.isArchived
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 0) {
                HStack(spacing: 6) {
                    Text(task.shortId).font(.system(size: 10, weight: .medium)).foregroundStyle(Theme.inkFaint)
                    if task.sourceConnectorId != nil && task.sourceExternalId != nil {
                        Text("·").font(.system(size: 10)).foregroundStyle(Theme.inkFaint)
                        SourceBadge(task: task)
                    }
                    if task.isArchived {
                        LucideIcon(.archive, size: 10).foregroundStyle(Theme.inkFaint)
                    }
                }
                Spacer(minLength: 0)
                GlyphButton(glyph: .ellipsis, help: "More actions") {
                    store.toggle("menu:\(task.id)", anchor: menuFrame, align: .trailing) {
                        CardMenu(task: task, store: store)
                    }
                }
                .globalFrame($menuFrame)
                .opacity(hovered || store.popup?.id == "menu:\(task.id)" ? 1 : 0)
                .frame(height: 15)
            }
            .padding(.bottom, 6)

            HStack(alignment: .top, spacing: 6) {
                StatusIcon(status: task.status).padding(.top, 2)
                Text(task.title)
                    .font(.system(size: 13, weight: .medium))
                    .strikethrough(task.status == .cancelled)
                    .foregroundStyle(task.status == .cancelled ? Theme.inkFaint : Theme.ink)
                    .lineLimit(2)
                    .lineSpacing(1.5)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }

            HStack {
                if let agent = task.assignedAgent {
                    HStack(spacing: 4) {
                        AgentIcon(agent, size: 13)
                        if store.isSessionLive(task) { LiveDot() }
                    }
                }
                Spacer(minLength: 0)
                Text("Created \(TaskDates.short(task.createdAt))")
                    .font(.system(size: 11)).foregroundStyle(Theme.inkFaint)
            }
            .padding(.top, 10)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .background(Theme.surfaceRaised, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(hovered ? 0.16 : 0.08)))
        .opacity(dimmed ? 0.6 : 1)
        .contentShape(RoundedRectangle(cornerRadius: Theme.radiusMd))
        .onHover { hovered = $0 }
        .animation(.easeOut(duration: 0.15), value: hovered)
        .onTapGesture { store.select(task) }
    }
}

/// The one moving thing on the board: a pulsing dot for a task whose session is running.
struct LiveDot: View {
    @State private var on = false

    var body: some View {
        Circle().fill(Theme.ink).frame(width: 6, height: 6)
            .opacity(on ? 0.4 : 1)
            .onAppear {
                withAnimation(.easeInOut(duration: 1).repeatForever(autoreverses: true)) { on = true }
            }
    }
}

/// TaskCard, list variant: one row with hover actions.
struct TaskRow: View {
    let task: VornTask
    let store: TasksStore
    @State private var hovered = false
    @State private var trashFrame: CGRect = .zero

    var body: some View {
        let dimmed = task.status == .cancelled || task.isArchived
        let id = task.id
        HStack(spacing: 12) {
            Group {
                if let agent = task.assignedAgent {
                    AgentIcon(agent, size: 14)
                } else {
                    Text("---").font(.system(size: 10)).foregroundStyle(Theme.inkFaint)
                }
            }
            .frame(width: 16)

            HStack(spacing: 6) {
                Text(task.shortId)
                SourceBadge(task: task, size: 11)
                if task.isArchived { LucideIcon(.archive, size: 10) }
            }
            .font(.system(size: 11, weight: .medium))
            .foregroundStyle(Theme.inkFaint)
            .fixedSize()

            StatusIcon(status: task.status)

            Text(task.title)
                .font(.system(size: 14))
                .strikethrough(task.status == .cancelled)
                .foregroundStyle(task.status == .cancelled ? Theme.inkFaint : Theme.ink)
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(maxWidth: .infinity, alignment: .leading)

            Text(TaskDates.short(task.createdAt)).font(.system(size: 11)).foregroundStyle(Theme.inkFaint).fixedSize()

            HStack(spacing: 2) {
                if task.status == .inReview {
                    action(.fileCode, "Review diff") { store.selectedTaskId = id }
                    action(.circleCheck, "Mark as done") { store.complete(id) }
                }
                if task.status == .cancelled {
                    action(.rotateCcw, "Reopen task") { store.reopen(id) }
                }
                if let open = store.openSession {
                    let live = store.isSessionLive(task)
                    action(live ? .terminal : .play, live ? "Focus session" : "Resume session") { open(task) }
                }
                action(.pencil, "Edit task") { store.selectedTaskId = id }
                if !task.status.isTerminal {
                    action(.circleX, "Cancel task") { store.cancel(id) }
                }
                if !task.isArchived && task.status.isTerminal {
                    action(.archive, "Archive task") { store.archive(id) }
                }
                if task.isArchived {
                    action(.archiveRestore, "Unarchive task") { store.unarchive(id) }
                }
                action(.trash2, "Delete task") {
                    store.toggle("confirm:\(id)", anchor: trashFrame) {
                        ConfirmDelete(message: "Delete this task permanently?") {
                            store.closePopup()
                        } onConfirm: {
                            store.closePopup()
                            store.delete(id)
                        }
                    }
                }
                .globalFrame($trashFrame)
            }
            .opacity(hovered || store.popup?.id == "confirm:\(id)" ? 1 : 0)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(hovered ? Theme.white(0.04) : .clear, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
        .opacity(dimmed ? 0.6 : 1)
        .contentShape(Rectangle())
        .onHover { hovered = $0 }
        .onTapGesture { store.select(task) }
    }

    private func action(_ glyph: LucideGlyph, _ help: String, _ run: @escaping () -> Void) -> GlyphButton {
        GlyphButton(glyph: glyph, size: 12, hoverColor: Theme.ink, help: help, action: run)
    }
}
