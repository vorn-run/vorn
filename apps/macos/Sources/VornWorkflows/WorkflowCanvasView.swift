import AppKit
import SwiftUI
import VornCore
import VornUI

/// workflow-canvas-layout.ts stepEdgePath: straight between ports when nearly aligned, else the bottom-to-top bezier.
enum EdgeGeometry {
    static func controlOffset(_ distance: CGFloat) -> CGFloat {
        distance >= 0 ? 0.5 * distance : 0.25 * 25 * sqrt(-distance)
    }

    /// The path and the label point between a source port (below a card) and a target port (above one).
    static func path(_ s: CGPoint, _ t: CGPoint) -> (path: Path, label: CGPoint, samples: [CGPoint]) {
        var p = Path()
        p.move(to: s)
        if abs(s.x - t.x) < EditorLayout.straightEdgeTolerance {
            p.addLine(to: t)
            return (p, CGPoint(x: (s.x + t.x) / 2, y: (s.y + t.y) / 2), [s, t])
        }
        let c1 = CGPoint(x: s.x, y: s.y + controlOffset(t.y - s.y))
        let c2 = CGPoint(x: t.x, y: t.y - controlOffset(t.y - s.y))
        p.addCurve(to: t, control1: c1, control2: c2)
        func at(_ u: CGFloat) -> CGPoint {
            let a = (1 - u) * (1 - u) * (1 - u), b = 3 * (1 - u) * (1 - u) * u, c = 3 * (1 - u) * u * u, d = u * u * u
            return CGPoint(x: a * s.x + b * c1.x + c * c2.x + d * t.x, y: a * s.y + b * c1.y + c * c2.y + d * t.y)
        }
        return (p, at(0.5), (0...20).map { at(CGFloat($0) / 20) })
    }

    static func distance(_ p: CGPoint, toPolyline pts: [CGPoint]) -> CGFloat {
        var best = CGFloat.infinity
        for (a, b) in zip(pts, pts.dropFirst()) {
            let dx = b.x - a.x, dy = b.y - a.y
            let len = dx * dx + dy * dy
            let u = len == 0 ? 0 : max(0, min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / len))
            best = min(best, hypot(p.x - (a.x + u * dx), p.y - (a.y + u * dy)))
        }
        return best
    }
}

/// WorkflowCanvas.tsx: the editable graph on a pannable, zoomable dotted ground.
struct WorkflowCanvasView: View {
    @Bindable var editor: WorkflowEditorModel
    var staticLayout = false

    @State private var dragStart: [String: CGPoint]?
    @State private var dragPositions: [String: CGPoint] = [:]
    @State private var hoveredCard: String?
    @State private var hoveredToolbar: String?
    @State private var hoveredEdge: String?
    @State private var edgeLeave: Task<Void, Never>?
    @State private var connecting: Connecting?
    @State private var panStart: EditorLayout.Viewport?
    @State private var openedView: EditorLayout.Viewport?
    @State private var scrollSettle: Task<Void, Never>?
    @State private var catcher = CanvasEventCatcher.Box()

    struct Connecting {
        var source: String
        var fromBody: Bool
        var start: CGPoint
        var pointer: CGPoint
    }

    private var nodes: [WorkflowNode] { editor.nodes }

