import CoreGraphics
import Foundation
import Testing
@testable import VornCore

/// The welcome guide's pages and opening rule, against onboarding-data.ts, App.tsx and onboarding-mcp.test.tsx.
struct OnboardingRuleTests {
    @Test func sevenPagesInTheRendererOrder() {
        #expect(Onboarding.steps.map(\.id) == ["agents", "projects", "sessions", "workspace", "workflows", "shortcuts", "ready"])
        #expect(Onboarding.steps.first?.title == "Coding Agents")
        #expect(Onboarding.steps.last?.subtitle == "Start building with your agents")
        #expect(Onboarding.version == 2)
    }

    @Test func opensUntilTheCurrentVersionWasSeen() {
        #expect(Onboarding.shouldOpen(hasSeen: nil))
        #expect(Onboarding.shouldOpen(hasSeen: .null))
        #expect(Onboarding.shouldOpen(hasSeen: .bool(false)))
        #expect(Onboarding.shouldOpen(hasSeen: .bool(true)))
        #expect(Onboarding.shouldOpen(hasSeen: .number(1)))
        #expect(!Onboarding.shouldOpen(hasSeen: .number(2)))
        #expect(!Onboarding.shouldOpen(hasSeen: .number(3)))
    }

    @Test func readsTheFlagAsJavaScriptsNumberDoes() {
        #expect(Onboarding.shouldOpen(hasSeen: .string("1")))
        #expect(Onboarding.shouldOpen(hasSeen: .string("")))
        #expect(!Onboarding.shouldOpen(hasSeen: .string(" 2 ")))
        #expect(!Onboarding.shouldOpen(hasSeen: .string("yes")))
        #expect(!Onboarding.shouldOpen(hasSeen: .object([:])))
        #expect(Onboarding.shouldOpen(hasSeen: .array([])))
        #expect(Onboarding.shouldOpen(hasSeen: .array([.number(1)])))
        #expect(!Onboarding.shouldOpen(hasSeen: .array([.number(1), .number(2)])))
        #expect(Onboarding.shouldOpen(hasSeen: .string("0x1")))
        #expect(Onboarding.shouldOpen(hasSeen: .string("-Infinity")))
    }

    @Test func arrowsStopAtTheEnds() {
        #expect(Onboarding.next(0) == 1)
        #expect(Onboarding.next(6) == 6)
        #expect(Onboarding.previous(0) == 0)
        #expect(Onboarding.previous(3) == 2)
        #expect(Onboarding.clamp(42) == 6)
        #expect(Onboarding.clamp(-1) == 0)
    }

    @Test func pluginSetupsListTheInstalledAgentsElseTheFirstTwo() {
        let order: [AgentType] = [.claude, .copilot, .codex, .opencode, .gemini]
        #expect(Onboarding.mcpAgents(order: order) { $0 == .codex || $0 == .gemini } == [.codex, .gemini])
        #expect(Onboarding.mcpAgents(order: order) { _ in false } == [.claude, .copilot])
        #expect(Onboarding.mcpAgents(order: order) { $0 == .claude } == [.claude])
    }
}

/// The widget's glow, rows, sizes and permission cards, against widget-glow-color.test.ts, Widget.tsx and index.ts.
struct WidgetLogicTests {
    @Test func glowColoursMatchTheRenderer() {
        #expect(WidgetGlow.of(.claude).css == "rgba(217,119,87,0.45)")
        #expect(WidgetGlow.of(.copilot).css == "rgba(255,255,255,0.3)")
        #expect(WidgetGlow.of(.codex).css == "rgba(122,157,255,0.45)")
        #expect(WidgetGlow.of(.opencode).css == "rgba(255,255,255,0.3)")
        #expect(WidgetGlow.of(.gemini).css == "rgba(49,134,255,0.45)")
        #expect(WidgetGlow.of(.shell).css == "rgba(156,163,175,0.35)")
        #expect(WidgetGlow.of(.shell) != WidgetGlow.of(.claude))
    }

    @Test func rowsNameTheSessionOrItsProject() {
        let named = WidgetAgent(id: "a", agentType: .claude, displayName: "fix auth", projectName: "app", status: .running)
        let bare = WidgetAgent(id: "b", agentType: .codex, displayName: "", projectName: "app", status: .idle)
        #expect(named.label == "fix auth")
        #expect(bare.label == "app")
        #expect(named.tooltip == "app — running")
    }

    @Test func countsByStatus() {
        let agents: [WidgetAgent] = [AgentStatus.running, .running, .waiting, .error, .idle].enumerated().map {
            WidgetAgent(id: "\($0.offset)", agentType: .shell, displayName: nil, projectName: "p", status: $0.element)
        }
        let c = WidgetCounts(agents)
        #expect(c == { var w = WidgetCounts([]); w.running = 2; w.waiting = 1; w.error = 1; w.total = 5; return w }())
    }

    @Test func enabledOnlyByALiteralTrue() {
        var d = AppConfigSummary.Defaults()
        #expect(!d.isWidgetEnabled)
        d.widgetEnabled = .string("true")
        #expect(!d.isWidgetEnabled)
        d.widgetEnabled = .bool(true)
        #expect(d.isWidgetEnabled)
    }

