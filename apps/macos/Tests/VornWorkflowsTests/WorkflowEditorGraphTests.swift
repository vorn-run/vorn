import CoreGraphics
import Foundation
import Testing
@testable import VornCore
@testable import VornWorkflows

private func wn(_ id: String, _ type: WorkflowNodeType = .script, _ label: String? = nil,
                _ config: JSONValue = [:], x: Double = 0, y: Double = 0) -> WorkflowNode
{
    WorkflowNode(id: id, type: type, label: label ?? id, config: config, position: .init(x: x, y: y))
}

private func we(_ id: String, _ s: String, _ t: String, _ branch: String? = nil) -> WorkflowEdge {
    WorkflowEdge(id: id, source: s, target: t, conditionBranch: branch)
}

private func g(_ nodes: [WorkflowNode], _ edges: [WorkflowEdge]) -> WorkflowGraph { WorkflowGraph(nodes: nodes, edges: edges) }

private func bodyOf(_ nodes: [WorkflowNode], _ id: String = "loop") -> [String] {
    nodes.first { $0.id == id }?.bodyNodeIds ?? []
}

private let trigger = wn("trigger-1", .trigger, "Manual", ["triggerType": "manual"])

private let chain = g(
    [wn("t", .trigger, "Manual", ["triggerType": "manual"]), wn("a", .script, "One"), wn("b", .script, "Two")],
    [we("e1", "t", "a"), we("e2", "a", "b")])

private let fork = g(
    [wn("t", .trigger, "Manual", ["triggerType": "manual"]),
     wn("c", .condition, "Ready?", ["variable": "x", "operator": "equals", "value": "1"]),
     wn("yes"), wn("no"), wn("join")],
    [we("e1", "t", "c"), we("e2", "c", "yes", "true"), we("e3", "c", "no", "false"),
     we("e4", "yes", "join"), we("e5", "no", "join")])

private let looped = g(
    [wn("t", .trigger, "Manual", ["triggerType": "manual"]),
     wn("loop", .loop, "Repeat", ["nodeType": "loop", "bodyNodeIds": ["w", "r"], "maxIterations": 2]),
     wn("w"), wn("r"), wn("after")],
    [we("e1", "t", "loop"), we("e2", "loop", "w"), we("e3", "w", "r"), we("e4", "r", "after")])

private let bodyLoop = g(
    [wn("t", .trigger), wn("loop", .loop, nil, ["nodeType": "loop", "bodyNodeIds": ["a", "b"], "maxIterations": 1]),
     wn("a"), wn("b"), wn("after")],
    [we("e1", "t", "loop"), we("e2", "loop", "a"), we("e3", "a", "b"), we("e4", "b", "after")])

// MARK: workflow-helpers.test.ts

@Suite struct EditorGraphOpsTests {
    @Test func factoriesSlugAndMerge() {
        let approval = GraphOps.createApproval(["message": "please", "timeoutMs": 30000])
        #expect(approval.config.str("message") == "please")
        #expect(approval.config.num("timeoutMs") == 30000)
        #expect(approval.slug == "approval_gate")
        #expect(GraphOps.createTrigger().slug == nil)
        #expect(GraphOps.createTrigger().label == "Manual Trigger")
        #expect(GraphOps.createLoop().bodyNodeIds.isEmpty)
    }

    @Test func autoLayoutPitchAndOrphans() {
        let a = wn("a")
        let laid = GraphOps.autoLayoutNodes([trigger, a, wn("orphan")], [we("e", "trigger-1", "a")])
        #expect(laid.map(\.id) == ["trigger-1", "a", "orphan"])
        #expect(laid[0].position.y == 0)
        #expect(laid[1].position.y == 144)
        #expect(GraphOps.autoLayoutNodes([], []).isEmpty)
    }

    @Test func appendNodeLinksTheLastLeaf() {
        let r = GraphOps.appendNode(g([trigger, wn("a")], [we("e", "trigger-1", "a")]), wn("b"))
        #expect(r.nodes.count == 3)
        #expect(r.edges.contains { $0.source == "a" && $0.target == "b" })
    }

