import AppKit
import Foundation
import VornCore
import VornUI

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
    nonisolated public static let imageExtensions: Set<String> = ["png", "jpg", "jpeg", "gif", "webp", "bmp"]

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
    /// The task and folder the diff was last fetched for; nil while the task shows no changes.
    private var diffKey: String??
    var showDiff = true
    public let review = DiffReviewState()
    public var comments: [DiffComment] { review.comments }
    public var commitDialogOpen = false

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

    /// The totals over every file, as the panel's header and the commit dialog show them.
    var stat: DiffStat? {
        guard let diff else { return nil }
        return DiffStat(filesChanged: diff.files.count, insertions: diff.files.reduce(0) { $0 + $1.insertions },
                                  deletions: diff.files.reduce(0) { $0 + $1.deletions })
    }

    var hasChanges: Bool { stat.map { $0.insertions > 0 || $0.deletions > 0 } ?? false }

    /// Today's diffKey: the task and folder whose changes show, nil when none do.
    var currentDiffKey: String? {
        guard let taskId, showsChanges, !cwd.isEmpty else { return nil }
        return "\(taskId)|\(cwd)"
    }

    var taskTitles: [String: String] {
        Dictionary(store.tasks.map { ($0.id, $0.title) }, uniquingKeysWith: { a, _ in a })
    }

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
        // The shell's workflows store is empty until the Workflows view first opens, so the panel names runs itself.
        let known = Set((store.workflows?.workflows ?? []).map(\.id) + workflows.map(\.id))
        if runs.contains(where: { !known.contains($0.workflowId) }), let all = try? await extras.listWorkflows() {
            workflows = all
        }
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
        store.workflows?.workflow(run.workflowId) ?? workflows.first { $0.id == run.workflowId }
    }

    // MARK: - Changes

    /// Fetches the changes when the task or its folder changed, and drops them when it no longer shows any.
    public func syncDiff() async {
        let key = currentDiffKey
        guard diffKey != .some(key) else { return }
        diffKey = .some(key)
        if key == nil {
            diff = nil
            review.reset()
        } else {
            await loadDiff()
        }
    }

    /// fetchDiff: the worktree's uncommitted changes; a new diff starts the review over.
    public func loadDiff() async {
        let cwd = cwd
        guard showsChanges, !cwd.isEmpty, let extras = store.extras else { return }
        diffLoading = true
        defer { diffLoading = false }
        do {
            diff = try await extras.gitDiff(cwd: cwd)
            review.reset()
        } catch {}
    }

    /// What the send button says: "Send to Agent" when the task has a conversation, else "Copy Feedback".
    var sendsToAgent: Bool { task?.agentSessionId != nil }

    /// handleSendFeedback: the task's conversation resumed with the review as its prompt, else the clipboard.
    public func sendFeedback() async {
        guard !comments.isEmpty, let task else { return }
        let feedback = DiffComment.feedback(comments)
        if task.agentSessionId != nil, let agent = task.assignedAgent, agent.supportsExactSessionResume,
           store.projects.contains(where: { $0.name == task.projectName }) {
            let prompt = DiffReview.prompt(feedback, task: task, projectName: task.projectName)
            guard await store.resumeSession(task, initialPrompt: prompt) != nil else { return }
            store.onToast("Feedback sent to agent", .success)
        } else {
            store.copyText(feedback)
            store.onToast("Review feedback copied to clipboard", .info)
        }
        review.clearComments()
        review.commentingLine = nil
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