    @Test func sitsBottomRightAndResizesFromThatCorner() {
        let area = CGRect(x: 0, y: 0, width: 1440, height: 900)
        let full = WidgetPlacement.initial(WidgetViewMode.full.size, in: area)
        #expect(full == CGRect(x: 1140, y: 480, width: 280, height: 400))
        let compact = WidgetPlacement.resized(full, to: WidgetViewMode.compact.size, in: area)
        #expect(compact == CGRect(x: 1280, y: 844, width: 140, height: 36))
        let pinned = WidgetPlacement.resized(CGRect(x: 1400, y: 880, width: 140, height: 36),
                                             to: WidgetViewMode.full.size, in: area)
        #expect(pinned == CGRect(x: 1160, y: 500, width: 280, height: 400))
        let corner = WidgetPlacement.resized(CGRect(x: 0, y: 0, width: 140, height: 36),
                                             to: WidgetViewMode.full.size, in: area)
        #expect(corner.origin == .zero)
    }

    @Test func suggestionLabels() {
        func addRule(_ content: String?, tool: String? = "Bash") -> JSONValue {
            var rule: [String: JSONValue] = [:]
            if let tool { rule["toolName"] = .string(tool) }
            if let content { rule["ruleContent"] = .string(content) }
            return .object(["type": .string("addRules"), "rules": .array([.object(rule)])])
        }
        #expect(PermissionRequest.label(suggestion: addRule("/Users/me/repo/src/**"), toolName: "Edit") == "Allow Bash in src/")
        #expect(PermissionRequest.label(suggestion: addRule("npm test"), toolName: "Bash") == "Always allow `npm test`")
        #expect(PermissionRequest.label(suggestion: addRule(String(repeating: "x", count: 31)), toolName: "Bash")
            == "Always allow `\(String(repeating: "x", count: 27))…`")
        #expect(PermissionRequest.label(suggestion: addRule(String(repeating: "x", count: 30)), toolName: "Bash")
            == "Always allow `\(String(repeating: "x", count: 30))`")
        #expect(PermissionRequest.label(suggestion: addRule(nil, tool: nil), toolName: "Write") == "Always allow Write")
        let mode = { (m: String?) -> JSONValue in
            .object(["type": .string("setMode")].merging(m.map { ["mode": .string($0)] } ?? [:]) { a, _ in a })
        }
        #expect(PermissionRequest.label(suggestion: mode("acceptEdits"), toolName: "") == "Auto-accept edits")
        #expect(PermissionRequest.label(suggestion: mode("plan"), toolName: "") == "Switch to plan mode")
        #expect(PermissionRequest.label(suggestion: mode("bypass"), toolName: "") == "bypass")
        #expect(PermissionRequest.label(suggestion: mode(nil), toolName: "") == "Set mode")
        #expect(PermissionRequest.label(suggestion: .object(["type": .string("other"), "label": .string("Do it")]),
                                        toolName: "") == "Do it")
        #expect(PermissionRequest.label(suggestion: .object(["type": .string("other")]), toolName: "") == "other")
    }

    @Test func cardContextIsTheFileTheCommandOrTheDescription() throws {
        func request(_ input: [String: JSONValue], description: String? = nil) throws -> PermissionRequest {
            var o: [String: JSONValue] = ["requestId": "r", "toolName": "Bash", "toolInput": .object(input)]
            if let description { o["description"] = .string(description) }
            return try #require(PermissionRequest(.object(o)))
        }
        let file = try request(["file_path": "/repo/src/main.ts"]).context
        #expect(file?.kind == .file && file?.text == "main.ts")
        let long = String(repeating: "a", count: 70)
        let cmd = try request(["command": .string(long)]).context
        #expect(cmd?.kind == .command && cmd?.text == String(repeating: "a", count: 61) + "…")
        let text = try request([:], description: "Fetch a page").context
        #expect(text?.kind == .text && text?.text == "Fetch a page")
        #expect(try request([:]).context == nil)
    }

    @Test func questionsAnswerIntoTheToolInput() throws {
        let raw: JSONValue = .object([
            "requestId": "q1", "terminalId": "t1", "toolName": "AskUserQuestion",
            "toolInput": .object(["questions": .array([])]),
            "questions": .array([.object(["header": "Pick", "question": "Which?",
                                          "options": .array([.object(["label": "A"]), .object(["label": "B"])])])]),
        ])
        let r = try #require(PermissionRequest(raw))
        #expect(r.isQuestion)
        #expect(r.questions.first?.options == ["A", "B"])
        #expect(r.answering([0: "B"])["answers"] == .object(["0": "B"]))
        #expect(r.answering([0: "B"])["questions"] == .array([]))
    }

    @Test func queueKeepsOneCopyAndGroupsByTerminal() throws {
        func req(_ id: String, _ terminal: String?) throws -> PermissionRequest {
            var o: [String: JSONValue] = ["requestId": .string(id), "toolName": "Edit"]
            if let terminal { o["terminalId"] = .string(terminal) }
            return try #require(PermissionRequest(.object(o)))
        }
        var q = PermissionQueue()
        q.add(try req("1", "t1"))
        q.add(try req("1", "t1"))
        q.add(try req("2", "t2"))
        q.add(try req("3", nil))
        #expect(q.requests.count == 3)
        #expect(q.forTerminal("t1").map(\.id) == ["1"])
        q.remove("1")
        #expect(q.forTerminal("t1").isEmpty)
        #expect(PermissionQueue.resolveParams("2", allow: true, updatedPermissions: [.string("s")])
            == .object(["requestId": "2", "allow": true, "updatedPermissions": .array(["s"])]))
        #expect(PermissionQueue.resolveParams("2", allow: false) == .object(["requestId": "2", "allow": false]))
    }
}