    @Test func removeReconnectsAroundTheStep() {
        let r = GraphOps.removeNode(g([trigger, wn("a"), wn("b")], [we("1", "trigger-1", "a"), we("2", "a", "b")]), "a")
        #expect(r.nodes.map(\.id) == ["trigger-1", "b"])
        #expect(r.edges.contains { $0.source == "trigger-1" && $0.target == "b" })
    }

    @Test func removingAConditionCascadesToTheJoin() {
        let inserted = GraphOps.insertConditionBetween(
            g([trigger, wn("tail")], [we("1", "trigger-1", "tail")]), after: "trigger-1", before: "tail")
        let cond = inserted.nodes.first { $0.type == .condition }!
        let r = GraphOps.removeNode(inserted, cond.id)
        #expect(r.nodes.map(\.id).sorted() == ["tail", "trigger-1"])
        #expect(r.edges.contains { $0.source == "trigger-1" && $0.target == "tail" })
    }

    @Test func removingATerminalConditionLeavesNoEdges() {
        let inserted = GraphOps.insertConditionBetween(g([trigger], []), after: "trigger-1", before: nil)
        let cond = inserted.nodes.first { $0.type == .condition }!
        let r = GraphOps.removeNode(inserted, cond.id)
        #expect(r.nodes.map(\.id) == ["trigger-1"])
        #expect(r.edges.isEmpty)
    }

    @Test func insertBetweenSplitsTheEdge() {
        let r = GraphOps.insertNodeBetween(g([trigger, wn("b")], [we("e1", "trigger-1", "b")]), edgeId: "e1", wn("mid"))
        #expect(r.nodes.count == 3)
        #expect(r.edges.contains { $0.source == "trigger-1" && $0.target == "mid" })
        #expect(r.edges.contains { $0.source == "mid" && $0.target == "b" })
        #expect(!r.edges.contains { $0.id == "e1" })
        let none = GraphOps.insertNodeBetween(g([trigger], []), edgeId: "missing", wn("x"))
        #expect(none.nodes == [trigger])
    }

    @Test func insertBetweenKeepsTheBranchOnTheFirstHalf() {
        let r = GraphOps.insertNodeBetween(fork, edgeId: "e2", wn("mid"))
        #expect(r.edges.first { $0.target == "mid" }?.conditionBranch == "true")
        #expect(r.edges.first { $0.source == "mid" }?.conditionBranch == nil)
    }

    @Test func conditionAtTheEndAndWithBranches() {
        let tail = GraphOps.insertConditionBetween(g([trigger, wn("a")], [we("1", "trigger-1", "a")]), after: "a", before: nil)
        let cond = tail.nodes.first { $0.type == .condition }!
        #expect(tail.edges.contains { $0.source == "a" && $0.target == cond.id })
        #expect(tail.nodes.count == 3)

        let mid = GraphOps.insertConditionBetween(
            g([trigger, wn("a"), wn("b")], [we("1", "trigger-1", "a"), we("2", "a", "b")]), after: "a", before: "b")
        let c = mid.nodes.filter { $0.type == .condition }
        #expect(c.count == 1)
        #expect(mid.edges.contains { $0.source == c[0].id && $0.conditionBranch == "true" })
        #expect(mid.edges.contains { $0.source == c[0].id && $0.conditionBranch == "false" })
        #expect(!mid.edges.contains { $0.source == "a" && $0.target == "b" })
        #expect(mid.nodes.filter { $0.label == "True Branch" }.first?.slug == "true_branch")
    }

    @Test func parallelBranchFromAFork() {
        let r = GraphOps.addParallelBranch(g([trigger, wn("a")], [we("1", "trigger-1", "a")]), forkFrom: "trigger-1", wn("p"))
        #expect(r.nodes.count == 3)
        #expect(r.edges.contains { $0.source == "trigger-1" && $0.target == "p" })
        let joined = GraphOps.addParallelBranch(chain, forkFrom: "t", wn("p"))
        #expect(joined.edges.contains { $0.source == "p" && $0.target == "b" })
    }

