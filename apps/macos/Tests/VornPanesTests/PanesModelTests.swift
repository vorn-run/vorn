import Foundation
import Testing
import VornCore
import VornUI
@testable import VornPanes

@MainActor
struct PanesModelTests {
    private func model() -> PanesModel {
        let m = PanesModel(persists: false)
        m.confirmDiscard = { true }
        return m
    }

    @Test func previewTabIsReusedUntilPinnedOrEdited() {
        let m = model()
        m.toggleFilesPane("s")
        m.openFileTab("s", "/r/a.ts")
        m.openFileTab("s", "/r/b.ts")
        #expect(m.filesPanes["s"]?.tabs == ["/r/b.ts"])
        #expect(m.filesPanes["s"]?.preview == "/r/b.ts")
        m.setDirty(PaneID.fileTab("s", "/r/b.ts"), true)
        m.openFileTab("s", "/r/c.ts")
        #expect(m.filesPanes["s"]?.tabs == ["/r/b.ts", "/r/c.ts"])
        m.openFileTab("s", "/r/c.ts", pin: true)
        #expect(m.filesPanes["s"]?.preview == nil)
        m.closeFileTab("s", "/r/c.ts")
        #expect(m.filesPanes["s"]?.active == "/r/b.ts")
        m.closeSavedFileTabs("s")
        #expect(m.filesPanes["s"]?.tabs == ["/r/b.ts"])
    }

    @Test func discardIsAskedOnlyForDirtyEditors() {
        let m = model()
        var asked = 0
        m.confirmDiscard = { asked += 1; return false }
        m.toggleFilesPane("s")
        m.openFileTab("s", "/r/a.ts", pin: true)
        m.requestCloseFileTab("s", "/r/a.ts")
        #expect(asked == 0 && m.filesPanes["s"]?.tabs.isEmpty == true)
        m.openFileTab("s", "/r/a.ts", pin: true)
        m.setDirty(PaneID.fileTab("s", "/r/a.ts"), true)
        m.requestCloseFileTab("s", "/r/a.ts")
        #expect(asked == 1 && m.filesPanes["s"]?.tabs == ["/r/a.ts"])
        m.toggleFilesPane("s")
        #expect(asked == 2 && m.filesPanes["s"] != nil)
    }

    @Test func aFileOpensAsOneCardAndGoesBackAsATab() {
        let m = model()
        m.toggleFilesPane("s")
        m.openFileTab("s", "/r/a.ts", pin: true)
        m.popOutTab("s", "/r/a.ts")
        #expect(m.filesPanes["s"]?.tabs.isEmpty == true)
        let id = m.promoteFile("s", "/r/a.ts")
        #expect(m.cards(ownedBy: "s") == [id])
        m.returnCard(id)
        #expect(m.editorCards.isEmpty)
        #expect(m.filesPanes["s"]?.tabs == ["/r/a.ts"])
    }

    @Test func panelTerminalsAreClaimedUntilReleased() {
        let m = model()
        m.openTerminalsPane("s", terminal: "t1")
        m.openTerminalsPane("s", terminal: "t2")
        #expect(m.claimedTerminalIDs == ["t1", "t2"])
        #expect(m.terminalsPanes["s"]?.activeID == "t2")
        #expect(m.columnEntries("s").map(\.kind) == [.terminals])
        m.releaseTerminal("t2")
        #expect(m.terminalsPanes["s"]?.terminals == ["t1"])
        m.releaseTerminal("t1")
        #expect(m.terminalsPanes["s"] == nil)
        m.toggleFilesPane("s")
        m.openTerminalsPane("s", terminal: "t3")
        m.prune(liveSessions: ["s"])
        #expect(m.terminalsPanes["s"] == nil)
        #expect(m.columnEntries("s").map(\.kind) == [.files])
    }

    @Test func splitWeightsNormaliseAndClamp() {
        #expect(SplitRatio.weights(nil, count: 2) == [0.5, 0.5])
        #expect(SplitRatio.weights([3, 1], count: 2) == [0.75, 0.25])
        let r = SplitRatio.resize([0.5, 0.5], index: 0, ratio: 0.95)
        #expect(abs(r[0] - 0.85) < 1e-9)
        #expect(SplitRatio.clamp(0.01) == 0.15)
    }

    @Test func persistedStateRoundTrips() throws {
        let suite = "vorn.panes.test.\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let a = PanesModel(defaults: defaults)
        a.toggleFilesPane("s")
        a.openFileTab("s", "/r/a.ts", pin: true)
        a.openTerminalsPane("s", terminal: "t1")
        a.setCardSplit("s", terminal: 0.6)
        let id = a.promoteFile("s", "/r/b.ts")
        let b = PanesModel(defaults: defaults)
        #expect(b.filesPanes["s"]?.tabs == ["/r/a.ts"])
        #expect(b.terminalsPanes["s"]?.terminals == ["t1"])
        #expect(b.cardSplits["s"]?.terminal == 0.6)
        #expect(b.editorCards[id]?.filePath == "/r/b.ts")
        #expect(b.promoteFile("s", "/r/c.ts") != id)
    }

    @Test func findMatchesAreCaseInsensitiveAndDisjoint() {
        let m = FindMatch.compute(["Greet greet", "none", "GREETgreet"], "greet")
        #expect(m == [FindMatch(line: 0, start: 0, end: 5), FindMatch(line: 0, start: 6, end: 11),
                      FindMatch(line: 2, start: 0, end: 5), FindMatch(line: 2, start: 5, end: 10)])
        #expect(FindMatch.compute(["aaa"], "aa").count == 1)
    }

    @Test func diffRowsAndFeedback() {
        let diff = "diff --git a/x b/x\nindex 1..2\n--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n a\n-b\n+c\n"
        let rows = DiffRow.parse(diff)
        #expect(rows.map(\.kind) == [.hunk, .ctx, .del, .add])
        #expect(rows[2].oldLine == 2 && rows[3].newLine == 2)
        let text = DiffComment.feedback([DiffComment(filePath: "x", lineIndex: 7, lineContent: "+c", comment: "why")])
        #expect(text == "Please address the following review comments:\n\n**x:**\n- Line `c`: why\n\n")
        #expect(isTruncatedRead("abc\n\n--- truncated (12 bytes total) ---"))
        #expect(SyntaxHighlighter.language(for: "Dockerfile") == "dockerfile")
        #expect(SyntaxHighlighter.language(for: "app.tsx") == "tsx")
    }

    @Test func highlighterColoursWithTheBundledTheme() async {
        // The first line also compiles the grammar, which can outlast Shiki's 500 ms line limit on a busy runner.
        _ = await SyntaxHighlighter.shared.tokens("const x = 1", language: "typescript")
        let lines = await SyntaxHighlighter.shared.tokens("const x = 1", language: "typescript")
        #expect(lines?.first?.first?.content == "const ")
        #expect(lines?.first?.first?.color == 0xCB7676)
    }
}
