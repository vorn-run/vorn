import Foundation
import Testing
@testable import VornApp
import VornCore

/// CommandPalette.tsx's matching, ranking, grouping and command list, without a window.
struct CommandPaletteTests {
    private func cmd(_ id: String, _ label: String, _ category: PaletteCategory = .actions,
                     keywords: [String] = []) -> PaletteCommand {
        PaletteCommand(id: id, label: label, category: category, keywords: keywords, action: .openTasks)
    }

    @Test func scoreTiers() {
        #expect(PaletteSearch.score("new session", label: "New Session", keywords: []) == 100)
        #expect(PaletteSearch.score("new", label: "New Session", keywords: []) == 80)
        #expect(PaletteSearch.score("ses", label: "New Session", keywords: []) == 60)
        #expect(PaletteSearch.score("ssio", label: "New Session", keywords: []) == 40)
        #expect(PaletteSearch.score("agent", label: "New Session", keywords: ["create", "agent"]) == 20)
        #expect(PaletteSearch.score("zzz", label: "New Session", keywords: ["create"]) == 0)
        // Keywords are joined with spaces, so a query can span two of them.
        #expect(PaletteSearch.score("e a", label: "Xyz", keywords: ["create", "agent"]) == 20)
    }

    @Test func emptyQueryGroupsByCategoryAndHidesQuickLaunch() {
        let commands = [
            cmd("p", "Proj", .projects), cmd("q", "Claude on vorn", .quicklaunch), cmd("a", "Act", .actions),
            cmd("f", "Show All", .filter), cmd("t", "Shell", .terminals),
        ]
        let results = PaletteSearch.results(commands, query: "  ")
        #expect(results.map(\.id) == ["a", "t", "p", "f"])
        let sections = PaletteSearch.sections(results, query: "")
        #expect(sections.map { $0.category?.label } == ["Actions", "Terminals", "Projects", "Filter & Sort"])
        #expect(sections.flatMap(\.items).map(\.index) == [0, 1, 2, 3])
    }

    @Test func queryRanksByScoreStablyInOneFlatSection() {
        let commands = [
            cmd("1", "Open Tasks View", keywords: ["term"]), cmd("2", "New Terminal Session"),
            cmd("3", "Terminal in vorn", .quicklaunch), cmd("4", "Terminal in web", .quicklaunch),
            cmd("5", "term"), cmd("6", "Nothing"),
        ]
        let results = PaletteSearch.results(commands, query: " term ")
        #expect(results.map(\.id) == ["5", "3", "4", "2", "1"])
        let sections = PaletteSearch.sections(results, query: " term ")
        #expect(sections.count == 1 && sections[0].category == nil)
        #expect(PaletteSearch.sections([], query: "zz").isEmpty)
    }

    @Test func arrowsWrap() {
        #expect(PaletteSearch.step(0, by: 1, count: 3) == 1)
        #expect(PaletteSearch.step(2, by: 1, count: 3) == 0)
        #expect(PaletteSearch.step(0, by: -1, count: 3) == 2)
        #expect(PaletteSearch.step(0, by: 1, count: 0) == 0)
        #expect(PaletteSearch.step(0, by: -1, count: 0) == 0)
    }

    @Test func highlightUsesTheRawQuery() {
        let parts = PaletteSearch.highlight("New Terminal Session", query: "TERM")
        #expect(parts?.before == "New " && parts?.match == "Term" && parts?.after == "inal Session")
        #expect(PaletteSearch.highlight("New Terminal", query: "   ") == nil)
        // The untrimmed query is searched, as highlightMatch does, so a trailing space must match too.
        #expect(PaletteSearch.highlight("New Terminal", query: "new ")?.match == "New ")
        #expect(PaletteSearch.highlight("Terminal", query: "term ") == nil)
    }

    @Test func recentProjectNames() {
        let projects = [Project(name: "vorn", path: "/Users/me/dev/vorn/"), Project(name: "web", path: "/Users/me/web")]
        #expect(PaletteCommands.projectName(for: "/users/me/dev/VORN", in: projects) == "vorn")
        #expect(PaletteCommands.projectName(for: "/Users/me/dev/.vorn-worktrees/vorn/feat-x", in: projects) == "vorn")
        #expect(PaletteCommands.projectName(for: "/Users/me/other/thing/", in: projects) == "thing")
        #expect(PaletteCommands.projectName(for: "/", in: []) == "untitled")
    }

    @Test func activityAndDisplayName() {
        func recent(_ n: Int, _ label: String) -> RecentSession {
            RecentSession(sessionId: "r", agentType: .claude, display: "", projectPath: "/p", timestamp: 0,
                          activityCount: n, activityLabel: label, canResumeExact: true)
        }
        #expect(PaletteCommands.activity(recent(1, "prompt")) == "1 prompt")
        #expect(PaletteCommands.activity(recent(3, "prompt")) == "3 prompts")
        #expect(PaletteCommands.activity(recent(2, "entry")) == "2 entries")
        var s = TerminalSession(id: "a", agentType: .shell, projectName: "vorn", projectPath: "/p")
        #expect(PaletteCommands.displayName(s) == "vorn")
        s.displayName = "  Build  "
        #expect(PaletteCommands.displayName(s) == "Build")
    }