    @Test func insertBeforeForkTakesTheBranches() {
        let r = GraphOps.insertBeforeFork(fork, forkNodeId: "c", wn("pre"))
        #expect(r.edges.contains { $0.source == "c" && $0.target == "pre" })
        #expect(Set(r.edges.filter { $0.source == "pre" }.map(\.target)) == ["yes", "no"])
    }

    @Test func flowLayoutRowsForkAndJoin() {
        #expect(GraphOps.computeFlowLayout([], []).isEmpty)
        let rows = GraphOps.computeFlowLayout(fork.nodes, fork.edges)
        #expect(rows.count == 4)
        guard case .fork(let forkId, let branches, let join) = rows[2] else {
            Issue.record("expected a fork row")
            return
        }
        #expect(forkId == "c")
        #expect(branches.count == 2)
        #expect(join == "join")
        if case .node(let last) = rows[3] { #expect(last.id == "join") }
    }

    @Test func flowLayoutStopsOnCycles() {
        let rows = GraphOps.computeFlowLayout(chain.nodes, chain.edges + [we("back", "b", "t")])
        #expect(rows.count == 3)
    }

    @Test func switchTriggerDefaults() {
        #expect(GraphOps.switchTriggerType("recurring").str("cron") == "0 9 * * *")
        #expect(GraphOps.switchTriggerType("connectorPoll").str("cron") == "*/5 * * * *")
        #expect(GraphOps.switchTriggerType("webhook").str("method") == "POST")
        #expect(!(GraphOps.switchTriggerType("webhook").str("token") ?? "").isEmpty)
        #expect(GraphOps.switchTriggerType("manual") == ["triggerType": "manual"])
    }

    @Test func placementHangsNewStepsUnderTheirParent() {
        let arranged = g([wn("t", .trigger, x: 40, y: 0), wn("a", x: 40, y: 144)], [we("1", "t", "a")])
        let r = GraphOps.appendNodeAfter(arranged, after: "a", wn("n"))
        #expect(r.nodes.last?.position == WorkflowNodePosition(x: 40, y: 288))
        let mid = GraphOps.insertNodeBetween(arranged, edgeId: "1", wn("m"))
        #expect(mid.nodes.first { $0.id == "m" }?.position == WorkflowNodePosition(x: 40, y: 144))
        #expect(mid.nodes.first { $0.id == "a" }?.position.y == 288)
    }

    @Test func snapRoundsLikeMathRound() {
        #expect(GraphOps.snapToLattice(300) == 304)
        #expect(GraphOps.snapToLattice(-140) == -136)
        #expect(GraphOps.snapToLattice(-2).sign == .plus)
    }
}

// MARK: loop-body-helpers.test.ts

@Suite struct EditorLoopBodyTests {
    @Test func insertBetweenBodySteps() {
        let inserted = GraphOps.insertNodeBetween(bodyLoop, edgeId: "e3", wn("new"))
        let adopted = GraphOps.adoptIntoLoopBody(before: bodyLoop.nodes, inserted, afterNodeId: "a", beforeNodeId: "b")
        #expect(bodyOf(adopted.nodes) == ["a", "b", "new"])
    }

    @Test func insertOnTheEntryEdge() {
        let inserted = GraphOps.insertNodeBetween(bodyLoop, edgeId: "e2", wn("new"))
        let adopted = GraphOps.adoptIntoLoopBody(before: bodyLoop.nodes, inserted, afterNodeId: "loop", beforeNodeId: "a")
        #expect(bodyOf(adopted.nodes).contains("new"))
    }

    @Test func insertOnTheExitEdgeStaysOutside() {
        let inserted = GraphOps.insertNodeBetween(bodyLoop, edgeId: "e4", wn("new"))
        let adopted = GraphOps.adoptIntoLoopBody(before: bodyLoop.nodes, inserted, afterNodeId: "b", beforeNodeId: "after")
        #expect(bodyOf(adopted.nodes) == ["a", "b"])
    }

    @Test func conditionInsideTheBodyTakesBothBranches() {
        let inserted = GraphOps.insertConditionBetween(bodyLoop, after: "a", before: "b")
        let adopted = GraphOps.adoptIntoLoopBody(before: bodyLoop.nodes, inserted, afterNodeId: "a", beforeNodeId: "b")
        #expect(bodyOf(adopted.nodes).count == 5)
        let cond = adopted.nodes.first { $0.type == .condition }!
        #expect(bodyOf(GraphOps.removeNode(adopted, cond.id).nodes) == ["a", "b"])
    }

