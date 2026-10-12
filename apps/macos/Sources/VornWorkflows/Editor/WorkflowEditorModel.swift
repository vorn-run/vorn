import Foundation
import Observation
import VornCore

/// What the editor remembers between opens: canvas views and the step library's recent picks and open groups.
public struct EditorMemory: Sendable {
    public var read: @Sendable (String) -> Data?
    public var write: @Sendable (String, Data) -> Void

    public init(read: @escaping @Sendable (String) -> Data?, write: @escaping @Sendable (String, Data) -> Void) {
        self.read = read
        self.write = write
    }

    /// The app's own defaults, like the renderer's localStorage.
    public static let userDefaults = EditorMemory(
        read: { UserDefaults.standard.data(forKey: $0) },
        write: { UserDefaults.standard.set($1, forKey: $0) })

    /// Remembers within the process only, for renders and tests.
    public static func ephemeral() -> EditorMemory {
        let box = MemoryBox()
        return EditorMemory(read: { box.get($0) }, write: { box.set($0, $1) })
    }

    func json<T: Decodable>(_ key: String, as: T.Type) -> T? {
        read(key).flatMap { try? JSONDecoder().decode(T.self, from: $0) }
    }

    func setJSON<T: Encodable>(_ key: String, _ value: T) {
        if let data = try? JSONEncoder().encode(value) { write(key, data) }
    }
}

private final class MemoryBox: @unchecked Sendable {
    private var values: [String: Data] = [:]
    private let lock = NSLock()
    func get(_ key: String) -> Data? { lock.withLock { values[key] } }
    func set(_ key: String, _ value: Data) { lock.withLock { values[key] = value } }
}

/// canvas-views.ts: where each workflow's canvas was last left, fifty kept.
enum CanvasViews {
    static let key = "vorn:workflowViews"
    static let kept = 50

    struct Entry: Codable {
        var id: String
        var x: Double
        var y: Double
        var zoom: Double
    }

    static func read(_ memory: EditorMemory, _ workflowId: String) -> EditorLayout.Viewport? {
        guard let e = (memory.json(key, as: [Entry].self) ?? []).first(where: { $0.id == workflowId }),
              e.x.isFinite, e.y.isFinite, e.zoom.isFinite, e.zoom > 0 else { return nil }
        return EditorLayout.Viewport(x: e.x, y: e.y, zoom: e.zoom)
    }

    static func write(_ memory: EditorMemory, _ workflowId: String, _ view: EditorLayout.Viewport) {
        var list = (memory.json(key, as: [Entry].self) ?? []).filter { $0.id != workflowId }
        list.append(Entry(id: workflowId, x: view.x, y: view.y, zoom: view.zoom))
        memory.setJSON(key, Array(list.suffix(kept)))
    }
}

/// WorkflowEditor.tsx: the open workflow's draft and everything its page shows about it.
@Observable @MainActor
public final class WorkflowEditorModel {
    @ObservationIgnored unowned let store: WorkflowsStore
    /// Connections, packs and the catalog, for connector marks and the step library.
    public let directory = ConnectorDirectory()

    public private(set) var editingId: String?
    public var name = "New Workflow"
    public var icon = "Workflow"
    public var iconColor = "#3b82f6"
    public private(set) var nodes: [WorkflowNode] = []
    public private(set) var edges: [WorkflowEdge] = []
    public var enabled = true
    public var staggerDelayMs: Double?
    public var autoCleanupWorktrees = false
    public var selectedNodeId: String?
    var pendingInsert: InsertAnchor?
    public var showStartFrom = true
    public var showProperties = true
    public var iconPickerOpen = false
    public var overflowOpen = false
    /// The click answered before the server did; cleared when the run shows up or after 8s.
    public var launchingSince: Date?
    /// The canvas remounts per workflow; nil for a new one.
    public private(set) var loadKey: String?
    /// Bumped on every load, so the canvas re-opens its view.
    public private(set) var loadGeneration = 0
    /// The canvas has the keyboard: Tab, zoom keys and Delete go to it.
    public var canvasFocused = false
    var viewport: EditorLayout.Viewport?
    public var canvasSize: CGSize = .zero
    /// A search the library opens with, for renders.
    public var renderQuery: String?
    /// Offscreen renders only: a popup the step panel opens, and how far its form is scrolled.
    public var renderFormOpen: String?
    public var renderPanelScroll: Double = 0

