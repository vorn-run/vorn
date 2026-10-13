import CoreGraphics
import Foundation
import Testing
import VornCore
@testable import VornWorkflows

/// The workflow editor against a throwaway vornd: build a new workflow through the library, edit it, save, read it back.
@MainActor
struct WorkflowEditorLiveTests {
    @Test func createEditSaveAndReadBack() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let vorn = VornStore(dataDir: server.dataDir, readOnly: false)
        vorn.start()
        defer { vorn.stop() }
        #expect(await until(timeout: 15) { vorn.phase == .connected })
        try #require(vorn.phase == .connected)
        let backend = LiveWorkflowsBackend(client: vorn.client)
        let store = WorkflowsStore(backend: backend)
        store.editorMemory = .ephemeral()
        await store.reload()

        // New workflow: a blank editor offering Start from.
        store.newWorkflow()
        let editor = try #require(store.editor)
        #expect(store.isEditing && editor.isNew && editor.startFromOpen)
        #expect(editor.name == "New Workflow" && editor.icon == "Workflow")

        // The trigger spot, then a step through the library's pick path.
        editor.openLibrary(.trigger)
        #expect(!editor.startFromOpen)
        editor.pick(.triggerType("manual"))
        let trigger = try #require(editor.triggerNode)
        #expect(editor.selectedNodeId == trigger.id && editor.pendingInsert == nil)
        editor.openLibrary(InsertAnchor(afterNodeId: trigger.id, beforeNodeId: nil))
        editor.pick(.type(.script))
        let script = try #require(editor.nodes.first { $0.type == .script })
        #expect(editor.selectedNodeId == script.id)
        #expect(editor.edges.contains { $0.source == trigger.id && $0.target == script.id })

        // Insert between them, from the edge's +.
        editor.openLibrary(InsertAnchor(afterNodeId: trigger.id, beforeNodeId: script.id))
        editor.pick(.type(.approval))
        let gate = try #require(editor.nodes.first { $0.type == .approval })
        #expect(editor.edges.contains { $0.source == trigger.id && $0.target == gate.id })
        #expect(editor.edges.contains { $0.source == gate.id && $0.target == script.id })

        // Undo takes the insert back; redo puts it again.
        editor.undo()
        #expect(!editor.nodes.contains { $0.id == gate.id })
        editor.redo()
        #expect(editor.nodes.contains { $0.id == gate.id })

        // A step dropped on the canvas from the script's port lands where it was released.
        editor.openLibrary(InsertAnchor(afterNodeId: script.id, beforeNodeId: nil, position: CGPoint(x: 320, y: 504)))
        editor.pick(.type(.httpRequest))
        let http = try #require(editor.selectedNode)
        #expect(http.type == .httpRequest && http.position == WorkflowNodePosition(x: 320, y: 504))
        #expect(editor.edges.contains { $0.source == script.id && $0.target == http.id })
        #expect(!GraphOps.positionsAreSeed(editor.nodes))

        // Connected by hand, within the rules.
        #expect(EditorLayout.canConnect(editor.nodes, editor.edges, source: gate.id, target: http.id))
        #expect(!EditorLayout.canConnect(editor.nodes, editor.edges, source: http.id, target: trigger.id))
        #expect(!EditorLayout.canConnect(editor.nodes, editor.edges, source: http.id, target: gate.id))
        editor.connect(gate.id, http.id)
        #expect(editor.edges.contains { $0.source == gate.id && $0.target == http.id })

        // A parallel branch off the trigger, then deleted: its edges go and its neighbours are bridged.
        editor.openLibrary(InsertAnchor(afterNodeId: trigger.id, beforeNodeId: nil))
        editor.pick(.parallel)
        let branch = try #require(editor.selectedNode)
        #expect(branch.type == .launchAgent)
        editor.deleteNode(branch.id)
        #expect(!editor.nodes.contains { $0.id == branch.id })
        #expect(!editor.edges.contains { $0.source == branch.id || $0.target == branch.id })
        #expect(editor.selectedNodeId == nil)

        // Move: dragged cards land on the lattice and every top-level position is written.
        let placed = EditorLayout.toCanvasElements(editor.nodes, editor.edges)
        var positions = Dictionary(placed.cards.map { ($0.id, $0.absolute) }, uniquingKeysWith: { a, _ in a })
        positions[script.id] = CGPoint(x: GraphOps.snapToLattice(203), y: GraphOps.snapToLattice(301))
        editor.commitPositions(positions)

        // Rename a step through its panel binding; the name and icon in the bar.
        var renamed = PanelEdits.setLabel(try #require(editor.nodes.first { $0.id == script.id }), "Say hi", others: editor.nodes)
        renamed.config.set("scriptContent", .string("echo hi"))
        editor.updateNode(renamed)
        editor.name = "Editor live"
        editor.icon = "Rocket"
        editor.iconColor = "#22c55e"

        let saved = await editor.persist()
        #expect(store.editingWorkflowId == saved.id && !editor.isNew)
        let back = try #require(try await backend.getWorkflow(saved.id))
        #expect(back.name == "Editor live" && back.icon == "Rocket" && back.iconColor == "#22c55e")
        #expect(Set(back.nodes.map(\.id)) == Set([trigger.id, gate.id, script.id, http.id]))
        #expect(back.edges == editor.edges)
        let say = try #require(back.nodes.first { $0.id == script.id })
        #expect(say.label == "Say hi" && say.slug == "say_hi" && say.config.str("scriptContent") == "echo hi",
                "\(say.label) \(say.slug ?? "-") \(say.config)")
        #expect(say.position.x == 200 && say.position.y == 304)

        // A second save is an update, not another workflow.
        editor.name = "Editor live 2"
        await editor.persist()
        #expect(try await backend.getWorkflow(saved.id)?.name == "Editor live 2")
        #expect(try await backend.listWorkflows().filter { $0.name.hasPrefix("Editor live") }.count == 1)

        // Reopening loads the saved draft with fresh panels.
        store.showAllRuns()
        await store.reload()
        await store.openWorkflow(saved.id).value
        #expect(editor.nodes.count == 4 && editor.selectedNodeId == nil && !editor.showRunHistory)
        #expect(editor.name == "Editor live 2")

        // Delete from the overflow menu.
        editor.deleteWorkflow()
        #expect(!store.isEditing)
        #expect(await until(timeout: 5) { (try? await backend.listWorkflows())?.contains { $0.id == saved.id } == false })
    }

    private func until(timeout: TimeInterval = 10, _ check: @MainActor () async -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if await check() { return true }
            try? await Task.sleep(for: .milliseconds(150))
        }
        return false
    }
}