    @Test func appendAfterABodyStep() {
        let r = GraphOps.appendToLoopBody(bodyLoop, loopId: "loop", wn("new"), after: "a")
        #expect(bodyOf(r.nodes) == ["a", "b", "new"])
        #expect(r.edges.contains { $0.source == "a" && $0.target == "new" })
        #expect(r.edges.contains { $0.source == "new" && $0.target == "b" })
        #expect(!r.edges.contains { $0.source == "a" && $0.target == "b" })
    }

    @Test func removingABodyStepLeavesTheBody() {
        #expect(bodyOf(GraphOps.removeNode(bodyLoop, "a").nodes) == ["b"])
    }

    @Test func collapseRedrawsExitsFromTheLoop() {
        let collapsed = GraphOps.collapseLoopBodies(looped.nodes, looped.edges)
        #expect(collapsed.map(\.id) == ["e1", "e4:via-loop"])
        #expect(collapsed.last?.source == "loop")
    }

    @Test func bodyGraphChainsWithoutInnerEdges() {
        let loop = bodyLoop.nodes[1]
        let graph = GraphOps.loopBodyGraph(bodyLoop.nodes, [], loop: loop)
        #expect(graph.edges.map(\.id) == ["a->b:chain"])
        #expect(graph.entries == ["a"])
    }
}

// MARK: workflow-canvas-layout.test.ts

@Suite struct EditorLayoutTests {
    @Test func chainTopToBottom() {
        let p = EditorLayout.layoutPositions(chain.nodes, chain.edges).positions
        #expect(p["t"]!.y < p["a"]!.y)
        #expect(p["a"]!.y < p["b"]!.y)
        #expect(p["t"]!.x == p["a"]!.x)
        for v in p.values {
            #expect(v.x.truncatingRemainder(dividingBy: 8) == 0)
            #expect(v.y.truncatingRemainder(dividingBy: 8) == 0)
        }
    }

    @Test func forkSideBySideAndEven() {
        let placed = EditorLayout.layoutPositions(fork.nodes, fork.edges)
        let p = placed.positions
        #expect(p["yes"]!.x != p["no"]!.x)
        #expect(p["yes"]!.y == p["no"]!.y)
        #expect(p["join"]!.x == p["c"]!.x)
        #expect(placed.branchMembers.contains("yes") && placed.branchMembers.contains("no"))
        #expect(!placed.branchMembers.contains("join"))
        let parent = p["c"]!.x + 140
        #expect(parent - (p["yes"]!.x + 140) == (p["no"]!.x + 140) - parent)
    }

    @Test func loopBodyDrawnAsChildren() {
        let el = EditorLayout.toCanvasElements(looped.nodes, looped.edges)
        let ids = el.cards.map(\.id)
        #expect(ids.firstIndex(of: "loop")! < ids.firstIndex(of: "w")!)
        let w = el.card("w")!
        let r = el.card("r")!
        #expect(w.parentId == "loop" && r.parentId == "loop")
        #expect(w.position.y > 0)
        #expect(r.position.y > w.position.y)
        let entry = el.edges.first { $0.id == "e2" }!
        #expect(entry.source == "loop" && entry.fromBodyHandle && entry.target == "w")
        #expect(entry.data?.bodyOf == "loop")
        #expect(el.edges.first { $0.id == "e3" }?.data?.bodyOf == "loop")
        let exit = el.edges.first { $0.id == "e4" }!
        #expect(exit.source == "loop")
        #expect(exit.data?.afterNodeId == "r" && exit.data?.beforeNodeId == "after")
    }