    @Test func buildsEveryCommandFromSources() {
        var src = PaletteSources()
        src.now = Date(timeIntervalSince1970: 1_000_000)
        src.sessions = [
            TerminalSession(id: "s1", agentType: .claude, projectName: "vorn", projectPath: "/v", status: .waiting,
                            branch: "main"),
            TerminalSession(id: "s2", agentType: .shell, projectName: "vorn", projectPath: "/v"),
        ]
        src.claimed = ["s2"]
        src.cards = [PaletteCard(id: "card-1", sessionId: "s1", path: "/v/src/greet.ts")]
        src.recent = [
            RecentSession(sessionId: "r1", agentType: .codex, display: "", projectPath: "/v", timestamp: 1_000_000_000 - 120_000,
                          activityCount: 3, activityLabel: "prompt", canResumeExact: true),
            RecentSession(sessionId: "r2", agentType: .codex, display: "Old", projectPath: "/v", timestamp: 0,
                          activityCount: 1, activityLabel: "prompt", canResumeExact: false),
        ]
        var remote = Project(name: "box", path: "/srv/box")
        remote.hostIds = ["host-1"]
        src.projects = [Project(name: "vorn", path: "/v"), Project(name: "plain", path: "/plain"), remote]
        src.workflows = [WorkflowDefinition(id: "wf1", name: "Nightly", nodes: [
            WorkflowNode(id: "t", type: .trigger, label: "T"),
            WorkflowNode(id: "l", type: .launchAgent, label: "L", config: ["projectName": "vorn"]),
        ])]
        src.installedAgents = [.claude]
        src.gitRepos = ["/v": true, "/plain": false]
        let wt = Worktree(path: "/v/.vorn-worktrees/feat", branch: "feat", isMain: false, name: "feat")
        src.worktrees = ["/v": [Worktree(path: "/v", branch: "main", isMain: true, name: "vorn"), wt],
                         "/plain": [wt]]
        let all = PaletteCommands.build(src)
        let byId = Dictionary(all.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })

        #expect(all.filter { $0.category == .actions }.count == 14)
        #expect(byId["action:stop-sessions-and-server"]?.sublabel == "2 running")
        #expect(byId["action:new-session"]?.shortcut == "⌘N")
        #expect(byId["action:manage-ssh"]?.action == .settings(.ssh))

        #expect(byId["terminal:s1"]?.label == "vorn")
        #expect(byId["terminal:s1"]?.keywords == [AgentType.claude.displayName, "vorn", "waiting"])
        #expect(byId["terminal:s2"] == nil)
        #expect(byId["card:card-1"]?.label == "greet.ts")
        #expect(byId["card:card-1"]?.keywords == ["/v/src/greet.ts", "vorn", "main"])

        #expect(byId["recent:r1"]?.label == "Untitled session")
        #expect(byId["recent:r1"]?.sublabel == "vorn · 2m ago · 3 prompts")
        #expect(byId["recent:r2"] == nil)

        #expect(all.filter { $0.category == .projects }.map(\.id) == ["project:all", "project:vorn", "project:plain", "project:box"])
        #expect(byId["project:vorn"]?.keywords == ["/v"])
        #expect(byId["workflow:wf1"]?.keywords == ["vorn"])

        let quick = all.filter { $0.category == .quicklaunch }.map(\.id)
        #expect(quick == [
            "quicklaunch:claude:vorn", "quicklaunch:claude:vorn:wt:/v/.vorn-worktrees/feat", "quicklaunch:claude:vorn:worktree",
            "quicklaunch:claude:plain",
            "terminal:project:vorn", "terminal:project:vorn:wt:/v/.vorn-worktrees/feat", "terminal:project:plain",
        ])
        #expect(byId["quicklaunch:claude:vorn:wt:/v/.vorn-worktrees/feat"]?.sublabel == "Existing worktree")
        #expect(byId["quicklaunch:claude:vorn:worktree"]?.action == .launchNewWorktree(.claude, project: "vorn"))
        #expect(byId["terminal:project:vorn:wt:/v/.vorn-worktrees/feat"]?.action == .terminalIn(project: "vorn", worktree: wt))

        #expect(all.filter { $0.category == .filter }.map(\.label) == [
            "Show All", "Show Running", "Show Waiting", "Show Idle", "Show Errors",
            "Sort: Manual", "Sort: Created", "Sort: Recent Activity",
        ])
        #expect(byId["filter:status:error"]?.shortcut == "⌘5")
    }
}
