import AppKit
import Foundation
import CVornGrid
import Testing
import VornCore
@testable import VornTerminals

/// Copying a selection out of a live shell, double-click words, and pasting, against a test vornd.
@MainActor
struct TerminalSelectionLiveTests {
    @Test func selectsCopiesAndPastes() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let store = VornStore(dataDir: server.dataDir, readOnly: false)
        store.start()
        defer { store.stop() }
        #expect(await until(timeout: 15) { store.phase == .connected })
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

        func settled(_ check: () -> Bool) async -> Bool {
            await until(timeout: 10) {
                await engine.settle(timeout: 0.1)
                pane.contentChanged()
                pane.refresh()
                return check()
            }
        }

        pane.text("stty -echo; clear; for i in $(seq 1 300); do echo row$i; done\r")
        #expect(await settled { pane.screenText().contains("row300") })
        let v = try #require(pane.view)
        // Two whole rows, as a drag from the first column of one to the first of the row after the next.
        var copied = ""
        var top = v.first_line
        #expect(await settled {
            for line in v.first_line..<(v.first_line + UInt64(v.rows)) {
                let t = pane.text(of: TerminalSelection(GridPoint(line: line, col: 0), GridPoint(line: line + 2, col: 0)))
                if t.hasPrefix("row") { (copied, top) = (t, line); return true }
            }
            return false
        })
        let rows = copied.split(separator: "\n").map(String.init)
        let n = try #require(rows.first.flatMap { Int($0.dropFirst(3)) })
        #expect(rows == ["row\(n)", "row\(n + 1)"], "copied \(copied.debugDescription)")
        #expect(pane.word(at: GridPoint(line: top, col: 1)) == TerminalSelection(GridPoint(line: top, col: 0),
                                                                                   GridPoint(line: top, col: UInt16(rows[0].count))))
        // Without history kept by vornd, scrolling up stays on the live screen.
        pane.scrollTo(10)
        #expect(await settled { pane.view?.first_line == v.first_line })

        // A paste arrives as typed lines, the live screen showing.
        pane.paste("echo pasted-ok\n")
        #expect(await settled { pane.screenText().contains("pasted-ok") })
        #expect(await settled { pane.view.map { $0.screen & UInt8(VG_SCROLLED) == 0 } ?? false })
    }

    private func until(timeout: TimeInterval = 10, _ check: @MainActor () async -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if await check() { return true }
            try? await Task.sleep(for: .milliseconds(100))
        }
        return false
    }
}