    @Test func branchingBodyWidensTheLoop() {
        let nodes = [
            wn("t", .trigger, "Manual", ["triggerType": "manual"]),
            wn("loop", .loop, "Each", ["nodeType": "loop", "mode": "forEach", "bodyNodeIds": ["c", "y", "n"]]),
            wn("c", .condition, "Keep?", ["variable": "{{loop.item.keep}}", "operator": "equals", "value": "yes"]),
            wn("y"), wn("n"),
        ]
        let edges = [we("e1", "t", "loop"), we("e2", "loop", "c"), we("e3", "c", "y", "true"), we("e4", "c", "n", "false")]
        let el = EditorLayout.toCanvasElements(nodes, edges)
        #expect(el.card("loop")!.width > 560)
        #expect(el.card("y")!.position.y == el.card("n")!.position.y)
        #expect(el.card("y")!.position.x != el.card("n")!.position.x)
        #expect(el.edges.first { $0.id == "e3" }?.label == "True")
        #expect(el.addSteps.contains { $0.id == "add:loop" })
    }

    @Test func leavesTrailAnAddButton() {
        #expect(EditorLayout.toCanvasElements(chain.nodes, chain.edges).addSteps.map(\.id) == ["add:b"])
        let terminal = g(looped.nodes.filter { $0.id != "after" }, looped.edges.filter { $0.id != "e4" })
        #expect(EditorLayout.toCanvasElements(terminal.nodes, terminal.edges).addSteps.contains { $0.id == "add:loop" })
        let yes = EditorLayout.toCanvasElements(fork.nodes, fork.edges).edges.first { $0.id == "e2" }!
        #expect(yes.label == "True" && yes.data?.conditionBranch == "true" && yes.data?.insideBranch == true)
    }

    @Test func storedPositionsOnTheLattice() {
        var arranged = chain.nodes
        arranged[1].position = .init(x: 120, y: 300)
        #expect(!GraphOps.positionsAreSeed(arranged))
        #expect(EditorLayout.toCanvasElements(arranged, chain.edges).card("a")!.position == CGPoint(x: 120, y: 304))
    }

    @Test func emptyLoopAndHalfFork() {
        let empty = [wn("t", .trigger), wn("loop", .loop, nil, ["nodeType": "loop", "bodyNodeIds": [], "maxIterations": 2])]
        #expect(EditorLayout.toCanvasElements(empty, [we("e1", "t", "loop")]).card("loop")!.height > 120)
        let half = fork.nodes.filter { $0.id != "no" && $0.id != "join" }
        let halfEdges = fork.edges.filter { ["e1", "e2", "e3"].contains($0.id) }
        #expect(EditorLayout.layoutPositions(half, halfEdges).positions["yes"] != nil)
    }

    @Test func healsPreLatticePositions() {
        var stored = Array(chain.nodes.prefix(2))
        stored[0].position = .init(x: -140, y: 0)
        stored[1].position = .init(x: -140, y: 58)
        let el = EditorLayout.toCanvasElements(stored, [we("e1", "t", "a")])
        #expect(el.card("t")!.position == CGPoint(x: -136, y: 0))
        #expect(el.card("a")!.position == CGPoint(x: -136, y: 56))
        #expect(EditorLayout.alignedNodes(stored).map(\.position) == [.init(x: -136, y: 0), .init(x: -136, y: 56)])
        let aligned = EditorLayout.alignedNodes(stored)
        #expect(EditorLayout.alignedNodes(aligned) == aligned)
        #expect(EditorLayout.alignedNodes(chain.nodes) == chain.nodes)
    }

    @Test func cardUnderALoopSharesItsCentre() {
        let nodes = [wn("loop", .loop, nil, ["nodeType": "loop", "bodyNodeIds": [], "maxIterations": 2]), wn("after")]
        let p = EditorLayout.layoutPositions(nodes, [we("e1", "loop", "after")]).positions
        #expect(abs((p["loop"]!.x + 156) - (p["after"]!.x + 140)) <= 4)
    }

    @Test func addTriggerPlaceholderAboveTheTopCard() {
        let el = EditorLayout.toCanvasElements([wn("a")], [])
        #expect(el.addTrigger == CGPoint(x: 0, y: -114))
        #expect(EditorLayout.toCanvasElements([], []).addTrigger == .zero)
    }

