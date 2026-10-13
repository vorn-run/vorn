import AppKit
import CVornGrid
import Foundation
import Testing
import VornCore
@testable import VornTerminals

/// Each terminal shortcut against a live shell on a test vornd: the bytes keys send, paste confirmation,
/// find, clear, font size, input method text and dropped paths.
@MainActor
struct TerminalKeysLiveTests {
    @Test func everyShortcutReachesTheShell() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let store = VornStore(dataDir: server.dataDir, readOnly: false)
        store.start()
        defer { store.stop() }
        #expect(await liveUntil(timeout: 15) { store.phase == .connected })
        try #require(store.phase == .connected)

        let shell = try await store.createShell(cwd: server.root.path)
        let engine = TerminalEngine(socket: try #require(store.endpoint?.gridSocket), readOnly: false)
        defer { engine.shutdown() }
        let window = NSWindow(contentRect: NSRect(x: -20000, y: -20000, width: 640, height: 400),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        let view = TerminalNSView(pane: engine.makePane(sessionID: shell.id))
        view.frame = CGRect(x: 0, y: 0, width: 640, height: 400)
        window.contentView = view
        window.makeFirstResponder(view)
        await engine.settle(timeout: 8)
        let pane = view.pane
        try #require(pane.view != nil, "state \(pane.state) error \(engine.lastError ?? "-")")

        func settled(_ check: () -> Bool) async -> Bool {
            await liveUntil(timeout: 10) {
                await engine.settle(timeout: 0.1)
                pane.contentChanged()
                pane.refresh()
                return check()
            }
        }
        // The screen's rows run together, as the loop's output wraps.
        func flat() -> String { pane.screenText().replacingOccurrences(of: "\n", with: "") }
        func key(_ code: UInt16, _ mods: NSEvent.ModifierFlags, _ chars: String, unmodified: String? = nil) {
            let e = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: mods, timestamp: 0,
                                     windowNumber: window.windowNumber, context: nil, characters: chars,
                                     charactersIgnoringModifiers: unmodified ?? chars, isARepeat: false, keyCode: code)!
            if !view.performKeyEquivalent(with: e) { view.keyDown(with: e) }
        }

        // A plain shell, then a loop that prints the code of each byte it reads.
        pane.text("exec /bin/bash --norc --noprofile\r")
        pane.text("PS1='$ '; clear\r")
        #expect(await settled { pane.screenText().hasPrefix("$") })
        pane.text("stty -icanon -echo -isig -iexten; while IFS= read -r -n1 -d '' c; do [ \"$c\" = q ] && break; printf '[%d]' \"'$c\"; done; stty sane\r")
        try await Task.sleep(for: .milliseconds(300))

        key(123, [.option, .function, .numericPad], "\u{F702}")
        key(124, [.option, .function, .numericPad], "\u{F703}")
        key(51, .option, "\u{7f}")
        key(123, [.command, .function, .numericPad], "\u{F702}")
        key(124, [.command, .function, .numericPad], "\u{F703}")
        key(51, .command, "\u{7f}")
        #expect(await settled { flat().contains("[27][98][27][102][27][127][1][5][21]") },
                "\(pane.screenText())")

        // Option as Meta: Esc then the key; off, the key goes to the encoder as the character Option makes.
        UserDefaults.standard.set(true, forKey: TerminalPreferences.optionAsMetaKey)
        key(0, .option, "å", unmodified: "a")
        UserDefaults.standard.removeObject(forKey: TerminalPreferences.optionAsMetaKey)
        #expect(await settled { flat().contains("[21][27][97]") }, "\(pane.screenText())")

        // Input method text arrives once composed, and dictation as plain text.
        view.setMarkedText("ka", selectedRange: NSRange(location: 2, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
        #expect(view.hasMarkedText())
        view.insertText("x", replacementRange: NSRange(location: NSNotFound, length: 0))
        #expect(!view.hasMarkedText())
        view.insertText("y", replacementRange: NSRange(location: NSNotFound, length: 0))
        #expect(await settled { flat().contains("[27][97][120][121]") }, "\(pane.screenText())")

        // A multi-line paste outside bracketed paste waits for a yes; a single line does not.
        var asked: [String] = []
        var answer = false
        view.confirmPaste = { text, send in asked.append(text); send(answer) }
        pane.paste("z")
        #expect(await settled { flat().contains("[121][122]") }, "\(pane.screenText())")
        #expect(asked.isEmpty)
        pane.paste("m\nn")
        #expect(await settled { asked == ["m\nn"] }, "asked \(asked)")
        try await Task.sleep(for: .milliseconds(300))
        await engine.settle(timeout: 0.2)
        #expect(!flat().contains("[109]"))
        answer = true
        pane.paste("m\nn")
        #expect(await settled { asked.count == 2 && flat().contains("[109]") && flat().contains("[110]") },
                "\(pane.screenText())")

        // A dropped file types its quoted path.
        pane.paste(TerminalKeys.quotedPaths([URL(fileURLWithPath: "/a b")]), confirmed: true)
        #expect(await settled { flat().contains("[39][47][97][32][98][39][32]") }, "\(pane.screenText())")
        pane.text("q")
        #expect(await settled { flat().contains("[32]$") }, "\(pane.screenText())")

        // Find: ⌘F opens the bar on the newest match, ⌘G and ⇧⌘G step round, Esc's close puts focus back.
        pane.text("clear; for i in $(seq 1 20); do echo hit$i; done\r")
        #expect(await settled { pane.screenText().contains("hit20") })
        #expect(view.performShortcut(keyCode: 3, modifiers: .command))
        #expect(view.isFinding)
        #expect(TerminalNSView.owning(window.firstResponder) === view)
        view.search("hit1")
        #expect(await settled { view.findStatus == "11 of 11" }, "status \(view.findStatus)")
        #expect(view.performShortcut(keyCode: 5, modifiers: .command))
        #expect(view.findStatus == "1 of 11")
        #expect(view.performShortcut(keyCode: 5, modifiers: [.command, .shift]))
        #expect(view.findStatus == "11 of 11")
        view.search("nothing-here")
        #expect(await settled { view.findStatus == "No results" }, "status \(view.findStatus)")
        view.closeFind()
        #expect(!view.isFinding)
        #expect(window.firstResponder === view)

        // ⌥⌘K clears the screen; ⌘K is left for the command palette.
        #expect(!view.performShortcut(keyCode: 40, modifiers: .command))
        #expect(view.performShortcut(keyCode: 40, modifiers: [.command, .option]))
        #expect(await settled { !pane.screenText().contains("hit") }, "\(pane.screenText())")

        // ⌘A selects the whole screen for ⌘C (left off the shared clipboard here).
        pane.text("echo copy-me\r")
        #expect(await settled { pane.screenText().contains("copy-me\n") })
        view.selectAll(nil)
        #expect(pane.selection.map { pane.text(of: $0) }?.contains("copy-me") == true)

        // A bigger font refits the grid in place, without a new engine.
        let cols = try #require(pane.view?.cols)
        engine.fontSize = 20
        #expect(await settled { (pane.view?.cols ?? cols) < cols }, "cols \(pane.view?.cols ?? 0) of \(cols)")
        #expect(pane.screenText().contains("copy-me"))
    }
}
