import AppKit
import Foundation
import Testing
@testable import VornApp
import VornCore

/// Where a sidebar click, a shortcut or Esc lands (SessionItem.tsx, useKeyboardShortcuts.ts), without a window.
@MainActor
struct RoutingTests {
    private func fixture(_ layout: LayoutMode = .grid) -> (AppModel, VornStore) {
        var wt = TerminalSession(id: "c", agentType: .claude, projectName: "web", projectPath: "/web")
        wt.isWorktree = true
        wt.worktreePath = "/web-wt"
        let store = VornStore(
            projects: [Project(name: "vorn", path: "/vorn"), Project(name: "web", path: "/web")],
            sessions: [
                TerminalSession(id: "a", agentType: .claude, projectName: "vorn", projectPath: "/vorn"),
                TerminalSession(id: "b", agentType: .shell, projectName: "web", projectPath: "/web"),
                wt,
            ])
        let model = AppModel()
        model.persistsSidebar = false
        model.layoutOverride = layout
        return (model, store)
    }

    @Test func sidebarSessionExpandsItsCardInTheGrid() {
        let (model, store) = fixture()
        model.reveal("b", store)
        #expect(model.expandedSessionID == "b")
        #expect(model.selectedSessionID == "b")
        #expect(model.focusRequests["b"] == 1)
        #expect(model.isShowing("b", store))
        #expect(!model.isShowing("a", store))
    }

    @Test func sidebarSessionSelectsItsTabInTabs() {
        let (model, store) = fixture(.tabs)
        model.setFocused("a")
        model.reveal("b", store)
        #expect(model.activeTabID == "b")
        #expect(model.expandedSessionID == nil)
        #expect(model.selectedSessionID == "b")
        #expect(model.focusRequests["b"] == 1)
        #expect(model.isShowing("b", store))
    }

    @Test func projectAndWorktreeFilterTheGridAndCollapse() {
        let (model, store) = fixture()
        model.setFocused("a")
        model.select(project: "web")
        #expect(model.expandedSessionID == nil)
        #expect(model.visibleSessions(store).map(\.id) == ["b", "c"])

        model.toggle(worktree: "/web-wt", in: "web")
        #expect(model.visibleSessions(store).map(\.id) == ["c"])
        model.toggle(worktree: AppModel.mainWorktree, in: "web")
        #expect(model.visibleSessions(store).map(\.id) == ["b"])
        model.toggle(worktree: AppModel.mainWorktree, in: "web")
        #expect(model.activeWorktreePath == nil)
        #expect(model.visibleSessions(store).map(\.id) == ["b", "c"])

        model.select(project: nil)
        #expect(model.visibleSessions(store).map(\.id) == ["a", "b", "c"])
    }

    @Test func cycleFocusFollowsTheLayout() {
        let (model, store) = fixture()
        model.cycleFocus(store, by: 1)
        #expect(model.selectedSessionID == "a" && model.expandedSessionID == nil)
        model.cycleFocus(store, by: -1)
        #expect(model.selectedSessionID == "c")

        model.setFocused("c")
        model.cycleFocus(store, by: 1)
        #expect(model.expandedSessionID == "a")

        let (tabs, tabStore) = fixture(.tabs)
        tabs.activeTabID = "b"
        tabs.cycleFocus(tabStore, by: 1)
        #expect(tabs.activeTabID == "c")
        tabs.cycleFocus(tabStore, by: 1)
        #expect(tabs.activeTabID == "a")
    }

    @Test func jumpTakesThePositionInTheVisibleOrder() {
        let (model, store) = fixture()
        model.jump(to: 2, store)
        #expect(model.selectedSessionID == "b" && model.expandedSessionID == nil)
        model.jump(to: 9, store)
        #expect(model.selectedSessionID == "b")
        model.setFocused("a")
        model.jump(to: 3, store)
        #expect(model.expandedSessionID == "c")

        let (tabs, tabStore) = fixture(.tabs)
        tabs.jump(to: 1, tabStore)
        #expect(tabs.activeTabID == "a")
    }

    @Test func reorderMovesIntoTheTargetSlot() {
        #expect(AppModel.reordered(["a", "b", "c"], moving: "a", onto: "c") == ["b", "c", "a"])
        #expect(AppModel.reordered(["a", "b", "c"], moving: "c", onto: "a") == ["c", "a", "b"])
        #expect(AppModel.reordered(["a", "b"], moving: "a", onto: "a") == nil)
        #expect(AppModel.reordered(["a", "b"], moving: "x", onto: "a") == nil)
    }

    @Test func escapeClosesTheInnermostThingFirst() {
        let (model, _) = fixture()
        model.setFocused("a")
        model.renamingSessionID = "a"
        model.settingsOpen = true
        model.overlay = .commandPalette
        #expect(model.escape() && model.overlay == nil)
        #expect(model.escape() && !model.settingsOpen)
        #expect(model.escape() && model.renamingSessionID == nil)
        #expect(model.escape() && model.expandedSessionID == nil && model.selectedSessionID == "a")
        #expect(model.escape() && model.selectedSessionID == nil)
        #expect(!model.escape())
    }

    @Test func terminalKeepsEscTabAndControlKeys() {
        let (model, _) = fixture()
        model.setFocused("a")
        // Esc in a focused terminal is the program's, with nothing open over it.
        #expect(!KeyRouting.handle(keyCode: 53, modifiers: [], typing: true, model: model))
        #expect(model.expandedSessionID == "a")
        // Tab, Shift-Tab and Ctrl-C are never the app's.
        #expect(!KeyRouting.handle(keyCode: 48, modifiers: [], typing: true, model: model))
        #expect(!KeyRouting.handle(keyCode: 48, modifiers: .shift, typing: true, model: model))
        #expect(!KeyRouting.handle(keyCode: 8, modifiers: .control, typing: true, model: model))
        // With the palette open, Esc closes it even from the terminal.
        model.overlay = .commandPalette
        #expect(KeyRouting.handle(keyCode: 53, modifiers: [], typing: true, model: model))
        #expect(model.overlay == nil && model.expandedSessionID == "a")
        // Outside a terminal Esc collapses the expanded card.
        #expect(KeyRouting.handle(keyCode: 53, modifiers: [], typing: false, model: model))
        #expect(model.expandedSessionID == nil && model.selectedSessionID == "a")
    }

    @Test func commandWCollapsesOnlyAnExpandedCard() {
        let (model, _) = fixture()
        #expect(!KeyRouting.handle(keyCode: 13, modifiers: .command, typing: true, model: model))
        model.setFocused("b")
        #expect(KeyRouting.handle(keyCode: 13, modifiers: .command, typing: true, model: model))
        #expect(model.expandedSessionID == nil && model.selectedSessionID == "b")
        #expect(KeyRouting.handle(keyCode: 40, modifiers: .command, typing: true, model: model))
        #expect(model.overlay == .commandPalette)
    }
}