    @Test func connectRules() {
        #expect(!EditorLayout.canConnect(chain.nodes, chain.edges, source: "b", target: "a"))
        #expect(!EditorLayout.canConnect(chain.nodes, chain.edges, source: "a", target: "t"))
        #expect(!EditorLayout.canConnect(chain.nodes, chain.edges, source: "a", target: "b"))
        #expect(EditorLayout.canConnect(chain.nodes, [we("e1", "t", "a")], source: "t", target: "b"))
        #expect(!EditorLayout.canConnect(looped.nodes, looped.edges, source: "t", target: "r"))
    }

    @Test func viewportFitsAtTheTop() {
        let fit = EditorLayout.topAlignedFit(CGRect(x: 0, y: 0, width: 280, height: 2000), width: 800, height: 600)
        #expect(fit.zoom == 0.2)
        let open = EditorLayout.openingViewport(EditorLayout.toCanvasElements(chain.nodes, chain.edges), width: 800)
        #expect(open.zoom == 1)
        #expect(open.y == 48)
        #expect(open.x == 400)
    }
}

// MARK: use-definition-history.ts, library picks, requirements

@Suite struct EditorHistoryTests {
    @Test func absorbsTheLoadThenUndoesAndRedoes() {
        let start = Date(timeIntervalSince1970: 0)
        var h = DefinitionHistory(chain)
        var migrated = chain
        migrated.nodes[1].slug = "one"
        h.record(migrated, now: start)
        #expect(!h.canUndo)
        let one = GraphOps.appendNodeAfter(migrated, after: "b", wn("c"))
        h.record(one, now: start.addingTimeInterval(1))
        let two = GraphOps.appendNodeAfter(one, after: "c", wn("d"))
        h.record(two, now: start.addingTimeInterval(2))
        #expect(h.undo() == one)
        #expect(h.undo() == migrated)
        #expect(h.undo() == nil)
        #expect(h.redo() == one)
        #expect(h.redo() == two)
        #expect(h.redo() == nil)
    }

    @Test func mergesBurstsAndClearsTheFuture() {
        let start = Date(timeIntervalSince1970: 0)
        var h = DefinitionHistory(chain, absorbing: false)
        var a = chain
        a.nodes[1].label = "O"
        h.record(a, now: start)
        var b = a
        b.nodes[1].label = "On"
        h.record(b, now: start.addingTimeInterval(0.2))
        #expect(h.undo() == chain)
        #expect(h.canRedo)
        h.record(a, now: start.addingTimeInterval(5))
        #expect(!h.canRedo)
    }

    @Test func capsTheStack() {
        var h = DefinitionHistory(chain, absorbing: false)
        var current = chain
        for i in 0..<120 {
            current.nodes[1].label = "L\(i)"
            h.record(current, now: Date(timeIntervalSince1970: Double(i)))
        }
        #expect(h.past.count == DefinitionHistory.stackLimit)
    }
}

@Suite struct EditorPickTests {
    @Test func picksBecomeSteps() {
        #expect(LibraryPick.httpProfile(profileConnectionId: "p", profileName: "Api").step?.type == .httpRequest)
        #expect(LibraryPick.catalogAction(connectorId: "gh", action: "x", actionLabel: "X").step?.config?["connectionId"] == "")
        #expect(LibraryPick.parallel.step == nil)
    }

    @Test func insertingFromTheLibrarySelectsTheNewStep() throws {
        let anchor = InsertAnchor(afterNodeId: "a", beforeNodeId: "b")
        let r = try #require(EditorOps.applyPick(.type(.script), at: anchor, to: chain))
        let added = r.graph.nodes.first { $0.id == r.selected }!
        #expect(added.type == .script)
        #expect(r.graph.edges.contains { $0.source == "a" && $0.target == added.id })
        #expect(r.graph.edges.contains { $0.source == added.id && $0.target == "b" })
    }

    @Test func slugsStayUnique() throws {
        var g1 = chain
        g1 = try #require(EditorOps.applyPick(.type(.script), at: .init(afterNodeId: "b"), to: g1)).graph
        g1 = try #require(EditorOps.applyPick(.type(.script), at: .init(afterNodeId: "b"), to: g1)).graph
        let slugs = g1.nodes.compactMap(\.slug)
        #expect(Set(slugs).count == slugs.count)
        #expect(slugs.contains("execute_script_2"))
    }