    @ObservationIgnored private(set) var history = DefinitionHistory(WorkflowGraph(nodes: [], edges: []))
    @ObservationIgnored private var loadedSnapshot: WorkflowDefinition?
    @ObservationIgnored private var trackedRuns: [String: RunStatus] = [:]
    @ObservationIgnored private var followArmed = false

    init(store: WorkflowsStore) {
        self.store = store
    }

    // MARK: Derived

    public var graph: WorkflowGraph { WorkflowGraph(nodes: nodes, edges: edges) }
    public var isNew: Bool { editingId == nil }
    public var existing: WorkflowDefinition? { editingId.flatMap(store.workflow) }
    public var triggerNode: WorkflowNode? { nodes.first { $0.type == .trigger } }
    public var hasTrigger: Bool { triggerNode != nil }
    public var selectedNode: WorkflowNode? { selectedNodeId.flatMap { id in nodes.first { $0.id == id } } }
    public var showRunHistory: Bool {
        get { store.historyOpen }
        set { store.historyOpen = newValue }
    }

    /// Start-from wins the empty canvas of a new workflow and yields to everything opened deliberately.
    public var startFromOpen: Bool {
        editingId == nil && nodes.isEmpty && showStartFrom && pendingInsert == nil && !showRunHistory
            && selectedNode == nil
    }

    /// The runs of this workflow, newest first.
    public var runs: [WorkflowExecution] { editingId == nil ? [] : store.workflowRuns }

    public var runningRun: WorkflowExecution? {
        guard let id = editingId else { return nil }
        return store.runs.first { $0.workflowId == id && $0.status == .running }
            ?? store.workflowRuns.first { $0.status == .running }
    }

    /// run-presentation.ts liveNodeStatus: the most urgent state of each step across live runs.
    public var nodeStatus: [String: NodeExecutionStatus]? {
        guard let id = editingId else { return nil }
        return Self.liveNodeStatus(store.runs + store.workflowRuns, workflowId: id)
    }

    nonisolated static func liveNodeStatus(_ runs: [WorkflowExecution], workflowId: String) -> [String: NodeExecutionStatus]? {
        let rank: [NodeExecutionStatus: Int] = [.waiting: 4, .running: 3, .error: 2, .success: 1]
        var worst: [String: NodeExecutionStatus] = [:]
        var ranks: [String: Int] = [:]
        var seen = Set<String>()
        for run in runs where run.status == .running && run.workflowId == workflowId && seen.insert(run.runId).inserted {
            for ns in run.nodeStates {
                guard let r = rank[ns.status], r > (ranks[ns.nodeId] ?? 0) else { continue }
                worst[ns.nodeId] = ns.status
                ranks[ns.nodeId] = r
            }
        }
        return worst.isEmpty ? nil : worst
    }

    // MARK: Loading

    /// Opens a workflow, or a blank new one; switching resets the panels and history.
    func load(_ workflow: WorkflowDefinition?) {
        directory.load(store.backend)
        let before = graph
        let switched = editingId != workflow?.id || loadGeneration == 0
        editingId = workflow?.id
        if let workflow {
            fill(from: workflow)
            loadKey = workflow.id
        } else {
            name = "New Workflow"
            icon = "Workflow"
            iconColor = "#3b82f6"
            nodes = []
            edges = []
            enabled = true
            staggerDelayMs = nil
            autoCleanupWorktrees = false
            showStartFrom = true
            loadKey = nil
            loadedSnapshot = nil
        }
        if switched {
            selectedNodeId = nil
            pendingInsert = nil
            store.historyOpen = false
            showStartFrom = true
            launchingSince = nil
            followArmed = false
            trackedRuns = [:]
            iconPickerOpen = false
            overflowOpen = false
            history.reset(before)
            history.record(graph)
            viewport = nil
            loadGeneration += 1
        }
    }

