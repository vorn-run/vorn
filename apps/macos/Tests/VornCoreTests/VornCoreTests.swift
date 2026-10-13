import Foundation
import Testing
@testable import VornCore

struct JSONValueTests {
    @Test func roundTripsNestedValues() throws {
        let json = #"{"a":[1,"x",true,null],"b":{"c":2.5}}"#
        let value = try JSONDecoder().decode(JSONValue.self, from: Data(json.utf8))
        #expect(value["a"] == [1, "x", true, nil])
        #expect(value["b"]?["c"] == 2.5)
        let again = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(value))
        #expect(again == value)
    }

    @Test func decodesSessions() throws {
        let value: JSONValue = [
            "id": "s1", "agentType": "claude", "projectName": "p", "projectPath": "/p",
            "status": "bogus", "createdAt": 1, "pid": 2,
        ]
        let s = try value.decode(TerminalSession.self)
        #expect(s.agentType == .claude)
        #expect(s.status == .idle)
    }
}

@MainActor
struct VornStoreTests {
    private func session(_ id: String, status: AgentStatus = .running) -> TerminalSession {
        TerminalSession(id: id, agentType: .shell, projectName: "p", projectPath: "/p", status: status)
    }

    @Test func foldsSessionNotifications() throws {
        let store = VornStore(projects: [], sessions: [session("a"), session("b")])
        store.apply(.notification(method: "session:created", params: try .from(session("c"))))
        #expect(store.sessions.map(\.id) == ["a", "b", "c"])

        store.apply(.notification(method: "session:reordered", params: ["c", "a", "b"]))
        #expect(store.sessions.map(\.id) == ["c", "a", "b"])

        store.apply(.notification(method: "widget:status-update", params: [["id": "a", "status": "waiting"]]))
        #expect(store.sessions[1].status == .waiting)

        store.apply(.notification(method: "terminal:exit", params: ["id": "b"]))
        #expect(store.sessions[2].status == .idle)
        #expect(store.endedIDs == ["b"])
    }

    @Test func broadcastsKeepTheShellsProjectAndWorktree() throws {
        var held = session("a")
        held.worktreePath = "/p/.wt/x"
        held.worktreeName = "x"
        held.isWorktree = true
        let store = VornStore(projects: [], sessions: [held])
        var update = TerminalSession(id: "a", agentType: .shell, projectName: "x", projectPath: "/p/.wt/x", status: .waiting)
        update.displayName = "Renamed"
        store.apply(.notification(method: "session:updated", params: try .from(update)))
        let s = try #require(store.session("a"))
        #expect(s.projectName == "p" && s.projectPath == "/p")
        #expect(s.worktreePath == "/p/.wt/x" && s.worktreeName == "x" && s.isWorktree == true)
        #expect(s.status == .waiting && s.displayName == "Renamed")
    }
}

struct VornEndpointTests {
    @Test func discoversEndpointFiles() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        #expect(throws: VornEndpoint.DiscoveryError.self) { try VornEndpoint.discover(dataDir: dir) }

        try Data(#"{"port":4321,"pid":7}"#.utf8).write(to: dir.appendingPathComponent("ws-port"))
        try Data("secret\n".utf8).write(to: dir.appendingPathComponent("local-token"))
        let endpoint = try VornEndpoint.discover(dataDir: dir)
        #expect(endpoint.port == 4321)
        #expect(endpoint.token == "secret")
        #expect(endpoint.webSocketURL.absoluteString == "ws://127.0.0.1:4321/ws")
    }

    @Test func honoursDataDirOverride() {
        let dir = VornEndpoint.defaultDataDir(environment: ["VORN_DATA_DIR": "/tmp/vorn-x"])
        #expect(dir.path == "/tmp/vorn-x")
    }
}

struct NotificationTests {
    let session = TerminalSession(id: "s1", agentType: .claude, projectName: "api", projectPath: "/p", status: .idle,
                                  createdAt: 0, pid: 1, displayName: " fix login ")

    @Test func agentNoticesUseTodaysWords() {
        let bell = AppNotice.agent(session, .bell)
        #expect(bell.title == "Claude Code notification")
        #expect(bell.body == "fix login is requesting your attention")
        #expect(bell.key == "s1")
        #expect(bell.target == .session("s1"))
        #expect(AppNotice.agent(session, .waiting).title == "Claude Code needs input")
        #expect(AppNotice.agent(session, .error).body == "fix login encountered an error")
        var shell = session
        shell.agentType = .shell
        shell.displayName = nil
        #expect(AppNotice.agent(shell, .bell).title == "Shell notification")
        #expect(AppNotice.agent(shell, .bell).body == "api is requesting your attention")
    }

    @Test func aProgramNoticeKeepsItsOwnWords() {
        let n = AppNotice.program(session, title: "", body: "Done")
        #expect(n.title == "fix login")
        #expect(n.body == "Done")
        #expect(n.key == "notify:s1")
        #expect(AppNotice.program(session, title: "Build", body: "").title == "Build")
    }

    @Test func prefsGateBellAndWorkflowWaits() throws {
        #expect(!NotificationPrefs().wantsBell)
        #expect(NotificationPrefs(enabled: true).wantsBell)
        #expect(!NotificationPrefs(enabled: true, onBell: false).wantsBell)
        #expect(!NotificationPrefs(enabled: true, onWaiting: false).wantsWorkflowWait)
        let lenient = try JSONDecoder().decode(
            AppConfigSummary.self, from: Data(#"{"defaults":{"notifications":{"enabled":true,"soundVolume":"loud"}}}"#.utf8))
        #expect(lenient.defaults?.notifications?.enabled == true)
        #expect(lenient.defaults?.notifications?.volume == 0.5)
    }

    @Test func aRepeatWithinTenSecondsIsHeldBack() {
        var c = NoticeCooldown()
        let t = Date(timeIntervalSince1970: 1000)
        let answers = [
            c.admit("a", at: t), c.admit("a", at: t.addingTimeInterval(9)),
            c.admit("b", at: t.addingTimeInterval(9)), c.admit("a", at: t.addingTimeInterval(10)),
        ]
        #expect(answers == [true, false, true, true])
    }

    @Test func tonesMatchTheirProfiles() {
        #expect(NotificationReason.waiting.tone == (880, 0.15))
        #expect(NotificationReason.error.tone == (330, 0.3))
        #expect(NotificationReason.bell.tone == (660, 0.2))
    }
}