    @Test func triggerPickReplacesInPlaceOrReconnects() throws {
        let r = try #require(EditorOps.applyPick(.triggerType("recurring"), at: .trigger, to: chain))
        #expect(r.selected == "t")
        #expect(r.graph.nodes[0].label == "Schedule (Recurring)")
        #expect(EditorOps.applyPick(.triggerType("manual"), at: .trigger, to: chain) == nil)
        let headless = g(Array(chain.nodes.dropFirst()), [we("e2", "a", "b")])
        let added = try #require(EditorOps.applyPick(.triggerType("manual"), at: .trigger, to: headless))
        #expect(added.graph.edges.contains { $0.source == added.selected && $0.target == "a" })
    }

    @Test func replaceKeepsIdSlugAndEdges() throws {
        var anchor = InsertAnchor(afterNodeId: "a")
        anchor.replaceNodeId = "a"
        var base = chain
        base.nodes[1].slug = "one"
        let r = try #require(EditorOps.applyPick(.type(.approval), at: anchor, to: base))
        let a = r.graph.nodes.first { $0.id == "a" }!
        #expect(a.type == .approval && a.slug == "one")
        #expect(r.graph.edges == base.edges)
        #expect(EditorOps.applyPick(.type(.loop), at: anchor, to: base) == nil)
    }

    @Test func loopBodyAnchorAppendsIntoTheBody() throws {
        let r = try #require(EditorOps.applyPick(
            .type(.script), at: .init(afterNodeId: "loop", beforeNodeId: EditorAnchor.loopBody, bodyOnly: true), to: looped))
        #expect(bodyOf(r.graph.nodes).last == r.selected)
    }

    @Test func relabelRewritesReferences() {
        var nodes = chain.nodes
        nodes[1].slug = "one"
        nodes[2].config = ["scriptContent": "echo {{steps.one.output}}"]
        let r = EditorOps.relabel(nodes, nodeId: "a", label: "First step")
        #expect(r[1].slug == "first_step")
        #expect(r[2].config.str("scriptContent") == "echo {{steps.first_step.output}}")
    }

    @Test func migrationFillsAndDedupesSlugs() {
        var nodes = chain.nodes
        nodes[2].slug = "one"
        nodes.append(wn("c", .script, "X"))
        nodes[3].slug = "one"
        let m = EditorOps.migrateSlugs(nodes)
        #expect(m[0].slug == nil)
        #expect(m[1].slug == "one")
        #expect(m[3].slug == "one_2")
    }

    @Test func requirementsOfTheCanvas() {
        let nodes = [
            wn("h", .httpRequest, nil, ["profileConnectionId": ""]),
            wn("pub", .httpRequest, nil, ["url": "x"]),
            wn("c", .callConnectorAction, nil, ["connectionId": "", "connectorId": "github"]),
            wn("bound", .callConnectorAction, nil, ["connectionId": "c1", "connectorId": "github"]),
            wn("s", .script, nil, ["secretsFrom": ""]),
        ]
        let reqs = TemplateRequirements.requirementsOfDefinition(nodes)
        #expect(reqs.map(\.nodeId) == ["h", "c"])
        #expect(reqs.map(\.description) == ["an HTTP profile", "a github connection"])
    }
}

@Suite struct WorkflowModelRoundTripTests {
    @Test func unknownKeysAndPoliciesSurviveASave() throws {
        let json = #"""
        {"id":"w","name":"W","icon":"Workflow","iconColor":"#fff","enabled":true,"future":{"a":1},
         "nodes":[{"id":"n","type":"script","label":"S","config":{"x":1},"position":{"x":8,"y":16},
                   "onError":{"retry":2},"note":"keep"},
                  {"id":"m","type":"teleport","label":"T","config":{},"position":{"x":0,"y":0}}],
         "edges":[{"id":"e","source":"n","target":"m","weight":3}]}
        """#
        let wf = try JSONDecoder().decode(WorkflowDefinition.self, from: Data(json.utf8))
        let again = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(wf))
        let original = try JSONDecoder().decode(JSONValue.self, from: Data(json.utf8))
        #expect(again == original)
    }
}