    var body: some View {
        let elements = EditorLayout.toCanvasElements(editor.nodes, editor.edges)
        GeometryReader { geo in
            let v = editor.viewport ?? EditorLayout.Viewport(x: geo.size.width / 2, y: EditorLayout.openingTop, zoom: 1)
            ZStack(alignment: .topLeading) {
                ground(v, size: geo.size)
                flowLayer(elements)
                    .scaleEffect(v.zoom, anchor: .topLeading)
                    .offset(x: v.x, y: v.y)
                if let c = connecting { connectionLine(c, elements: elements, v: v) }
                controls(elements, size: geo.size)
                    .padding(15)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomLeading)
                CanvasMinimap(elements: elements, positions: livePositions(elements), viewport: v, size: geo.size) { dx, dy in
                    pan(dx: dx, dy: dy)
                }
                .padding(15)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomTrailing)
            }
            .coordinateSpace(.named("canvas"))
            .clipped()
            .contentShape(Rectangle())
            .onContinuousHover(coordinateSpace: .named("canvas")) { phase in
                if case .active(let p) = phase { hoverEdges(at: p, elements: elements, v: v) } else { leaveEdge() }
            }
            .background(
                CanvasEventCatcher(box: catcher)
                    .allowsHitTesting(false))
            .onAppear {
                editor.canvasSize = geo.size
                openView(elements, width: geo.size.width)
                bindEvents(elements)
            }
            .onChange(of: geo.size) { _, s in editor.canvasSize = s }
            .onChange(of: editor.loadGeneration) { _, _ in
                openView(EditorLayout.toCanvasElements(editor.nodes, editor.edges), width: geo.size.width)
            }
            .onChange(of: elements) { _, e in bindEvents(e) }
        }
        .background(Theme.surfaceBase)
    }

    // MARK: Ground

    private func ground(_ v: EditorLayout.Viewport, size: CGSize) -> some View {
        Canvas { ctx, size in
            let gap = 20 * v.zoom
            guard gap > 2 else { return }
            let r = max(0.5, 0.5 * v.zoom)
            var y = v.y.truncatingRemainder(dividingBy: gap) - gap
            while y < size.height + gap {
                var x = v.x.truncatingRemainder(dividingBy: gap) - gap
                while x < size.width + gap {
                    ctx.fill(Path(ellipseIn: CGRect(x: x - r, y: y - r, width: 2 * r, height: 2 * r)), with: .color(Theme.white(0.05)))
                    x += gap
                }
                y += gap
            }
        }
        .contentShape(Rectangle())
        .onTapGesture {
            editor.canvasFocused = true
            editor.nodeClick(nil)
        }
        .gesture(
            DragGesture(minimumDistance: 2, coordinateSpace: .named("canvas"))
                .onChanged { g in
                    let start = panStart ?? v
                    panStart = start
                    editor.viewport = EditorLayout.Viewport(
                        x: start.x + g.translation.width, y: start.y + g.translation.height, zoom: start.zoom)
                }
                .onEnded { _ in
                    panStart = nil
                    moveEnded()
                })
    }

    // MARK: Flow layer

    private func livePositions(_ elements: EditorLayout.Elements) -> [String: CGPoint] {
        var out: [String: CGPoint] = [:]
        for card in elements.cards where card.parentId == nil { out[card.id] = dragPositions[card.id] ?? card.absolute }
        for card in elements.cards {
            guard let parent = card.parentId, let at = out[parent] else { continue }
            out[card.id] = CGPoint(x: at.x + card.position.x, y: at.y + card.position.y)
        }
        return out
    }

    private func drawnHeight(_ card: EditorLayout.Card) -> CGFloat {
        card.isLoop ? card.height : CardText.drawnHeight(card.node)
    }

    /// The source port under a card, or the loop's body port; edges leave from its far side.
    private func sourcePoint(_ card: EditorLayout.Card, _ at: CGPoint, body: Bool) -> CGPoint {
        body ? CGPoint(x: at.x + card.width / 2, y: at.y + EditorLayout.loopHeader + 10.5)
            : CGPoint(x: at.x + card.width / 2, y: at.y + drawnHeight(card) + 3.5)
    }

    private struct DrawnEdge: Identifiable {
        var id: String
        var path: Path
        var label: CGPoint
        var samples: [CGPoint]
        var text: String?
        var data: EditorLayout.EdgeData?
        /// Inside a loop's frame, so drawn over it.
        var inBody = false
    }

    private func drawnEdges(_ elements: EditorLayout.Elements, _ pos: [String: CGPoint]) -> [DrawnEdge] {
        var out: [DrawnEdge] = []
        for e in elements.edges {
            guard let sc = elements.card(e.source), let tc = elements.card(e.target),
                  let sp = pos[e.source], let tp = pos[e.target] else { continue }
            let s = sourcePoint(sc, sp, body: e.fromBodyHandle)
            let t = CGPoint(x: tp.x + tc.width / 2, y: tp.y - 3.5)
            let g = EdgeGeometry.path(s, t)
            out.append(DrawnEdge(id: e.id, path: g.path, label: g.label, samples: g.samples, text: e.label, data: e.data,
                                 inBody: tc.parentId != nil))
        }
        for add in elements.addSteps {
            guard let sc = elements.card(add.afterNodeId), let sp = pos[add.afterNodeId] else { continue }
            let g = EdgeGeometry.path(sourcePoint(sc, sp, body: false), CGPoint(x: add.position.x + 11, y: add.position.y - 3.5))
            out.append(DrawnEdge(id: add.id, path: g.path, label: g.label, samples: g.samples, text: nil, data: nil))
        }
        return out
    }

    private func flowLayer(_ elements: EditorLayout.Elements) -> some View {
        let pos = livePositions(elements)
        let edges = drawnEdges(elements, pos)
        let status = editor.nodeStatus
        let bodyMembers = Set(GraphOps.loopBodyOwners(nodes).keys)
        let anchor = editor.pendingInsert
        return ZStack(alignment: .topLeading) {
            ForEach(edges.filter { !$0.inBody }) { e in
                e.path.stroke(Theme.white(0.16), lineWidth: 1.5)
            }
            ForEach(elements.cards.filter { $0.parentId == nil && $0.isLoop }) { card in
                loopView(card, elements: elements, at: pos[card.id] ?? card.absolute, status: status?[card.id], anchor: anchor)
            }
            ForEach(edges.filter(\.inBody)) { e in
                e.path.stroke(Theme.white(0.16), lineWidth: 1.5)
            }
            ForEach(elements.cards.filter { !$0.isLoop }) { card in
                stepView(card, at: pos[card.id] ?? card.absolute, status: status?[card.id], inBody: bodyMembers.contains(card.id))
                    .zIndex(card.id == editor.selectedNodeId ? 2 : 1)
            }
            ForEach(elements.addSteps) { add in
                ConnectorButton(active: anchor?.afterNodeId == add.afterNodeId && anchor?.beforeNodeId == nil && anchor?.replaceNodeId == nil) {
                    openLibrary(InsertAnchor(afterNodeId: add.afterNodeId, beforeNodeId: nil, insideBranch: add.insideBranch))
                }
                .offset(x: add.position.x, y: add.position.y)
                .zIndex(3)
            }
            if let t = elements.addTrigger {
                AddTriggerCard(active: anchor?.isTrigger == true) { openLibrary(.trigger) }
                    .offset(x: t.x, y: t.y)
            }
            ForEach(edges.filter { $0.text != nil || ($0.data != nil && (hoveredEdge == $0.id || anchored($0.data, anchor))) }) { e in
                edgeLabel(e, anchor: anchor)
                    .zIndex(hoveredEdge == e.id ? 5 : 4)
            }
            ForEach(elements.cards.filter { !$0.isLoop || $0.parentId == nil }) { card in
                if hoveredCard == card.id || hoveredToolbar == card.id, let at = pos[card.id] {
                    NodeHoverToolbar(
                        node: card.node, inBody: bodyMembers.contains(card.id),
                        onRunToStep: { editor.run(toStep: card.id) },
                        onReplace: { replace(card.node, inBody: bodyMembers.contains(card.id)) },
                        onDelete: { editor.deleteNode(card.id) })
                    .fixedSize()
                    .onHover { h in hoveredToolbar = h ? card.id : (hoveredToolbar == card.id ? nil : hoveredToolbar) }
                    .frame(height: drawnHeight(card))
                    .offset(x: at.x + card.width + 6, y: at.y)
                    .zIndex(6)
                }
            }
        }
        .frame(width: 0, height: 0, alignment: .topLeading)
    }

    private func anchored(_ data: EditorLayout.EdgeData?, _ anchor: InsertAnchor?) -> Bool {
        guard let data, let anchor else { return false }
        return anchor.afterNodeId == data.afterNodeId && anchor.beforeNodeId == data.beforeNodeId
    }

    private func edgeLabel(_ e: DrawnEdge, anchor: InsertAnchor?) -> some View {
        let isAnchored = anchored(e.data, anchor)
        return VStack(spacing: 4) {
            if let text = e.text {
                Text(text)
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(Theme.inkSecondary)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 2)
                    .background(Theme.surfaceBase, in: Capsule())
                    .overlay(Capsule().strokeBorder(Theme.white(0.08), lineWidth: 1))
            }
            if let data = e.data, hoveredEdge == e.id || isAnchored {
                ConnectorButton(active: isAnchored) {
                    openLibrary(InsertAnchor(
                        afterNodeId: data.afterNodeId, beforeNodeId: data.beforeNodeId,
                        insideBranch: data.insideBranch, bodyOnly: data.bodyOf != nil))
                }
            }
        }
        .fixedSize()
        .onHover { h in if h { enterEdge(e.id) } else { leaveEdge() } }
        .frame(width: 0, height: 0)
        .offset(x: e.label.x, y: e.label.y)
    }

    private func stepView(_ card: EditorLayout.Card, at: CGPoint, status: NodeExecutionStatus?, inBody: Bool) -> some View {
        let node = card.node
        return NodeCardView(node: node, directory: editor.directory, selected: node.id == editor.selectedNodeId, status: status)
            .onHover { h in hoveredCard = h ? node.id : (hoveredCard == node.id ? nil : hoveredCard) }
            .onTapGesture {
                editor.canvasFocused = true
                editor.nodeClick(node.id)
            }
            .gesture(card.parentId == nil ? dragCard(card.id) : nil)
            .overlay(alignment: .top) {
                if node.type != .trigger { PortDot().offset(y: -3.5).allowsHitTesting(false) }
            }
            .overlay(alignment: .bottom) {
                PortDot().offset(y: 3.5).gesture(connectGesture(node.id, fromBody: false, origin: { sourcePoint(card, livePositions(currentElements()).first { $0.key == card.id }?.value ?? at, body: false) }))
            }
            .offset(x: at.x, y: at.y)
    }

    private func loopView(_ card: EditorLayout.Card, elements: EditorLayout.Elements, at: CGPoint,
                          status: NodeExecutionStatus?, anchor: InsertAnchor?) -> some View
    {
        let node = card.node
        let body = EditorLayout.layoutLoopBody(node, nodes, editor.edges)
        let members = node.bodyNodeIds.filter { id in nodes.contains { $0.id == id } }
        let after = members.last ?? node.id
        return LoopFrameView(
            node: node, width: card.width, bodyHeight: body.height, empty: members.isEmpty,
            selected: node.id == editor.selectedNodeId, status: status,
            anchorActive: anchor?.afterNodeId == after && anchor?.beforeNodeId == EditorAnchor.loopBody,
            onSelect: {
                editor.canvasFocused = true
                editor.nodeClick(node.id)
            },
            onAdd: {
                openLibrary(InsertAnchor(afterNodeId: after, beforeNodeId: EditorAnchor.loopBody, bodyOnly: true))
            })
            .onHover { h in hoveredCard = h ? node.id : (hoveredCard == node.id ? nil : hoveredCard) }
            .gesture(dragCard(card.id))
            .overlay(alignment: .top) { PortDot().offset(y: -3.5).allowsHitTesting(false) }
            .overlay(alignment: .top) {
                PortDot().offset(y: EditorLayout.loopHeader + 7 - 3.5)
                    .gesture(connectGesture(node.id, fromBody: true, origin: { sourcePoint(card, livePositions(currentElements())[card.id] ?? at, body: true) }))
            }
            .overlay(alignment: .bottom) {
                PortDot().offset(y: 3.5).gesture(connectGesture(node.id, fromBody: false, origin: { sourcePoint(card, livePositions(currentElements())[card.id] ?? at, body: false) }))
            }
            .offset(x: at.x, y: at.y)
            .zIndex(0)
    }

    private func currentElements() -> EditorLayout.Elements { EditorLayout.toCanvasElements(editor.nodes, editor.edges) }

    // MARK: Dragging cards

    private func dragCard(_ id: String) -> some Gesture {
        DragGesture(minimumDistance: 3, coordinateSpace: .named("canvas"))
            .onChanged { g in
                let elements = currentElements()
                let start = dragStart ?? Dictionary(
                    elements.cards.filter { $0.parentId == nil }.map { ($0.id, $0.absolute) }, uniquingKeysWith: { a, _ in a })
                dragStart = start
                guard let origin = start[id] else { return }
                let zoom = editor.viewport?.zoom ?? 1
                dragPositions[id] = CGPoint(
                    x: GraphOps.snapToLattice(origin.x + g.translation.width / zoom),
                    y: GraphOps.snapToLattice(origin.y + g.translation.height / zoom))
            }
            .onEnded { _ in
                guard let start = dragStart else { return }
                var positions = start
                for (k, p) in dragPositions { positions[k] = p }
                dragStart = nil
                editor.commitPositions(positions)
                dragPositions = [:]
            }
    }

    // MARK: Connecting

    private func connectGesture(_ source: String, fromBody: Bool, origin: @escaping () -> CGPoint) -> some Gesture {
        DragGesture(minimumDistance: 1, coordinateSpace: .named("canvas"))
            .onChanged { g in
                let v = editor.viewport ?? EditorLayout.Viewport(x: 0, y: 0, zoom: 1)
                let o = origin()
                let start = CGPoint(x: o.x * v.zoom + v.x, y: o.y * v.zoom + v.y)
                connecting = Connecting(source: source, fromBody: fromBody, start: start, pointer: g.location)
            }
            .onEnded { g in
                defer { connecting = nil }
                let elements = currentElements()
                let v = editor.viewport ?? EditorLayout.Viewport(x: 0, y: 0, zoom: 1)
                if let target = closestTarget(g.location, elements: elements, v: v) {
                    if EditorLayout.canConnect(nodes, editor.edges, source: source, target: target.id, fromBodyHandle: fromBody) {
                        editor.connect(source, target.id)
                    }
                    return
                }
                let flow = CGPoint(x: (g.location.x - v.x) / v.zoom, y: (g.location.y - v.y) / v.zoom)
                openLibraryAppend(source, elements: elements, position: flow, fromLoopBody: fromBody)
            }
    }

    /// The target port nearest the pointer within 60pt; add-step spots count, and refuse.
    private func closestTarget(_ p: CGPoint, elements: EditorLayout.Elements, v: EditorLayout.Viewport) -> (id: String, point: CGPoint)? {
        let pos = livePositions(elements)
        var ports: [(String, CGPoint)] = elements.cards.compactMap { card in
            guard card.node.type != .trigger, let at = pos[card.id] else { return nil }
            return (card.id, CGPoint(x: at.x + card.width / 2, y: at.y))
        }
        ports += elements.addSteps.map { ($0.id, CGPoint(x: $0.position.x + 11, y: $0.position.y)) }
        var best: (String, CGPoint)?
        var bestDistance: CGFloat = 60
        for (id, f) in ports {
            let s = CGPoint(x: f.x * v.zoom + v.x, y: f.y * v.zoom + v.y)
            let d = hypot(s.x - p.x, s.y - p.y)
            if d < bestDistance {
                bestDistance = d
                best = (id, s)
            }
        }
        return best.map { (id: $0.0, point: $0.1) }
    }

    private func connectionLine(_ c: Connecting, elements: EditorLayout.Elements, v: EditorLayout.Viewport) -> some View {
        let end = closestTarget(c.pointer, elements: elements, v: v)?.point ?? c.pointer
        return EdgeGeometry.path(c.start, end).path
            .stroke(Color(hex: 0xB1B1B7), lineWidth: 1)
            .allowsHitTesting(false)
    }

    // MARK: Library anchors

    private func openLibrary(_ anchor: InsertAnchor) {
        editor.canvasFocused = true
        editor.openLibrary(anchor)
    }

    private func replace(_ node: WorkflowNode, inBody: Bool) {
        openLibrary(node.type == .trigger ? .trigger
            : InsertAnchor(afterNodeId: node.id, beforeNodeId: nil, bodyOnly: inBody, replaceNodeId: node.id))
    }

    /// openLibraryAppend: inside a loop the new step joins its body; elsewhere it lands where the edge was dropped.
    private func openLibraryAppend(_ after: String, elements: EditorLayout.Elements, position: CGPoint? = nil, fromLoopBody: Bool = false) {
        if fromLoopBody || GraphOps.loopBodyOwners(nodes)[after] != nil {
            openLibrary(InsertAnchor(afterNodeId: after, beforeNodeId: EditorAnchor.loopBody, bodyOnly: true))
            return
        }
        openLibrary(InsertAnchor(
            afterNodeId: after, beforeNodeId: nil, insideBranch: elements.branchMembers.contains(after), position: position))
    }

    // MARK: Edge hover

    private func hoverEdges(at p: CGPoint, elements: EditorLayout.Elements, v: EditorLayout.Viewport) {
        let edges = drawnEdges(elements, livePositions(elements)).filter { $0.data != nil }
        let screen = { (q: CGPoint) in CGPoint(x: q.x * v.zoom + v.x, y: q.y * v.zoom + v.y) }
        let hit = edges.first { EdgeGeometry.distance(p, toPolyline: $0.samples.map(screen)) <= 20 * v.zoom }
        if let hit { enterEdge(hit.id) } else if hoveredEdge != nil { leaveEdge() }
    }

    private func enterEdge(_ id: String) {
        edgeLeave?.cancel()
        edgeLeave = nil
        if hoveredEdge != id { hoveredEdge = id }
    }

    private func leaveEdge() {
        guard edgeLeave == nil, hoveredEdge != nil else { return }
        edgeLeave = Task {
            try? await Task.sleep(for: .milliseconds(600))
            guard !Task.isCancelled else { return }
            hoveredEdge = nil
            edgeLeave = nil
        }
    }

    // MARK: Viewport

    /// The canvas opens once per workflow: where it was left, or at 100% on its first step.
    private func openView(_ elements: EditorLayout.Elements, width: CGFloat) {
        guard editor.viewport == nil, width > 0 else { return }
        let view = editor.rememberedView() ?? EditorLayout.openingViewport(elements, width: width)
        openedView = view
        editor.viewport = view
    }

    private func moveEnded() {
        guard let view = editor.viewport else { return }
        if let opened = openedView, opened == view { return }
        openedView = nil
        editor.rememberView(view)
    }

    private func pan(dx: CGFloat, dy: CGFloat) {
        guard var v = editor.viewport else { return }
        v.x += dx
        v.y += dy
        editor.viewport = v
        settleSoon()
    }

    private func settleSoon() {
        scrollSettle?.cancel()
        scrollSettle = Task {
            try? await Task.sleep(for: .milliseconds(300))
            guard !Task.isCancelled else { return }
            moveEnded()
        }
    }

    /// Scales about a screen point, clamped to the canvas zoom range.
    private func zoom(by factor: CGFloat, around p: CGPoint, animated: Bool = false) {
        guard let v = editor.viewport else { return }
        let z = min(EditorLayout.maxZoom, max(EditorLayout.minZoom, v.zoom * factor))
        let next = EditorLayout.Viewport(x: p.x - (p.x - v.x) * z / v.zoom, y: p.y - (p.y - v.y) * z / v.zoom, zoom: z)
        if animated {
            withAnimation(.easeOut(duration: 0.2)) { editor.viewport = next }
        } else {
            editor.viewport = next
        }
        settleSoon()
    }

    private func zoomTo(_ z: CGFloat) {
        guard let v = editor.viewport else { return }
        zoom(by: z / v.zoom, around: CGPoint(x: editor.canvasSize.width / 2, y: editor.canvasSize.height / 2), animated: true)
    }

    private func fitFromTop(_ elements: EditorLayout.Elements) {
        let pos = livePositions(elements)
        let rects = elements.cards.filter { $0.parentId == nil }.compactMap { c in
            pos[c.id].map { CGRect(x: $0.x, y: $0.y, width: c.width, height: drawnHeight(c)) }
        } + elements.addSteps.map { CGRect(origin: $0.position, size: CGSize(width: 22, height: 22)) }
            + (elements.addTrigger.map { [CGRect(origin: $0, size: CGSize(width: GraphOps.cardWidth, height: 58))] } ?? [])
        guard let first = rects.first else { return }
        let bounds = rects.dropFirst().reduce(first) { $0.union($1) }
        let view = EditorLayout.topAlignedFit(bounds, width: editor.canvasSize.width, height: editor.canvasSize.height)
        withAnimation(.easeOut(duration: 0.2)) { editor.viewport = view }
        settleSoon()
    }

    // MARK: Controls

    private func controls(_ elements: EditorLayout.Elements, size: CGSize) -> some View {
        let zoom = editor.viewport?.zoom ?? 1
        let center = CGPoint(x: size.width / 2, y: size.height / 2)
        return VStack(spacing: 0) {
            controlButton(.plus, "Zoom In", heavy: true, disabled: zoom >= EditorLayout.maxZoom) { self.zoom(by: 1.2, around: center, animated: true) }
            Rectangle().fill(Theme.white(0.08)).frame(height: 1)
            controlButton(.minus, "Zoom Out", heavy: true, disabled: zoom <= EditorLayout.minZoom) { self.zoom(by: 1 / 1.2, around: center, animated: true) }
            Rectangle().fill(Theme.white(0.08)).frame(height: 1)
            controlButton(.maximize, "Fit the workflow") { fitFromTop(elements) }
            Rectangle().fill(Theme.white(0.08)).frame(height: 1)
            controlButton(.alignVerticalSpaceAround, "Tidy up") { editor.tidyUp() }
        }
        .frame(width: 26)
        .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.12), lineWidth: 1))
        .clipShape(RoundedRectangle(cornerRadius: Theme.radiusMd))
    }

    private func controlButton(_ glyph: LucideGlyph, _ help: String, heavy: Bool = false, disabled: Bool = false,
                               action: @escaping () -> Void) -> some View {
        Button(action: action) {
            LucideIcon(glyph, size: heavy ? 14 : 12, strokeWidth: heavy ? 2.25 : 2)
                .foregroundStyle(heavy ? Theme.gray200 : Theme.gray400)
                .frame(width: 26, height: 26)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(disabled)
        .opacity(disabled ? 0.4 : 1)
        .help(help)
        .accessibilityLabel(help)
    }

    // MARK: Events

    private func bindEvents(_ elements: EditorLayout.Elements) {
        guard !staticLayout else { return }
        catcher.onScroll = { dx, dy in pan(dx: dx * 0.5, dy: dy * 0.5) }
        catcher.onMagnify = { m, p in zoom(by: 1 + m, around: p) }
        catcher.onMouseDown = { inside in editor.canvasFocused = inside }
        catcher.onKey = { event in handleKey(event) }
    }

    /// The canvas's keys, and undo anywhere outside a text field.
    private func handleKey(_ event: NSEvent) -> Bool {
        if NSApp.keyWindow?.firstResponder is NSText { return false }
        let key = event.charactersIgnoringModifiers?.lowercased() ?? ""
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        if flags.contains(.command) || flags.contains(.control) {
            guard key == "z" else { return false }
            if flags.contains(.shift) { editor.redo() } else { editor.undo() }
            return true
        }
        guard editor.canvasFocused else { return false }
        let elements = currentElements()
        let center = CGPoint(x: editor.canvasSize.width / 2, y: editor.canvasSize.height / 2)
        switch event.keyCode {
        case 48:
            let outgoing = Set(editor.edges.map(\.source))
            let body = GraphOps.loopBodyOwners(nodes)
            guard let leaf = nodes.reversed().first(where: { !outgoing.contains($0.id) && body[$0.id] == nil }) else { return false }
            openLibraryAppend(leaf.id, elements: elements)
            return true
        case 51, 117:
            guard let id = editor.selectedNodeId, nodes.contains(where: { $0.id == id }) else { return false }
            editor.deleteNode(id)
            return true
        default: break
        }
        switch event.characters ?? "" {
        case "+", "=": zoom(by: 1.2, around: center, animated: true)
        case "-": zoom(by: 1 / 1.2, around: center, animated: true)
        case "0": zoomTo(1)
        case "1": fitFromTop(elements)
        default: return false
        }
        return true
    }
}

