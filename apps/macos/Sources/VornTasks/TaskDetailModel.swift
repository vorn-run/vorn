import AppKit
import Foundation
import VornCore

/// An image picked before the task exists; it is uploaded once the task is created.
struct PendingImage: Identifiable, Hashable {
    let id = UUID()
    let data: Data
    let filename: String
}

/// TaskDetailPanel.tsx's state beyond the form: images, the task's workflow runs, and the review of its changes.
@MainActor
@Observable
public final class TaskDetailModel {
    /// The extensions vornd's `task:imageSave` takes.
    public static let imageExtensions: Set<String> = ["png", "jpg", "jpeg", "gif", "webp", "bmp"]

    let store: TasksStore
    public let taskId: String?

    public private(set) var imagePaths: [String: String] = [:]
    private(set) var pending: [PendingImage] = []

    public private(set) var runs: [WorkflowExecution] = []
    private(set) var workflows: [WorkflowDefinition] = []
    var showRuns = true
    var fullOutput: String?

    public private(set) var diff: GitDiffResult?
    public private(set) var diffLoading = false
    var showDiff = true
    var selectedFile: String?
    public private(set) var comments: [DiffComment] = []
    /// The line whose comment box is open; its `comment` is unused.
    var commentingLine: DiffComment?
    var commitDialogOpen = false

    public init(store: TasksStore, taskId: String?) {
        self.store = store
        self.taskId = taskId
    }

    var task: VornTask? { taskId.flatMap(store.task(id:)) }
    public var images: [String] { task?.images ?? [] }

    /// task.worktreePath || project.path.
    public var cwd: String {
        guard let task else { return "" }
        if let wt = task.worktreePath, !wt.isEmpty { return wt }
        return store.projects.first { $0.name == task.projectName }?.path ?? ""
    }

    /// The changes section shows while the task is being worked on or reviewed.
    public var showsChanges: Bool { task.map { $0.status == .inProgress || $0.status == .inReview } ?? false }

    var hasChanges: Bool { !(diff?.files.isEmpty ?? true) }

    // MARK: - Images

    public func loadImagePaths() async {
        guard let id = taskId, let extras = store.extras else { return }
        for name in images where imagePaths[name] == nil {
            if let path = try? await extras.taskImagePath(taskId: id, filename: name) { imagePaths[name] = path }
        }
    }

    /// handleAddImages / handleDrop: files are copied into the task's folder; before it exists they wait.
    public func addImages(_ urls: [URL]) async {
        let picked = urls.filter { Self.imageExtensions.contains($0.pathExtension.lowercased()) }
        guard !picked.isEmpty else { return }
        guard let id = taskId else {
            for url in picked {
                if let data = try? Data(contentsOf: url) { pending.append(PendingImage(data: data, filename: url.lastPathComponent)) }
            }
            return
        }
        guard let extras = store.extras else { return }
        var added: [String] = []
        for url in picked {
            do {
                let name = try await extras.saveTaskImage(taskId: id, sourcePath: url.path)
                imagePaths[name] = try? await extras.taskImagePath(taskId: id, filename: name)
                added.append(name)
            } catch {
                store.onToast(error.localizedDescription, .error)
            }
        }
        if !added.isEmpty { store.setImages(id, images + added) }
    }

    /// A pasted image: its bytes are stored under a fresh name.
    public func addImage(data: Data, filename: String = "pasted.png") async {
        guard let id = taskId else {
            pending.append(PendingImage(data: data, filename: filename))
            return
        }
        guard let extras = store.extras else { return }
        do {
            let name = try await extras.uploadTaskImage(taskId: id, data: data, filename: filename)
            imagePaths[name] = try? await extras.taskImagePath(taskId: id, filename: name)
            store.setImages(id, images + [name])
        } catch {
            store.onToast(error.localizedDescription, .error)
        }
    }

    public func removeImage(_ name: String) async {
        guard let id = taskId, let extras = store.extras else { return }
        do {
            try await extras.deleteTaskImage(taskId: id, filename: name)
        } catch {
            store.onToast(error.localizedDescription, .error)
            return
        }
        imagePaths[name] = nil
        store.setImages(id, images.filter { $0 != name })
    }

    func removePending(_ image: PendingImage) {
        pending.removeAll { $0.id == image.id }
    }

    /// After create: the images picked while drafting go to the new task.
    func uploadPending(to id: String) async {
        guard !pending.isEmpty, let extras = store.extras else { return }
        var names: [String] = []
        for image in pending {
            if let name = try? await extras.uploadTaskImage(taskId: id, data: image.data, filename: image.filename) {
                names.append(name)
            }
        }
        pending = []
        if !names.isEmpty { store.setImages(id, names) }
    }

