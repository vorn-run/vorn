import AppKit
import Foundation
import Testing
import VornCore
@testable import VornTerminals

/// Where each terminal key goes before the grid's encoder sees it.
struct TerminalKeysTests {
    private func route(_ keyCode: UInt16, _ mods: NSEvent.ModifierFlags = [], _ chars: String? = nil,
                       alternate: Bool = false, meta: Bool = false) -> TerminalKey {
        TerminalKeys.route(keyCode: keyCode, modifiers: mods, unmodified: chars, alternateScreen: alternate, optionAsMeta: meta)
    }

    @Test func optionArrowsMoveByWord() {
        #expect(route(123, .option) == .send("\u{1b}b"))
        #expect(route(124, .option) == .send("\u{1b}f"))
        // AppKit adds the function and numeric pad flags to arrows; they change nothing.
        #expect(route(123, [.option, .function, .numericPad]) == .send("\u{1b}b"))
    }

    @Test func commandArrowsMoveToLineEnds() {
        #expect(route(123, .command) == .send("\u{01}"))
        #expect(route(124, .command) == .send("\u{05}"))
    }

    @Test func commandUpDownStayWithTheEncoder() {
        #expect(route(126, .command) == .encode)
        #expect(route(125, .command) == .encode)
    }

    @Test func backspaceChordsDeleteWordAndLine() {
        #expect(route(51, .option) == .send("\u{1b}\u{7f}"))
        #expect(route(51, .command) == .send("\u{15}"))
        #expect(route(51) == .encode)
    }

    @Test func plainArrowsHomeEndAndPagesGoToTheEncoder() {
        for key: UInt16 in [123, 124, 125, 126, 115, 119, 116, 121] {
            #expect(route(key, .function) == .encode, "key \(key)")
        }
        #expect(route(115, .shift) == .encode)
        #expect(route(119, .shift) == .encode)
    }

    @Test func shiftPagesScrollHistoryOutsideTheAlternateScreen() {
        #expect(route(116, .shift) == .page(up: true))
        #expect(route(121, .shift) == .page(up: false))
        #expect(route(116, .shift, alternate: true) == .encode)
        #expect(route(121, .shift, alternate: true) == .encode)
    }

    @Test func optionAsMetaSendsEscapeThenTheKey() {
        #expect(route(0, .option, "a", meta: true) == .send("\u{1b}a"))
        #expect(route(0, [.option, .shift], "A", meta: true) == .send("\u{1b}A"))
        #expect(route(11, .option, "b", meta: true) == .send("\u{1b}b"))
        // Off, Option types the character it makes.
        #expect(route(0, .option, "a") == .encode)
        // Not with Command or Control, nor for named keys.
        #expect(route(0, [.option, .command], "a", meta: true) == .encode)
        #expect(route(0, [.option, .control], "a", meta: true) == .encode)
        #expect(route(122, .option, "\u{F704}", meta: true) == .encode)
        #expect(route(36, .option, "\r", meta: true) == .encode)
        // The word moves win over Meta.
        #expect(route(123, .option, "\u{F702}", meta: true) == .send("\u{1b}b"))
    }

    @Test func terminalShortcutsFindAndClear() {
        #expect(TerminalKeys.shortcut(keyCode: 3, modifiers: .command) == .find)
        #expect(TerminalKeys.shortcut(keyCode: 5, modifiers: .command) == .findNext)
        #expect(TerminalKeys.shortcut(keyCode: 5, modifiers: [.command, .shift]) == .findPrevious)
        #expect(TerminalKeys.shortcut(keyCode: 40, modifiers: [.command, .option]) == .clear)
        // ⌘K stays the command palette; ⌘C ⌘V ⌘A go through the Edit menu.
        #expect(TerminalKeys.shortcut(keyCode: 40, modifiers: .command) == nil)
        for key: UInt16 in [8, 9, 0] { #expect(TerminalKeys.shortcut(keyCode: key, modifiers: .command) == nil) }
        #expect(TerminalKeys.shortcut(keyCode: 3, modifiers: []) == nil)
    }

    @Test func droppedPathsAreQuotedForTheShell() {
        #expect(TerminalKeys.shellQuoted("/tmp/a.txt") == "/tmp/a.txt")
        #expect(TerminalKeys.shellQuoted("/tmp/my file.txt") == "'/tmp/my file.txt'")
        #expect(TerminalKeys.shellQuoted("/tmp/it's") == "'/tmp/it'\\''s'")
        #expect(TerminalKeys.shellQuoted("/tmp/$HOME;rm") == "'/tmp/$HOME;rm'")
        let urls = [URL(fileURLWithPath: "/tmp/a"), URL(fileURLWithPath: "/tmp/b c")]
        #expect(TerminalKeys.quotedPaths(urls) == "/tmp/a '/tmp/b c' ")
    }

    @Test func fontSizeStepsStayInRange() {
        #expect(TerminalPreferences.fontSize(13, by: 1) == 14)
        #expect(TerminalPreferences.fontSize(13, by: -1) == 12)
        #expect(TerminalPreferences.fontSize(nil, by: 1) == 14)
        #expect(TerminalPreferences.fontSize(24, by: 1) == 24)
        #expect(TerminalPreferences.fontSize(8, by: -1) == 8)
        #expect(TerminalPreferences.fontSize(12.6, by: 1) == 14)
    }
}