    /// A saved new workflow becomes the open one: the canvas remounts and the panels reset, as a switch does.
    private func adopt(_ wf: WorkflowDefinition) {
        editingId = wf.id
        loadKey = wf.id
        selectedNodeId = nil
        pendingInsert = nil
        store.historyOpen = false
        showStartFrom = true
        history.reset(graph)
        viewport = nil
        loadGeneration += 1
        store.adoptEditing(wf.id)
    }

    /// A newer copy of the open workflow arrived; the draft takes it like the renderer's load effect.
    func sync(_ workflow: WorkflowDefinition) {
        guard workflow.id == editingId, workflow != loadedSnapshot else { return }
        fill(from: workflow)
        history.record(graph)
    }

    private func fill(from workflow: WorkflowDefinition) {
        loadedSnapshot = workflow
        name = workflow.name
        icon = workflow.icon
        iconColor = workflow.iconColor
        nodes = EditorOps.migrateSlugs(workflow.nodes)
        edges = workflow.edges
        enabled = workflow.enabled
        staggerDelayMs = workflow.staggerDelayMs
        autoCleanupWorktrees = workflow.autoCleanupWorktrees ?? false
    }

    // MARK: Graph edits

    /// Every edit lands here so undo sees it.
    func apply(_ next: WorkflowGraph, select: String?? = .none) {
        nodes = next.nodes
        edges = next.edges
        history.record(next)
        if case .some(let id) = select { selectedNodeId = id }
    }

    public func undo() {
        guard let g = history.undo() else { return }
        nodes = g.nodes
        edges = g.edges
    }

    public func redo() {
        guard let g = history.redo() else { return }
        nodes = g.nodes
        edges = g.edges
    }

    /// A step's form changed it; a new slug rewrites the other steps' references.
    func updateNode(_ node: WorkflowNode) {
        guard let old = nodes.first(where: { $0.id == node.id }) else { return }
        var next = nodes.map { $0.id == node.id ? node : $0 }
        if let oldSlug = old.slug, let newSlug = node.slug, oldSlug != newSlug {
            let others = PanelEdits.renameSlugReferences(next.filter { $0.id != node.id }, from: oldSlug, to: newSlug)
            next = next.map { n in n.id == node.id ? n : others.first { $0.id == n.id } ?? n }
        }
        apply(WorkflowGraph(nodes: next, edges: edges))
    }

    public func nodeClick(_ id: String?) {
        selectedNodeId = id
        showRunHistory = false
        pendingInsert = nil
        if id == nil { showProperties = true }
    }

    func openLibrary(_ anchor: InsertAnchor) {
        showRunHistory = false
        pendingInsert = anchor
    }

    private var firstProject: Project? { store.projects.first }

    /// handleLibraryPick.
    func pick(_ pick: LibraryPick) {
        guard let anchor = pendingInsert else { return }
        pendingInsert = nil
        if let position = anchor.position, anchor.beforeNodeId == nil, anchor.replaceNodeId == nil, !anchor.isTrigger,
           pick != .parallel, !pick.isTriggerPick
        {
            guard nodes.contains(where: { $0.id == anchor.afterNodeId }) else { return }
            paletteInsert(pick, after: anchor.afterNodeId, at: position)
            return
        }
        guard let result = EditorOps.applyPick(pick, at: anchor, to: graph, project: firstProject) else { return }
        apply(result.graph, select: .some(result.selected))
    }

