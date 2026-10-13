import AppKit
import CoreGraphics
import Foundation
import Testing
import VornCore
@testable import VornTerminals

/// Trackpad and wheel scrolling through a live shell's history, against a test vornd.
@MainActor
struct TerminalScrollLiveTests {
    @Test func scrollsHistoryByThePointAndTypingReturnsLive() async throws {
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
        await engine.settle(timeout: 8)
        let pane = view.pane
        try #require(pane.view != nil, "state \(pane.state) error \(engine.lastError ?? "-")")

        func pump() async {
            await engine.settle(timeout: 0.01)
            pane.contentChanged()
            pane.refresh()
            _ = view.scrollFrame()
        }
        func settled(_ check: () -> Bool) async -> Bool {
            await liveUntil(timeout: 15) {
                await pump()
                return check()
            }
        }
        /// Frames until every spring and ease has come to rest.
        func rest() async {
            for _ in 0..<80 {
                await pump()
                try? await Task.sleep(for: .milliseconds(8))
            }
        }
        /// One trackpad event, as the system sends them: touch or momentum phase, points.
        func pad(_ dy: Int32, scroll: Int64 = 0, momentum: Int64 = 0) async {
            let e = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1, wheel1: dy, wheel2: 0, wheel3: 0)!
            e.setIntegerValueField(.scrollWheelEventIsContinuous, value: 1)
            e.setIntegerValueField(.scrollWheelEventScrollPhase, value: scroll)
            e.setIntegerValueField(.scrollWheelEventMomentumPhase, value: momentum)
            view.scrollWheel(with: NSEvent(cgEvent: e)!)
            await pump()
            try? await Task.sleep(for: .milliseconds(8))
        }
        func shoot(_ name: String) {
            guard let dir = ProcessInfo.processInfo.environment["VORN_SHOT_DIR"] else { return }
            try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
            view.needsDisplay = true
            guard let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return }
            view.cacheDisplay(in: view.bounds, to: rep)
            try? rep.representation(using: .png, properties: [:])?
                .write(to: URL(fileURLWithPath: dir).appendingPathComponent("scroll-\(name).png"))
        }
        let (began, changed, ended): (Int64, Int64, Int64) = (1, 2, 4)
        let (mBegan, mGoing, mEnded): (Int64, Int64, Int64) = (1, 2, 3)
        func firstLine() -> String { pane.screenText().split(separator: "\n").first.map(String.init) ?? "" }

        pane.text("seq 1 5000; sleep 8; seq 7001 7040\r")
        #expect(await settled { pane.screenText().contains("5000") && (pane.view?.history_lines ?? 0) > 500 })
        let row = view.scroll.row

        // Fingers up the trackpad: the view follows by the point, between rows.
        await pad(0, scroll: began)
        for _ in 0..<10 { await pad(7, scroll: changed) }
        #expect(abs(view.scroll.pos - 70) < 1, "\(view.scroll.pos)")
        #expect((pane.view?.back ?? 0) > 0)
        shoot("mid-gesture")
        // Lifted with momentum, it carries on and rests on a whole row.
        await pad(0, scroll: ended)
        await pad(20, momentum: mBegan)
        for i in 0..<20 { await pad(Int32(19 - i), momentum: mGoing) }
        await pad(0, momentum: mEnded)
        await rest()
        let pos = view.scroll.pos
        #expect(pos > 200)
        #expect(abs(pos - CGFloat(view.scroll.split.back) * row) < 0.01, "rests on a row: \(pos) by \(row)")
        #expect(await settled { Int(firstLine().trimmingCharacters(in: .whitespaces)).map { $0 < 5000 } ?? false })
        let top = firstLine()
        shoot("rested")

        // New output does not move a view scrolled back.
        let live = pane.view?.live_line ?? 0
        #expect(await settled { (pane.view?.live_line ?? 0) >= live + 40 })
        #expect(firstLine() == top)
        #expect(view.scroll.scrolled)

        // Pulled past the oldest line it stretches, then springs back to it.
        await pad(0, scroll: began)
        for _ in 0..<40 { await pad(3000, scroll: changed) }
        #expect(view.scroll.pos > view.scroll.max)
        #expect(await settled { pane.view.map { $0.back == $0.history_lines } ?? false })
        shoot("stretched-at-top")
        await pad(0, scroll: ended)
        await rest()
        #expect(abs(view.scroll.pos - view.scroll.max) < 0.01)

        // Typing returns to the live screen.
        view.insertText("x", replacementRange: NSRange(location: NSNotFound, length: 0))
        #expect(!view.scroll.scrolled)
        #expect(await settled { pane.view?.back == 0 })
        pane.text("\u{7f}")

        // Pulled down at the live screen it stretches and springs back.
        await pad(0, scroll: began)
        for _ in 0..<3 { await pad(-60, scroll: changed) }
        #expect(view.scroll.pos < 0)
        shoot("stretched-at-bottom")
        await pad(0, scroll: ended)
        await rest()
        #expect(view.scroll.pos == 0)

        // A wheel notch eases three rows.
        let notch = CGEvent(scrollWheelEvent2Source: nil, units: .line, wheelCount: 1, wheel1: 1, wheel2: 0, wheel3: 0)!
        view.scrollWheel(with: NSEvent(cgEvent: notch)!)
        await rest()
        #expect(abs(view.scroll.pos - 3 * row) < 0.01)
        shoot("notch")
        view.insertText("x", replacementRange: NSRange(location: NSNotFound, length: 0))
        pane.text("\u{7f}")

        // On the alternate screen without mouse reporting, a notch is arrow keys.
        pane.text("seq 1 300 | less\r")
        #expect(await settled { pane.view?.alternate == true && firstLine().trimmingCharacters(in: .whitespaces) == "1" })
        let down = CGEvent(scrollWheelEvent2Source: nil, units: .line, wheelCount: 1, wheel1: -1, wheel2: 0, wheel3: 0)!
        view.scrollWheel(with: NSEvent(cgEvent: down)!)
        #expect(await settled { firstLine().trimmingCharacters(in: .whitespaces) == "4" })
        #expect(!view.scroll.scrolled)
        pane.text("q")
    }
}