/// The scroll, pinch, click and key events the canvas takes from AppKit, inside its own frame.
struct CanvasEventCatcher: NSViewRepresentable {
    @MainActor final class Box {
        var onScroll: (CGFloat, CGFloat) -> Void = { _, _ in }
        var onMagnify: (CGFloat, CGPoint) -> Void = { _, _ in }
        var onMouseDown: (Bool) -> Void = { _ in }
        var onKey: (NSEvent) -> Bool = { _ in false }
    }

    let box: Box

    final class Catcher: NSView {
        var box: Box?
        var monitors: [Any] = []
        override var isFlipped: Bool { true }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            monitors.forEach(NSEvent.removeMonitor)
            monitors = []
            guard window != nil else { return }
            if let m = NSEvent.addLocalMonitorForEvents(matching: [.scrollWheel, .magnify, .leftMouseDown, .keyDown], handler: { [weak self] e in
                self?.handle(e) ?? e
            }) { monitors.append(m) }
        }

        @MainActor private func handle(_ e: NSEvent) -> NSEvent? {
            guard let box, let window, e.window === window else { return e }
            let p = convert(e.locationInWindow, from: nil)
            let inside = bounds.contains(p)
            switch e.type {
            case .scrollWheel where inside:
                box.onScroll(e.scrollingDeltaX * (e.hasPreciseScrollingDeltas ? 1 : 20), e.scrollingDeltaY * (e.hasPreciseScrollingDeltas ? 1 : 20))
                return nil
            case .magnify where inside:
                box.onMagnify(e.magnification, p)
                return nil
            case .leftMouseDown:
                box.onMouseDown(inside)
                return e
            case .keyDown:
                return box.onKey(e) ? nil : e
            default:
                return e
            }
        }