    /// handlePaletteInsert: a step dropped where an edge was released, the rest laid out first if still on the seed column.
    private func paletteInsert(_ pick: LibraryPick, after afterNodeId: String, at position: CGPoint) {
        func placeAll(_ placed: [WorkflowNode], _ layoutEdges: [WorkflowEdge], _ newId: String?) -> [WorkflowNode] {
            let base = GraphOps.positionsAreSeed(nodes) ? EditorLayout.tidied(placed, layoutEdges) : placed
            return base.map { n in
                guard n.id == newId else { return n }
                var c = n
                c.position = WorkflowNodePosition(x: position.x, y: position.y)
                return c
            }
        }
        if pick == .type(.condition) {
            let result = GraphOps.insertConditionBetween(graph, after: afterNodeId, before: nil)
            let known = Set(nodes.map(\.id))
            let cond = result.nodes.first { $0.type == .condition && !known.contains($0.id) }
            apply(WorkflowGraph(nodes: placeAll(result.nodes, result.edges, cond?.id), edges: result.edges),
                  select: cond.map { .some($0.id) } ?? .none)
            return
        }
        guard let step = pick.step else { return }
        var node = EditorOps.createNode(step.type, in: nodes, project: firstProject)
        if let preset = step.config { node.config = node.config.merged(preset.mapValues { Optional($0) }) }
        let result = GraphOps.appendNodeAfter(graph, after: afterNodeId, node)
        apply(WorkflowGraph(nodes: placeAll(result.nodes, result.edges, node.id), edges: result.edges), select: .some(node.id))
    }

    public func deleteNode(_ id: String) {
        apply(EditorOps.delete(graph, nodeId: id))
        selectedNodeId = nil
        pendingInsert = nil
    }

    public func connect(_ source: String, _ target: String) {
        apply(WorkflowGraph(nodes: nodes, edges: edges + [WorkflowEdge(id: newGraphId(), source: source, target: target)]))
    }

    /// Dragged cards settled: every displayed top-level position is written, materialising the computed layout.
    public func commitPositions(_ positions: [String: CGPoint]) {
        let next = nodes.map { n -> WorkflowNode in
            guard let p = positions[n.id] else { return n }
            var c = n
            c.position = WorkflowNodePosition(x: p.x, y: p.y)
            return c
        }
        apply(WorkflowGraph(nodes: next, edges: edges))
    }

    public func tidyUp() {
        apply(WorkflowGraph(nodes: EditorLayout.tidied(nodes, edges), edges: edges))
    }

    // MARK: Saving and running

    /// persistWorkflow: the draft as a definition, written to vornd; a new one becomes the open workflow.
    @discardableResult
    public func persist() async -> WorkflowDefinition {
        let existing = self.existing
        var wf = existing ?? WorkflowDefinition(id: editingId ?? UUID().uuidString.lowercased(), name: name)
        wf.name = name
        wf.icon = icon
        wf.iconColor = iconColor
        wf.nodes = EditorLayout.alignedNodes(nodes)
        wf.edges = edges
        wf.enabled = enabled
        wf.staggerDelayMs = (staggerDelayMs ?? 0) != 0 ? staggerDelayMs : nil
        wf.autoCleanupWorktrees = autoCleanupWorktrees ? true : nil
        wf.lastRunAt = existing?.lastRunAt
        wf.lastRunStatus = existing?.lastRunStatus
        wf.workspaceId = existing?.workspaceId ?? store.workspaceId
        let isNew = editingId == nil
        loadedSnapshot = wf
        await store.saveWorkflow(wf, isNew: isNew)
        if isNew { adopt(wf) }
        return wf
    }

    public func save() {
        let wasNew = editingId == nil
        Task {
            _ = await persist()
            store.notify(wasNew ? "Workflow created" : "Workflow saved", tone: .success)
        }
    }

    /// handleRun and handleRunToStep: saved first, so the run is the graph on screen.
    public func run(toStep nodeId: String? = nil) {
        Task {
            let wf = await persist()
            if !RunInputs.needsPrompt(wf) { launchingSince = Date() }
            followArmed = true
            showRunHistory = true
            selectedNodeId = nil
            store.runNow(wf, targetNodeId: nodeId)
        }
    }

