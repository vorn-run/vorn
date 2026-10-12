import AppKit
import Foundation
import Testing
import VornCore
@testable import VornTasks
@testable import VornUI
import VornWorkflows

/// The task detail's extras and its description editor against a throwaway vornd.
@MainActor
struct TaskSurfacesLiveTests {
    @Test func extrasAndEditor() async throws {
        // vornd commits with a temporary HOME, so it gets an identity from the environment.
        for (k, v) in ["GIT_AUTHOR_NAME": "Test", "GIT_AUTHOR_EMAIL": "test@example.com",
                       "GIT_COMMITTER_NAME": "Test", "GIT_COMMITTER_EMAIL": "test@example.com"] { setenv(k, v, 1) }
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let vorn = VornStore(dataDir: server.dataDir, readOnly: false)
        vorn.start()
        defer { vorn.stop() }
        #expect(await until(timeout: 15) { vorn.phase == .connected })
        try #require(vorn.phase == .connected)
        let client = vorn.client
        let project = try await server.addProject("beta", client: client)
        let repo = server.root.appendingPathComponent(project)
        try git(repo, "init", "-q")
        try "one\n".write(to: repo.appendingPathComponent("notes.txt"), atomically: true, encoding: .utf8)
        try git(repo, "add", ".")
        try git(repo, "commit", "-qm", "First")
        try "one\ntwo\n".write(to: repo.appendingPathComponent("notes.txt"), atomically: true, encoding: .utf8)

        let tasks = TasksStore(service: client)
        await tasks.reload()
        let id = try #require(await tasks.create(TaskDraft(projectName: project, title: "Review notes")))
        tasks.update(id, TaskPatch(status: .inReview))
        #expect(await until { (try? await client.getTask(id: id))??.status == .inReview })
        await tasks.reload()
        let detail = TaskDetailModel(store: tasks, taskId: id)

        try await images(detail, client: client, id: id)
        try await changes(detail, repo: repo)
        try await description(tasks, client: client, id: id)
        try await runs(detail, client: client, id: id, project: project)
    }

    /// A pasted image is stored in the task's folder, shown from disk, and deleted again.
    private func images(_ detail: TaskDetailModel, client: VornClient, id: String) async throws {
        let png = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 2, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4,
                                   hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
            .representation(using: .png, properties: [:])!
        await detail.addImage(data: png)
        let name = try #require(detail.images.first)
        await detail.loadImagePaths()
        let path = try #require(detail.imagePaths[name])
        #expect(FileManager.default.fileExists(atPath: path))
        await detail.removeImage(name)
        #expect(detail.images.isEmpty)
        #expect(await until { !FileManager.default.fileExists(atPath: path) })
    }

    /// The worktree's change shows in the review, a comment collects, and committing clears it.
    private func changes(_ detail: TaskDetailModel, repo: URL) async throws {
        await detail.loadDiff()
        let file = try #require(detail.diff?.files.first)
        #expect(file.filePath == "notes.txt")
        #expect(file.insertions == 1)
        detail.comment(filePath: file.filePath, lineIndex: 1, lineContent: "two", "Say why")
        #expect(DiffReview.feedback(detail.comments).contains("Say why"))
        let error = await detail.commit(message: "Second", includeUnstaged: true, push: false)
        #expect(error == nil)
        #expect((detail.diff?.files ?? []).isEmpty)
        #expect(try git(repo, "log", "--oneline").contains("Second"))
    }

    /// Typing into the editor writes markdown the task keeps: input rules, a slash command, list Enter.
    private func description(_ tasks: TasksStore, client: VornClient, id: String) async throws {
        let view = RichTextView.make()
        view.frame = CGRect(x: 0, y: 0, width: 400, height: 300)
        view.load("")
        var saved = ""
        view.onMarkdown = { saved = $0 }
        func type(_ s: String) { for ch in s { view.insertText(String(ch), replacementRange: view.selectedRange()) } }
        type("## Plan")
        view.doCommand(by: #selector(NSResponder.insertNewline(_:)))
        type("Ship **it** now")
        view.doCommand(by: #selector(NSResponder.insertNewline(_:)))
        type("/task")
        let slash = try #require(view.floating)
        guard case .slash(_, let items, _) = slash.kind else { Issue.record("no slash menu"); return }
        #expect(items.map(\.title) == ["Task List"])
        view.doCommand(by: #selector(NSResponder.insertNewline(_:)))
        type("Write tests")
        view.doCommand(by: #selector(NSResponder.insertNewline(_:)))
        type("Merge")
        view.doCommand(by: #selector(NSResponder.insertNewline(_:)))
        view.doCommand(by: #selector(NSResponder.insertNewline(_:)))
        type("- ")
        type("bullet")
        #expect(saved == "## Plan\n\nShip **it** now\n\n- [ ] Write tests\n\n- [ ] Merge\n\n- bullet\n")

        tasks.update(id, TaskPatch(description: saved.trimmingCharacters(in: .whitespacesAndNewlines)))
        #expect(await until { (try? await client.getTask(id: id))??.description == "## Plan\n\nShip **it** now\n\n- [ ] Write tests\n\n- [ ] Merge\n\n- bullet" })

        // The stored description loads back to the same document.
        let reloaded = RichTextView.make()
        reloaded.load(try #require((try await client.getTask(id: id))?.description))
        #expect(RichMarkdown.serialize(reloaded.paragraphs()) == saved)
    }

    /// A run started with the task as its context lists under the task.
    private func runs(_ detail: TaskDetailModel, client: VornClient, id: String, project: String) async throws {
        let wfId = "wf-task-\(UUID().uuidString.prefix(8))"
        try await client.createWorkflow(WorkflowDefinition(
            id: wfId, name: "Task echo",
            nodes: [
                WorkflowNode(id: "trigger", type: .trigger, label: "Trigger", config: ["triggerType": "manual", "contextual": true]),
                WorkflowNode(id: "say", type: .script, label: "Say", slug: "say", config: [
                    "scriptType": "bash", "scriptContent": "echo {{task.title}}", "cwd": "/tmp",
                ], position: .init(x: 0, y: 120)),
            ],
            edges: [WorkflowEdge(id: "e0", source: "trigger", target: "say")],
            workspaceId: "personal"))
        let store = WorkflowsStore(backend: LiveWorkflowsBackend(client: client))
        await store.reload()
        let wf = try #require(store.workflow(wfId))
        let task = try #require(try await client.getTask(id: id))
        store.runNow(wf, context: ManualRunContext(task: try .from(task)))
        #expect(await until(timeout: 20) {
            await detail.loadRuns()
            return detail.runs.first?.status == .success
        }, "runs: \(detail.runs.map(\.status)) \(store.toast ?? "")")
        #expect(detail.workflow(for: try #require(detail.runs.first))?.name == "Task echo")
    }

    @discardableResult
    private func git(_ dir: URL, _ args: String...) throws -> String {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        p.arguments = ["-C", dir.path, "-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false"] + args
        let out = Pipe()
        p.standardOutput = out
        p.standardError = FileHandle.nullDevice
        try p.run()
        p.waitUntilExit()
        return String(decoding: out.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
    }

    private func until(timeout: TimeInterval = 10, _ check: @MainActor () async -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if await check() { return true }
            try? await Task.sleep(for: .milliseconds(100))
        }
        return false
    }
}
