import SwiftUI
import VornCore
import VornUI

/// TaskDetailPanel: a task's properties, title and description, saved as they are edited.
struct TaskDetailPanel: View {
    let store: TasksStore
    let task: VornTask?
    @State private var form: TaskForm
    @State private var saveTimer: Task<Void, Never>?
    @State private var trashFrame: CGRect = .zero
    @FocusState private var titleFocused: Bool

    private var isCreate: Bool { task == nil }

    init(store: TasksStore, task: VornTask?) {
        self.store = store
        self.task = task
        _form = State(initialValue: task.map(TaskForm.init(task:)) ?? TaskForm(projectName: store.defaultProjectName))
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            ScrollView(.vertical) {
                VStack(alignment: .leading, spacing: 0) {
                    properties
                    BareField(placeholder: "Task title", text: $form.title, font: .system(size: 16, weight: .semibold))
                        .focused($titleFocused)
                        .padding(.horizontal, 16).padding(.top, 16).padding(.bottom, 8)
                    if let task, task.sourceConnectorId != nil, let url = task.sourceExternalUrl.flatMap(URL.init(string:)) {
                        Link(destination: url) {
                            HStack(spacing: 6) {
                                Text(task.sourceExternalId.map { "#\($0)" } ?? task.sourceConnectorId ?? "")
                                Text("↗").foregroundStyle(Theme.inkFaint)
                            }
                            .font(.system(size: 12)).foregroundStyle(Theme.inkSecondary)
                            .padding(.horizontal, 8).padding(.vertical, 4)
                            .background(Theme.white(0.04), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                            .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.08)))
                        }
                        .padding(.horizontal, 16).padding(.bottom, 8)
                    }
                    DescriptionEditor(text: $form.description,
                                      placeholder: "Describe the task in detail, or type / for commands...")
                        .frame(minHeight: 200)
                        .padding(.horizontal, 16).padding(.bottom, 12)
                    sessionActions
                }
            }
            if isCreate { footer }
        }
        .frame(width: 420)
        .frame(maxHeight: .infinity)
        .background(Theme.surfacePanel)
        .overlay(alignment: .leading) { Rectangle().fill(Theme.white(0.08)).frame(width: 1) }
        .onAppear { if isCreate { titleFocused = true } }
        .onChange(of: form) { _, form in scheduleSave(form) }
        .onDisappear { flushSave() }
    }

    private var header: some View {
        HStack(spacing: 8) {
            if isCreate {
                Text("New Task").font(.system(size: 14, weight: .medium)).foregroundStyle(Theme.ink)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                Spacer(minLength: 0)
            }
            if let task {
                if task.isArchived {
                    GlyphButton(glyph: .archiveRestore, size: 13, strokeWidth: 1.5, hoverColor: Theme.ink, help: "Unarchive task") {
                        store.unarchive(task.id)
                    }
                } else if task.status.isTerminal {
                    GlyphButton(glyph: .archive, size: 13, strokeWidth: 1.5, hoverColor: Theme.ink, help: "Archive task") {
                        store.archive(task.id)
                    }
                }
                GlyphButton(glyph: .trash2, size: 13, strokeWidth: 1.5, hoverColor: Theme.danger, help: "Delete task") {
                    store.toggle("confirm:detail", anchor: trashFrame) {
                        ConfirmDelete(message: "Delete this task permanently?") {
                            store.closePopup()
                        } onConfirm: {
                            store.closePopup()
                            store.delete(task.id)
                            store.selectedTaskId = nil
                        }
                    }
                }
                .globalFrame($trashFrame)
            }
            GlyphButton(glyph: .x, strokeWidth: 1.5, hoverColor: .white, help: "Close") {
                store.selectedTaskId = nil
            }
        }
        .frame(height: 22)
        .padding(.horizontal, 16).padding(.vertical, 12)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.white(0.06)).frame(height: 1) }
    }

    private var properties: some View {
        VStack(alignment: .leading, spacing: 10) {
            row("Status") {
                StatusPicker(store: store, current: task?.status ?? .todo, disabled: isCreate, taskId: task?.id)
            }
            row("Project") {
                ProjectPicker(store: store, current: form.projectName, projects: store.projects) { form.projectName = $0 }
            }
            row("Branch") {
                HStack(spacing: 6) {
                    LucideIcon(.gitBranch, size: 11).foregroundStyle(Theme.inkFaint)
                    BareField(placeholder: "feature/my-task", text: $form.branch, font: .system(size: 12), color: Theme.inkSecondary)
                        .padding(.vertical, 2)
                }
            }
            row("Worktree") {
                Button { form.useWorktree.toggle() } label: {
                    HStack(spacing: 6) {
                        LucideIcon(.folderGit2, size: 13, strokeWidth: 1.5)
                        Text(form.useWorktree ? "Enabled" : "Disabled").font(.system(size: 12))
                    }
                    .foregroundStyle(form.useWorktree ? Theme.inkSecondary : Theme.inkFaint)
                    .padding(.vertical, 2)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            row("Agent") {
                AgentPicker(store: store, current: form.agent) { form.agent = $0 }
            }
            if let task {
                row("Created") { dated(.calendar, task.createdAt) }
                if let completed = task.completedAt {
                    row("Completed") { dated(.clock, completed) }
                }
            }
        }
        .padding(.horizontal, 16).padding(.vertical, 12)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.white(0.06)).frame(height: 1) }
    }

    @ViewBuilder
    private var sessionActions: some View {
        if let task, let open = store.openSession, store.isSessionLive(task) || task.agentSessionId != nil {
            let live = store.isSessionLive(task)
            HStack(spacing: 8) {
                Button { open(task) } label: {
                    HStack(spacing: 6) {
                        LucideIcon(live ? .terminal : .play, size: 12)
                        Text(live ? "Focus Session" : "Resume Session")
                    }
                    .font(.system(size: 12, weight: .medium)).foregroundStyle(Theme.inkSecondary)
                    .padding(.horizontal, 10).padding(.vertical, 6)
                    .background(Theme.white(0.04), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                    .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.08)))
                }
                .buttonStyle(.plain)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 16).padding(.vertical, 8)
            .overlay(alignment: .top) { Rectangle().fill(Theme.white(0.06)).frame(height: 1) }
        }
    }

    private var footer: some View {
        HStack(spacing: 8) {
            Spacer()
            Button { store.selectedTaskId = nil } label: {
                Text("Cancel").font(.system(size: 14)).foregroundStyle(Theme.inkSecondary)
                    .padding(.horizontal, 12).padding(.vertical, 6)
                    .background(Theme.white(0.04), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
            }
            .buttonStyle(.plain)
            Button {
                guard form.canSubmit else { return }
                var draft = form.draft
                draft.status = .todo
                Task {
                    if let id = await store.create(draft) { store.selectedTaskId = id }
                }
            } label: {
                HStack(spacing: 6) {
                    LucideIcon(.save, size: 13)
                    Text("Create Task")
                }
                .font(.system(size: 14, weight: .medium)).foregroundStyle(.white)
                .padding(.horizontal, 12).padding(.vertical, 6)
                .background(Theme.white(0.1), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
            }
            .buttonStyle(.plain)
            .disabled(!form.canSubmit)
            .opacity(form.canSubmit ? 1 : 0.3)
        }
        .padding(.horizontal, 16).padding(.vertical, 12)
        .overlay(alignment: .top) { Rectangle().fill(Theme.white(0.06)).frame(height: 1) }
    }

    private func row<C: View>(_ label: String, @ViewBuilder _ content: () -> C) -> some View {
        HStack(spacing: 8) {
            Text(label).font(.system(size: 12)).foregroundStyle(Theme.inkFaint).frame(width: 80, alignment: .leading)
            content()
            Spacer(minLength: 0)
        }
    }

    private func dated(_ glyph: LucideGlyph, _ date: String) -> some View {
        HStack(spacing: 4) {
            LucideIcon(glyph, size: 11)
            Text(TaskDates.long(date))
        }
        .font(.system(size: 12)).foregroundStyle(Theme.inkSecondary)
    }

    /// Existing tasks save half a second after the last edit, as today.
    private func scheduleSave(_ form: TaskForm) {
        guard let task else { return }
        saveTimer?.cancel()
        saveTimer = Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(500))
            guard !Task.isCancelled else { return }
            store.update(task.id, form.patch)
            saveTimer = nil
        }
    }

    private func flushSave() {
        guard let task, let timer = saveTimer else { return }
        timer.cancel()
        saveTimer = nil
        store.update(task.id, form.patch)
    }
}