    public func stopRun() {
        if let run = runningRun { store.stopRun(run) }
    }

    public func deleteWorkflow() {
        if let existing { store.delete(existing) }
        store.closeEditor()
    }

    public func exportFile() {
        Task {
            let wf = await persist()
            store.exportWorkflow(wf)
        }
    }

    /// Overflow menu "Workflow settings": answers what start-from was asking.
    public func showSettings() {
        selectedNodeId = nil
        showRunHistory = false
        showStartFrom = false
        showProperties = true
        overflowOpen = false
    }

    public func toggleHistory() {
        let opening = !showRunHistory
        showRunHistory = opening
        if opening { selectedNodeId = nil }
    }

    /// A template's seed replaces the empty canvas.
    func applySeed(_ seed: WorkflowSeed) {
        name = seed.name
        if let i = seed.icon { icon = i }
        if let c = seed.iconColor { iconColor = c }
        apply(WorkflowGraph(nodes: seed.nodes, edges: seed.edges))
        showStartFrom = false
    }

    // MARK: Run feedback

    /// Called as runs change: the launched run takes over the spinner, and finished ones toast.
    func followRuns() {
        if let running = runningRun {
            launchingSince = nil
            if followArmed {
                followArmed = false
                trackedRuns[running.runId] = .running
            }
        } else if let since = launchingSince, Date().timeIntervalSince(since) > 8 {
            launchingSince = nil
        }
        for (runId, prev) in trackedRuns {
            guard let run = (store.runs + store.workflowRuns).first(where: { $0.runId == runId }), run.status != prev
            else { continue }
            if run.status == .running {
                trackedRuns[runId] = .running
                continue
            }
            trackedRuns[runId] = nil
            if run.status == .success {
                let steps = RunPresenter.stepProgress(run, nodes: nodes).done
                let duration = TimeFormat.runDuration(run.startedAt, run.completedAt)
                store.notify("Run finished — \(steps) step\(steps == 1 ? "" : "s") in \(duration)", tone: .success)
            } else if run.status == .error && !RunPresenter.isRejected(run) {
                let failed = run.nodeStates.first { $0.status == .error }
                let node = failed.flatMap { f in nodes.first { $0.id == f.nodeId } }
                store.notify(node.map { "Run failed at \"\($0.label)\"" } ?? "Run failed", tone: .error)
            }
        }
    }

    // MARK: Canvas views

    func rememberedView() -> EditorLayout.Viewport? {
        loadKey.flatMap { CanvasViews.read(store.editorMemory, $0) }
    }

    func rememberView(_ view: EditorLayout.Viewport) {
        if let key = loadKey { CanvasViews.write(store.editorMemory, key, view) }
    }
}

extension LibraryPick {
    /// Picks that only make sense at the trigger spot.
    var isTriggerPick: Bool {
        switch self {
        case .triggerType, .connectorTrigger: return true
        default: return false
        }
    }
}

extension WorkflowEditorModel {
    /// Puts the editor in a state a render shows: a selected step, the library, a popover.
    public func driveForRender(
        select: String? = nil, library: Bool = false, query: String? = nil, iconPicker: Bool = false,
        more: Bool = false, triggerLibrary: Bool = false, formOpen: String? = nil, panelScroll: Double = 0
    ) {
        renderFormOpen = formOpen
        renderPanelScroll = panelScroll
        if let select, let node = nodes.first(where: { $0.label == select }) { nodeClick(node.id) }
        if library {
            let outgoing = Set(edges.map(\.source))
            if let leaf = nodes.reversed().first(where: { !outgoing.contains($0.id) }) {
                openLibrary(InsertAnchor(afterNodeId: leaf.id, beforeNodeId: nil))
            }
        }
        if triggerLibrary { openLibrary(.trigger) }
        renderQuery = query
        iconPickerOpen = iconPicker
        overflowOpen = more
    }
}