    // MARK: - Workflow runs

    /// listWorkflowRunsByTask(task.id, 20), with the workflows that name their steps.
    public func loadRuns() async {
        guard let id = taskId, let extras = store.extras else { return }
        if let rows = try? await extras.listTaskRuns(taskId: id, limit: 20) { runs = rows }
        if !runs.isEmpty, workflows.isEmpty, let all = try? await extras.listWorkflows() { workflows = all }
    }

    /// Follows the runs while one is still going.
    func followRuns() async {
        await loadRuns()
        while !Task.isCancelled {
            try? await Task.sleep(for: .seconds(runs.contains { $0.status == .running } ? 2 : 10))
            guard !Task.isCancelled else { return }
            await loadRuns()
        }
    }

    func workflow(for run: WorkflowExecution) -> WorkflowDefinition? {
        workflows.first { $0.id == run.workflowId }
    }

    // MARK: - Changes

    /// fetchDiff: the worktree's uncommitted changes.
    public func loadDiff() async {
        let cwd = cwd
        guard showsChanges, !cwd.isEmpty, let extras = store.extras else {
            diff = nil
            return
        }
        diffLoading = true
        defer { diffLoading = false }
        do {
            diff = try await extras.gitDiff(cwd: cwd)
        } catch {
            diff = nil
        }
        if let selected = selectedFile, diff?.files.contains(where: { $0.filePath == selected }) != true { selectedFile = nil }
    }

    /// handleClickLine: a second click on the open line closes it.
    func clickLine(filePath: String, lineIndex: Int, lineContent: String) {
        if commentingLine?.filePath == filePath, commentingLine?.lineIndex == lineIndex {
            commentingLine = nil
        } else {
            commentingLine = DiffComment(filePath: filePath, lineIndex: lineIndex, lineContent: lineContent, comment: "")
        }
    }

    public func addComment(_ text: String) {
        guard var line = commentingLine else { return }
        line.comment = text
        comments.append(line)
        commentingLine = nil
    }

    public func comment(filePath: String, lineIndex: Int, lineContent: String, _ text: String) {
        clickLine(filePath: filePath, lineIndex: lineIndex, lineContent: lineContent)
        addComment(text)
    }

    func removeComment(at index: Int) {
        guard comments.indices.contains(index) else { return }
        comments.remove(at: index)
    }

    func clearComments() { comments = [] }

    /// What the send button says: an agent to send to, or the clipboard.
    var sendsToAgent: Bool {
        guard let task else { return false }
        return task.agentSessionId != nil || store.isSessionLive(task)
    }

    /// handleSendFeedback: into the live session, else a resumed one, else the clipboard.
    public func sendFeedback() async {
        guard !comments.isEmpty, let task else { return }
        let feedback = DiffReview.feedback(comments)
        let prompt = DiffReview.prompt(feedback, task: task, projectName: task.projectName)
        if store.isSessionLive(task), let sid = task.assignedSessionId, let write = store.writeSession {
            do {
                try await write(sid, "\u{1b}[200~\(prompt)\u{1b}[201~")
                try await write(sid, "\r")
                store.focusSession?(sid)
                store.onToast("Feedback sent to agent", .success)
            } catch {
                store.onToast(error.localizedDescription, .error)
                return
            }
        } else if store.canResume(task) {
            guard await store.resumeSession(task, initialPrompt: prompt) != nil else { return }
            store.onToast("Feedback sent to agent", .success)
        } else {
            store.copyText(feedback)
            store.onToast("Review feedback copied to clipboard", .info)
        }
        comments = []
        commentingLine = nil
    }

    /// CommitDialog.handleSubmit: commit, optionally push; the answer is the error to show, if any.
    public func commit(message: String, includeUnstaged: Bool, push: Bool) async -> String? {
        guard let extras = store.extras else { return "Commit failed" }
        let cwd = cwd
        do {
            let result = try await extras.gitCommit(cwd: cwd, message: message, includeUnstaged: includeUnstaged)
            guard result.success else { return result.error ?? "Commit failed" }
            if push {
                let pushed = try await extras.gitPush(cwd: cwd)
                guard pushed.success else {
                    await loadDiff()
                    return "Committed, but push failed: \(pushed.error ?? "unknown error")"
                }
                store.onToast("Committed and pushed", .success)
            } else {
                store.onToast("Changes committed", .success)
            }
            await loadDiff()
            return nil
        } catch {
            return error.localizedDescription
        }
    }
}
