import SwiftUI
import VornCore
import VornUI

/// TaskKanbanBoard: five columns; cards drag between them and within one to reorder.
struct KanbanBoard: View {
    let store: TasksStore

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            ForEach(TaskStatus.boardOrder, id: \.self) { status in
                KanbanColumn(status: status, tasks: store.tasks(in: status, sortedByOrder: true), store: store)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
    }
}

private struct KanbanColumn: View {
    let status: TaskStatus
    let tasks: [VornTask]
    let store: TasksStore
    @State private var hovered = false
    @State private var targeted = false

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                StatusIcon(status: status)
                Text(status.label).font(.system(size: 13, weight: .medium)).foregroundStyle(Theme.inkSecondary)
                Text("\(tasks.count)").font(.system(size: 11)).foregroundStyle(Theme.inkFaint).padding(.leading, 2)
                Spacer(minLength: 0)
                GlyphButton(glyph: .plus, help: "Add task") { store.openNewTask(status: status) }
                    .opacity(hovered ? 1 : 0)
            }
            .frame(height: 22)
            .padding(12)

            ScrollView(.vertical) {
                VStack(spacing: 8) {
                    if tasks.isEmpty {
                        Text("Drop tasks here")
                            .font(.system(size: 12)).foregroundStyle(Theme.inkFaint)
                            .frame(maxWidth: .infinity)
                            .padding(.horizontal, 16).padding(.vertical, 20)
                            .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg)
                                .strokeBorder(Theme.white(0.08), style: StrokeStyle(lineWidth: 1, dash: [3, 3])))
                            .frame(minHeight: 80)
                    } else {
                        ForEach(tasks) { task in
                            KanbanCard(task: task, store: store)
                                .draggable(task.id) {
                                    KanbanCard(task: task, store: store).frame(width: 240).opacity(0.9)
                                }
                                .dropDestination(for: String.self) { ids, _ in
                                    guard let id = ids.first else { return false }
                                    store.move(taskId: id, before: task.id)
                                    return true
                                }
                        }
                    }
                }
                .padding(.horizontal, 8)
                .padding(.bottom, 8)
            }
            .scrollIndicators(.never)

            AddRowButton { store.openNewTask(status: status) }
                .padding(.horizontal, 8)
                .padding(.bottom, 8)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(targeted ? Theme.white(0.04) : .clear, in: RoundedRectangle(cornerRadius: Theme.radiusLg))
        .overlay {
            if targeted {
                RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.1))
            }
        }
        .contentShape(Rectangle())
        .onHover { hovered = $0 }
        .dropDestination(for: String.self) { ids, _ in
            guard let id = ids.first else { return false }
            store.drop(taskId: id, on: status)
            return true
        } isTargeted: { targeted = $0 }
        .animation(.easeOut(duration: 0.2), value: targeted)
    }
}

/// The full-width plus at a column's foot.
private struct AddRowButton: View {
    let action: () -> Void
    @State private var hovered = false

    var body: some View {
        Button(action: action) {
            LucideIcon(.plus, size: 12)
                .foregroundStyle(hovered ? Theme.inkSecondary : Theme.inkFaint)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 8)
                .background(hovered ? Theme.white(0.04) : .clear, in: RoundedRectangle(cornerRadius: Theme.radiusLg))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovered = $0 }
    }
}

/// TaskListView: a collapsible section per status.
struct TaskList: View {
    let store: TasksStore
    @State private var collapsed: Set<TaskStatus> = []

    var body: some View {
        ScrollView(.vertical) {
            VStack(spacing: 4) {
                ForEach(TaskStatus.boardOrder, id: \.self) { status in
                    let tasks = store.tasks(in: status, sortedByOrder: status == .todo)
                    VStack(spacing: 0) {
                        SectionHeader(status: status, count: tasks.count, collapsed: collapsed.contains(status)) {
                            withAnimation(.easeInOut(duration: 0.2)) {
                                if collapsed.contains(status) { collapsed.remove(status) } else { collapsed.insert(status) }
                            }
                        } onAdd: {
                            store.openNewTask(status: status)
                        }
                        if !collapsed.contains(status) {
                            VStack(spacing: 0) {
                                if tasks.isEmpty {
                                    Text(status.emptyText)
                                        .font(.system(size: 12)).foregroundStyle(Theme.inkFaint)
                                        .frame(maxWidth: .infinity, alignment: .leading)
                                        .padding(.vertical, 8).padding(.leading, 40)
                                } else {
                                    ForEach(tasks) { TaskRow(task: $0, store: store) }
                                }
                            }
                            .padding(.vertical, 4)
                            .transition(.opacity)
                        }
                    }
                    .clipped()
                }
            }
            .padding(16)
        }
    }
}

private struct SectionHeader: View {
    let status: TaskStatus
    let count: Int
    let collapsed: Bool
    let toggle: () -> Void
    let onAdd: () -> Void
    @State private var hovered = false

    var body: some View {
        HStack(spacing: 8) {
            LucideIcon(collapsed ? .chevronRight : .chevronDown, size: 14).foregroundStyle(Theme.inkSecondary)
            StatusIcon(status: status)
            Text(status.label).font(.system(size: 13, weight: .medium)).foregroundStyle(Theme.inkSecondary)
            Text("\(count)").font(.system(size: 11)).foregroundStyle(Theme.inkFaint)
            Spacer(minLength: 0)
            GlyphButton(glyph: .plus, help: "Add task", action: onAdd)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .frame(height: 38)
        .background(Theme.white(hovered ? 0.06 : 0.03), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
        .contentShape(Rectangle())
        .onHover { hovered = $0 }
        .onTapGesture(perform: toggle)
    }
}

/// The board with nothing on it.
struct EmptyBoard: View {
    let store: TasksStore

    var body: some View {
        let filtered = store.statusFilter != .all
        VStack(spacing: 0) {
            LucideIcon(.listTodo, size: 40, strokeWidth: 1).foregroundStyle(Theme.inkFaint).padding(.bottom, 12)
            Text(filtered ? "No matching tasks" : "No tasks yet")
                .font(.system(size: 14)).foregroundStyle(Theme.inkFaint).padding(.bottom, 4)
            Text(filtered ? "Try changing the status filter"
                 : store.scope.activeProject.map { "Create a task for \($0) to get started" }
                 ?? "Select a project or create a task to get started")
                .font(.system(size: 12)).foregroundStyle(Theme.inkFaint)
        }
        .multilineTextAlignment(.center)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