        deinit { monitors.forEach(NSEvent.removeMonitor) }
    }

    func makeNSView(context: Context) -> Catcher {
        let v = Catcher()
        v.box = box
        return v
    }

    func updateNSView(_ v: Catcher, context: Context) { v.box = box }
}

/// React Flow's MiniMap: every card as a soft block, the visible area cut out of a dark mask.
struct CanvasMinimap: View {
    let elements: EditorLayout.Elements
    let positions: [String: CGPoint]
    let viewport: EditorLayout.Viewport
    let size: CGSize
    let onPan: (CGFloat, CGFloat) -> Void
    @State private var last: CGPoint?

    static let width: CGFloat = 96
    static let height: CGFloat = 64

    private var rects: [CGRect] {
        var out: [CGRect] = elements.cards.compactMap { c in
            positions[c.id].map {
                CGRect(x: $0.x, y: $0.y, width: c.width, height: c.isLoop ? c.height : CardText.drawnHeight(c.node))
            }
        }
        out += elements.addSteps.map { CGRect(origin: $0.position, size: CGSize(width: 22, height: 22)) }
        if let t = elements.addTrigger { out.append(CGRect(origin: t, size: CGSize(width: GraphOps.cardWidth, height: 58))) }
        return out
    }

    private var geometry: (box: CGRect, view: CGRect, scale: CGFloat) {
        let view = CGRect(x: -viewport.x / viewport.zoom, y: -viewport.y / viewport.zoom,
                          width: size.width / viewport.zoom, height: size.height / viewport.zoom)
        let bounds = rects.reduce(view) { $0.union($1) }
        let viewScale = max(bounds.width / Self.width, bounds.height / Self.height)
        let w = viewScale * Self.width, h = viewScale * Self.height
        let offset = 5 * viewScale
        let box = CGRect(x: bounds.minX - (w - bounds.width) / 2 - offset, y: bounds.minY - (h - bounds.height) / 2 - offset,
                         width: w + 2 * offset, height: h + 2 * offset)
        return (box, view, viewScale)
    }

