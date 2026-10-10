import SwiftUI
import VornCore
import VornUI

/// The description a new task starts from.
let taskTemplate = """
## Description
Describe what needs to be done...

## Acceptance Criteria
- [ ] Criterion 1
- [ ] Criterion 2

## Notes
Any additional context or references.

"""

/// The task form's fields, shared by the dialog and the detail panel.
struct TaskForm: Equatable {
    var title = ""
    var projectName = ""
    var description = taskTemplate
    var status: TaskStatus = .todo
    var branch = ""
    var useWorktree = false
    var agent: AgentKind?

    init(projectName: String, status: TaskStatus = .todo) {
        self.projectName = projectName
        self.status = status
    }

    init(task: VornTask) {
        title = task.title
        projectName = task.projectName
        description = task.description
        status = task.status
        branch = task.branch ?? ""
        useWorktree = task.useWorktree ?? false
        agent = task.assignedAgent
    }

    var canSubmit: Bool {
        !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !projectName.isEmpty
            && !description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var draft: TaskDraft {
        let branch = branch.trimmingCharacters(in: .whitespaces)
        return TaskDraft(
            projectName: projectName,
            title: title.trimmingCharacters(in: .whitespacesAndNewlines),
            description: description.trimmingCharacters(in: .whitespacesAndNewlines),
            status: status,
            branch: branch.isEmpty ? nil : branch,
            useWorktree: useWorktree ? true : nil,
            assignedAgent: agent
        )
    }

    /// Everything but the status, which moves by its own rules.
    var patch: TaskPatch {
        TaskPatch(
            projectName: projectName,
            title: title.trimmingCharacters(in: .whitespacesAndNewlines),
            description: description.trimmingCharacters(in: .whitespacesAndNewlines),
            branch: branch.trimmingCharacters(in: .whitespaces),
            useWorktree: useWorktree,
            assignedAgent: agent,
            clearsAgent: agent == nil
        )
    }
}

/// A plain text field without the system chrome.
struct BareField: View {
    let placeholder: String
    @Binding var text: String
    var font: Font
    var color: Color = Theme.ink

    var body: some View {
        TextField("", text: $text, prompt: Text(placeholder).foregroundStyle(Theme.inkFaint))
            .textFieldStyle(.plain)
            .font(font)
            .foregroundStyle(color)
    }
}

/// The markdown body as plain text, with its placeholder.
struct DescriptionEditor: View {
    @Binding var text: String
    let placeholder: String

    var body: some View {
        TextEditor(text: $text)
            .font(.system(size: 14))
            .foregroundStyle(Theme.ink)
            .scrollContentBackground(.hidden)
            .background(.clear)
            .padding(.horizontal, -5)
            .overlay(alignment: .topLeading) {
                if text.isEmpty {
                    Text(placeholder).font(.system(size: 14)).foregroundStyle(Theme.inkFaint).allowsHitTesting(false)
                }
            }
    }
}

/// AddTaskDialog: the floating quick-add form.
struct AddTaskDialog: View {
    let store: TasksStore
    @State private var form: TaskForm
    @FocusState private var titleFocused: Bool
    private let editing: VornTask?

    init(store: TasksStore) {
        self.store = store
        editing = store.dialogTask
        if let task = store.dialogTask {
            _form = State(initialValue: TaskForm(task: task))
        } else {
            _form = State(initialValue: TaskForm(projectName: store.defaultProjectName, status: store.dialogStatus))
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text(editing == nil ? "New task" : "Edit Task").font(.system(size: 13)).foregroundStyle(Theme.inkFaint)
                Spacer()
                HStack(spacing: 4) {
                    GlyphButton(glyph: .maximize2, hoverColor: .white, help: "Expand to full panel") {
                        close()
                        store.selectedTaskId = TasksStore.newTaskId
                    }
                    GlyphButton(glyph: .x, hoverColor: .white, help: "Close", action: close)
                }
            }
            .padding(.horizontal, 16).padding(.vertical, 10)
            divider

            VStack(alignment: .leading, spacing: 0) {
                BareField(placeholder: "Task title", text: $form.title, font: .system(size: 15, weight: .medium))
                    .focused($titleFocused)
                    .padding(.horizontal, 16).padding(.top, 12).padding(.bottom, 4)
                DescriptionEditor(text: $form.description, placeholder: "Add description...")
                    .frame(minHeight: 120, maxHeight: 300)
                    .padding(.horizontal, 16).padding(.bottom, 12)
            }

            divider
            HStack(spacing: 8) {
                StatusPicker(store: store, current: form.status) { form.status = $0 }
                ProjectPicker(store: store, current: form.projectName, projects: store.scopedProjects) { form.projectName = $0 }
                AgentPicker(store: store, current: form.agent) { form.agent = $0 }
                if !form.branch.isEmpty || editing != nil {
                    HStack(spacing: 4) {
                        LucideIcon(.folderGit2, size: 10)
                        Text(form.branch.isEmpty ? "branch" : form.branch)
                    }
                    .font(.system(size: 12)).foregroundStyle(Theme.inkSecondary)
                    .padding(.horizontal, 8).padding(.vertical, 4)
                    .background(Theme.white(0.06), in: Capsule())
                }
                Spacer(minLength: 0)
                GlyphButton(glyph: .paperclip, padding: 6, help: "Attach images") {}
            }
            .padding(.horizontal, 16).padding(.vertical, 8)

            divider
            HStack {
                Spacer()
                Button(action: submit) {
                    Text(editing == nil ? "Create task" : "Save")
                        .font(.system(size: 14, weight: .medium)).foregroundStyle(.white)
                        .padding(.horizontal, 12).padding(.vertical, 6)
                        .background(Theme.white(0.1), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
                }
                .buttonStyle(.plain)
                .disabled(!form.canSubmit)
                .opacity(form.canSubmit ? 1 : 0.3)
                .keyboardShortcut(.return, modifiers: .command)
            }
            .padding(.horizontal, 16).padding(.vertical, 12)
        }
        .frame(width: 560)
        .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusXl))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusXl).strokeBorder(Theme.white(0.1)))
        .clipShape(RoundedRectangle(cornerRadius: Theme.radiusXl))
        .floatingShadow()
        .onAppear { titleFocused = true }
    }

    private var divider: some View {
        Rectangle().fill(Theme.white(0.06)).frame(height: 1)
    }

    private func close() {
        store.dialogOpen = false
        store.dialogTask = nil
    }

    private func submit() {
        guard form.canSubmit else { return }
        if let editing {
            store.update(editing.id, {
                var p = form.patch
                p.status = form.status
                return p
            }(), toast: "Task updated")
            close()
        } else {
            let draft = form.draft
            close()
            Task { await store.create(draft) }
        }
    }
}
