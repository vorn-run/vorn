import SwiftUI
import VornCore
import VornUI

/// The Tasks screen the shell mounts: the board, its detail panel, and the quick-add dialog.
public struct TasksView: View {
    @Bindable var store: TasksStore

    public init(store: TasksStore) {
        self.store = store
    }

    public var body: some View {
        HStack(spacing: 0) {
            board
            if let id = store.selectedTaskId {
                let task = store.task(id: id)
                if id == TasksStore.newTaskId || task != nil {
                    TaskDetailPanel(store: store, task: task).id(id)
                }
            }
        }
        .background(Theme.surfaceBase)
        .overlay { dialog }
        .overlay { viewOptions }
        .overlay { PopupLayer(store: store) }
        .animation(.spring(response: 0.25, dampingFraction: 0.85), value: store.dialogOpen)
        .animation(.easeOut(duration: 0.12), value: store.popup?.id)
        .onAppear { store.start() }
    }

    @ViewBuilder
    private var board: some View {
        Group {
            if store.visibleTasks.isEmpty {
                EmptyBoard(store: store).padding(16)
            } else if store.viewMode == .kanban {
                KanbanBoard(store: store).padding(16)
            } else {
                TaskList(store: store)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.surfaceBase)
    }

    @ViewBuilder
    private var dialog: some View {
        if store.dialogOpen {
            ZStack {
                Color.black.opacity(0.3)
                    .onTapGesture {
                        store.dialogOpen = false
                        store.dialogTask = nil
                    }
                    .transition(.opacity)
                AddTaskDialog(store: store)
                    .transition(.scale(scale: 0.95).combined(with: .opacity))
            }
        }
    }

    /// The view options panel, hung below the shell's button wherever it sits.
    @ViewBuilder
    private var viewOptions: some View {
        if store.viewOptionsOpen {
            GeometryReader { geo in
                let root = geo.frame(in: .global)
                let anchor = store.viewOptionsAnchor
                    ?? CGRect(x: root.maxX - 32, y: root.minY - 28, width: 24, height: 24)
                ZStack(alignment: .topLeading) {
                    Color.black.opacity(0.0001)
                        .onTapGesture { store.viewOptionsOpen = false }
                    ViewOptionsPanel(store: store)
                        .offset(x: min(anchor.maxX - root.minX, root.width - 8) - 200,
                                y: max(4, anchor.maxY - root.minY + 4))
                }
                .frame(width: geo.size.width, height: geo.size.height, alignment: .topLeading)
            }
        }
    }
}