    var body: some View {
        let g = geometry
        Canvas { ctx, size in
            let s = min(size.width / g.box.width, size.height / g.box.height)
            let ox = (size.width - g.box.width * s) / 2, oy = (size.height - g.box.height * s) / 2
            func map(_ r: CGRect) -> CGRect {
                CGRect(x: (r.minX - g.box.minX) * s + ox, y: (r.minY - g.box.minY) * s + oy, width: r.width * s, height: r.height * s)
            }
            for r in rects {
                ctx.fill(Path(roundedRect: map(r), cornerRadius: 5 * s), with: .color(Theme.white(0.25)))
            }
            var mask = Path(CGRect(origin: .zero, size: size))
            mask.addRect(map(g.view))
            ctx.fill(mask, with: .color(Color.black.opacity(0.55)), style: FillStyle(eoFill: true))
        }
        .frame(width: Self.width, height: Self.height)
        .background(Theme.surfacePanel)
        .clipShape(RoundedRectangle(cornerRadius: Theme.radius))
        .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.12), lineWidth: 1))
        .gesture(
            DragGesture(minimumDistance: 0)
                .onChanged { d in
                    if let last {
                        let k = g.scale * max(1, viewport.zoom)
                        onPan(-(d.location.x - last.x) * k, -(d.location.y - last.y) * k)
                    }
                    last = d.location
                }
                .onEnded { _ in last = nil })
    }
}
