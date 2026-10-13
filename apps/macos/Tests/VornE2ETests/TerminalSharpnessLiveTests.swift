import AppKit
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers
import VornCore
@testable import VornTerminals

/// Draws agent output through the live terminal view in an offscreen window (never ordered in) and checks the
/// layer's backing store is as many pixels as the window's screen has, so glyphs are not scaled up.
/// `VORN_TERMINAL_REPLAY` names raw terminal output to replay (a recorded agent run, at 100 x 28);
/// `VORN_TERMINAL_SHOT` a directory for the backing store and the screen's view of it as PNGs.
@MainActor
struct TerminalSharpnessLiveTests {
    static let cols = 100, rows = 28

    @Test func liveTerminalDrawsAtTheWindowsPixelScale() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let env = ProcessInfo.processInfo.environment
        let store = VornStore(dataDir: server.dataDir, readOnly: false)
        store.start()
        defer { store.stop() }
        #expect(await until(timeout: 15) { store.phase == .connected })
        try #require(store.phase == .connected)

        let replay = server.root.appendingPathComponent("agent.ans")
        if let recorded = env["VORN_TERMINAL_REPLAY"] {
            try FileManager.default.copyItem(at: URL(fileURLWithPath: recorded), to: replay)
        } else {
            try Data(Self.sample.utf8).write(to: replay)
        }
        let shell = try await store.createShell(cwd: server.root.path)
        let engine = TerminalEngine(socket: try #require(store.endpoint?.gridSocket), readOnly: false)
        defer { engine.shutdown() }

        let window = NSWindow(contentRect: NSRect(x: -20000, y: -20000, width: 10, height: 10),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        let scale = window.backingScaleFactor
        let fonts = TerminalFonts.of(size: engine.fontSize, scale: scale)
        let size = CGSize(width: fonts.cell.width * CGFloat(Self.cols) + 2 * terminalPadX,
                          height: fonts.cell.height * CGFloat(Self.rows) + 2 * terminalPadY)
        window.setContentSize(size)
        let view = TerminalNSView(pane: engine.makePane(sessionID: shell.id))
        view.frame = CGRect(origin: .zero, size: size)
        window.contentView = view
        #expect(view.pane.size == (UInt16(Self.cols), UInt16(Self.rows)))

        await engine.settle(timeout: 8)
        try #require(view.pane.view != nil, "state \(view.pane.state) error \(engine.lastError ?? "-")")
        // The first input claims the session's size; the replay needs the width it was recorded at.
        view.pane.text("stty -echo\r")
        #expect(await until {
            await engine.settle(timeout: 0.1)
            return view.pane.view?.cols == UInt16(Self.cols)
        })
        try? await Task.sleep(for: .milliseconds(300))
        view.pane.text("clear; cat '\(replay.path)'; read\r")
        let marker = "Worked for"
        let shown = await until(timeout: 10) {
            await engine.settle(timeout: 0.1)
            view.pane.contentChanged()
            return view.pane.screenText().contains(marker)
        }
        #expect(shown, "screen: \(view.pane.screenText())")
        try? await Task.sleep(for: .milliseconds(300))
        await engine.settle(timeout: 0.5)

        var backing: CGImage?
        view.didDraw = { backing = $0.makeImage() }
        view.needsDisplay = true
        view.displayIfNeeded()
        let image = try #require(backing)
        #expect(view.layer?.contentsScale == scale)
        #expect(image.width == Int((size.width * scale).rounded()) && image.height == Int((size.height * scale).rounded()),
                "backing \(image.width)x\(image.height) for \(size) at \(scale)x")

        if let dir = env["VORN_TERMINAL_SHOT"] {
            let out = URL(fileURLWithPath: dir)
            try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
            try Self.png(image, out.appendingPathComponent("terminal-backing.png"))
            // The compositor stretches a smaller store over the window's pixels with linear filtering.
            try Self.png(try #require(Self.composite(image, width: Int(size.width * scale), height: Int(size.height * scale))),
                         out.appendingPathComponent("terminal-screen.png"))
            try view.pane.screenText().write(to: out.appendingPathComponent("terminal.txt"), atomically: true, encoding: .utf8)
        }
        try await store.close(shell.id)
    }

    static func composite(_ image: CGImage, width: Int, height: Int) -> CGImage? {
        guard let ctx = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                                  space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
        ctx.interpolationQuality = .low
        ctx.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        return ctx.makeImage()
    }

    static func png(_ image: CGImage, _ url: URL) throws {
        let dest = try #require(CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil))
        CGImageDestinationAddImage(dest, image, nil)
        #expect(CGImageDestinationFinalize(dest))
    }

    /// An agent's turn: bold and coloured tool calls, dim results, a box and a status line.
    static let sample = [
        "\u{1b}[38;2;215;119;87m╭───────────────────────────────────────────────╮\u{1b}[39m",
        "\u{1b}[38;2;215;119;87m│\u{1b}[39m \u{1b}[38;2;215;119;87m✻\u{1b}[39m Welcome back                                \u{1b}[38;2;215;119;87m│\u{1b}[39m",
        "\u{1b}[38;2;215;119;87m╰───────────────────────────────────────────────╯\u{1b}[39m",
        "",
        "\u{1b}[1m>\u{1b}[22m Read notes.md and summarise it",
        "",
        "\u{1b}[38;2;78;186;101m⏺\u{1b}[39m \u{1b}[1mRead\u{1b}[22m(notes.md)",
        "  \u{1b}[2m⎿  Read 4 lines\u{1b}[22m",
        "",
        "\u{1b}[38;2;255;255;255m⏺\u{1b}[39m The terminal now draws glyphs at the display's \u{1b}[1mpixel scale\u{1b}[22m.",
        "  Bold and \u{1b}[2mdim text\u{1b}[22m keep the weights the font was designed with.",
        "  \u{1b}[3mItalic\u{1b}[23m, \u{1b}[4munderlined\u{1b}[24m, \u{1b}[31mred\u{1b}[39m \u{1b}[32mgreen\u{1b}[39m \u{1b}[33myellow\u{1b}[39m \u{1b}[34mblue\u{1b}[39m \u{1b}[35mmagenta\u{1b}[39m \u{1b}[36mcyan\u{1b}[39m",
        "",
        "\u{1b}[2m✻ Worked for 3s\u{1b}[22m",
        "",
    ].joined(separator: "\r\n")

    @discardableResult
    private func until(timeout: TimeInterval = 8, _ check: @MainActor () async -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if await check() { return true }
            try? await Task.sleep(for: .milliseconds(100))
        }
        return false
    }
}
