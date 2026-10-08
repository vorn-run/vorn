// macOS: an AppKit window hosting the SwiftUI content; each pane is an
// NSView that owns input (NSTextInputClient), accessibility, and either
// draws itself with CoreText (option A) or hands itself to the Rust GPU
// renderer, which adds its own Metal layer (option B). ⌥⌘R switches the
// focused pane between the two.

import AppKit
import SwiftUI

// MARK: - Keys

/// macOS virtual key codes to W3C `code` names (the wire's key names).
let keyCodes: [UInt16: String] = {
    var m: [UInt16: String] = [
        36: "Enter", 48: "Tab", 49: "Space", 51: "Backspace", 53: "Escape", 117: "Delete",
        115: "Home", 119: "End", 116: "PageUp", 121: "PageDown", 123: "ArrowLeft",
        124: "ArrowRight", 125: "ArrowDown", 126: "ArrowUp", 24: "Equal", 27: "Minus",
        30: "BracketRight", 33: "BracketLeft", 39: "Quote", 41: "Semicolon", 42: "Backslash",
        43: "Comma", 44: "Slash", 47: "Period", 50: "Backquote", 76: "NumpadEnter",
        122: "F1", 120: "F2", 99: "F3", 118: "F4", 96: "F5", 97: "F6", 98: "F7", 100: "F8",
        101: "F9", 109: "F10", 103: "F11", 111: "F12",
    ]
    let letters: [(UInt16, String)] = [
        (0, "A"), (11, "B"), (8, "C"), (2, "D"), (14, "E"), (3, "F"), (5, "G"), (4, "H"),
        (34, "I"), (38, "J"), (40, "K"), (37, "L"), (46, "M"), (45, "N"), (31, "O"), (35, "P"),
        (12, "Q"), (15, "R"), (1, "S"), (17, "T"), (32, "U"), (9, "V"), (13, "W"), (7, "X"),
        (16, "Y"), (6, "Z"),
    ]
    for (k, l) in letters { m[k] = "Key" + l }
    let digits: [UInt16] = [29, 18, 19, 20, 21, 23, 22, 26, 28, 25]
    for (d, k) in digits.enumerated() { m[k] = "Digit\(d)" }
    return m
}()

let letterKey: [Character: UInt16] = Dictionary(uniqueKeysWithValues:
    keyCodes.compactMap { k, v in
        v.hasPrefix("Key") ? (Character(v.dropFirst(3).lowercased()), k) : nil
    })

func wireMods(_ f: NSEvent.ModifierFlags) -> UInt16 {
    var m: UInt16 = 0
    if f.contains(.shift) { m |= 1 }
    if f.contains(.option) { m |= 2 }
    if f.contains(.control) { m |= 4 }
    if f.contains(.command) { m |= 8 }
    return m
}

// MARK: - A pane

final class TermPaneView: NSView, NSTextInputClient, PaneHostView {
    let core: PaneCore
    var current: NSEvent?

    init(core: PaneCore) {
        self.core = core
        super.init(frame: .zero)
        wantsLayer = true
        layerContentsRedrawPolicy = .onSetNeedsDisplay
        core.host = self
    }

    required init?(coder: NSCoder) { fatalError() }

    // A wide-gamut display gets half-float backing stores by default, where
    // blitting glyphs costs about 7 ms a pane; 8-bit is plenty.
    override func makeBackingLayer() -> CALayer {
        let l = CALayer()
        l.contentsFormat = .RGBA8Uint
        l.isOpaque = true
        return l
    }

    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override var isOpaque: Bool { true }

    var paneSize: CGSize { bounds.size }
    var scale: CGFloat { window?.backingScaleFactor ?? 2 }
    var handle: UnsafeMutableRawPointer { Unmanaged.passUnretained(self).toOpaque() }
    func redraw() { if core.gpu == nil { display() } }

    override func setFrameSize(_ s: NSSize) {
        super.setFrameSize(s)
        if window != nil { core.layout() }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard let w = window else { return }
        core.layout()
        // SwiftUI adds the views after the window is set up: the first pane
        // takes the focus once it is in the window.
        if core.index == 0 && w.firstResponder === w { w.makeFirstResponder(self) }
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        core.scaleChanged()
    }

    override func draw(_ dirty: NSRect) {
        guard core.gpu == nil, let ctx = NSGraphicsContext.current?.cgContext else { return }
        core.painter.draw(ctx, core.hasView ? core.view : nil, bounds: bounds,
                          focused: core.focused, marked: core.marked)
    }

    override func becomeFirstResponder() -> Bool {
        core.setFocus(true)
        return true
    }

    override func resignFirstResponder() -> Bool {
        core.setFocus(false)
        return true
    }

    // MARK: Input

    override func mouseDown(with e: NSEvent) {
        window?.makeFirstResponder(self)
    }

    override func keyDown(with e: NSEvent) {
        if e.modifierFlags.contains([.command, .option]) && e.keyCode == 15 {
            core.use(core.gpu == nil ? 1 : 0)
            return
        }
        // A synthesized event in a window that is not key has no input
        // context to compose with: send it as the key it is.
        if inputContext == nil || window?.isKeyWindow != true {
            sendKey(e, text: e.characters)
            return
        }
        current = e
        interpretKeyEvents([e])
        current = nil
    }

    func sendKey(_ e: NSEvent, text: String?) {
        core.key(keyCodes[e.keyCode] ?? "Unidentified", wireMods(e.modifierFlags), text)
    }

    func insertText(_ string: Any, replacementRange: NSRange) {
        let s = (string as? NSAttributedString)?.string ?? (string as? String) ?? ""
        let composed = !core.marked.isEmpty
        if composed { core.setMarked("") }
        if let e = current, !composed, s == e.characters {
            sendKey(e, text: s)
        } else {
            core.text(s)
        }
    }

    override func doCommand(by selector: Selector) {
        if let e = current { sendKey(e, text: nil) }
    }

    func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        core.setMarked((string as? NSAttributedString)?.string ?? (string as? String) ?? "")
    }

    func unmarkText() { core.setMarked("") }
    func selectedRange() -> NSRange { NSRange(location: core.cursorIndex, length: 0) }
    func markedRange() -> NSRange {
        core.marked.isEmpty ? NSRange(location: NSNotFound, length: 0)
            : NSRange(location: 0, length: (core.marked as NSString).length)
    }
    func hasMarkedText() -> Bool { !core.marked.isEmpty }
    func attributedSubstring(forProposedRange r: NSRange, actualRange: NSRangePointer?) -> NSAttributedString? { nil }
    func validAttributesForMarkedText() -> [NSAttributedString.Key] { [] }
    func characterIndex(for point: NSPoint) -> Int { 0 }
    func firstRect(forCharacterRange r: NSRange, actualRange: NSRangePointer?) -> NSRect {
        let local = core.cursorRect()
        guard let w = window else { return local }
        return w.convertToScreen(convert(local, to: nil))
    }

    // MARK: Accessibility: the pane reads as a text area holding its screen,
    // with the insertion point on the cursor's line.

    override func isAccessibilityElement() -> Bool { true }
    override func accessibilityRole() -> NSAccessibility.Role? { .textArea }
    override func accessibilityLabel() -> String? { "Terminal \(core.index + 1)" }
    override func accessibilityValue() -> Any? { core.screenText() }
    override func accessibilityNumberOfCharacters() -> Int { (core.screenText() as NSString).length }
    override func accessibilityInsertionPointLineNumber() -> Int { core.cursorLine }
    override func accessibilitySelectedTextRange() -> NSRange { NSRange(location: core.cursorIndex, length: 0) }
    override func accessibilitySelectedText() -> String? { "" }
    override func accessibilityLine(for index: Int) -> Int { core.line(at: index) }
    override func accessibilityRange(forLine line: Int) -> NSRange { core.range(ofLine: line) }
    override func accessibilityString(for r: NSRange) -> String? {
        let s = core.screenText() as NSString
        guard r.location + r.length <= s.length else { return nil }
        return s.substring(with: r)
    }
    override func accessibilityVisibleCharacterRange() -> NSRange {
        NSRange(location: 0, length: (core.screenText() as NSString).length)
    }
    override func accessibilityFrame(for r: NSRange) -> NSRect {
        let line = core.line(at: r.location)
        let c = core.cursorRect()
        let local = NSRect(x: padX, y: padY + CGFloat(line) * c.height, width: bounds.width - 2 * padX, height: c.height)
        return window?.convertToScreen(convert(local, to: nil)) ?? local
    }
}

struct PaneRep: NSViewRepresentable {
    let view: TermPaneView
    func makeNSView(context: Context) -> TermPaneView { view }
    func updateNSView(_ v: TermPaneView, context: Context) {}
}

// MARK: - The app

/// Keeps the size asked for: AppKit would shrink a 900-point window that
/// reaches the dock on a smaller display.
final class BenchWindow: NSWindow {
    override func constrainFrameRect(_ r: NSRect, to screen: NSScreen?) -> NSRect { r }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    var window: NSWindow!
    var driver: Driver?
    var views: [TermPaneView] = []
    var activity: NSObjectProtocol?
    var demo: Timer?
    var backdrop: NSWindow?

    func applicationDidFinishLaunching(_ n: Notification) {
        activity = ProcessInfo.processInfo.beginActivity(
            options: [.userInitiated, .latencyCritical], reason: "terminal frames")
        // VORN_SPIKE_WINDOW=WxH (the recordings use a smaller window that clears the Dock).
        let wh = (env["VORN_SPIKE_WINDOW"] ?? "1440x900").split(separator: "x").compactMap { Double($0) }
        let size = wh.count == 2 ? NSSize(width: wh[0], height: wh[1]) : NSSize(width: 1440, height: 900)
        let sessions = (env["VORN_SPIKE_SESSIONS"] ?? "").split(separator: ",").count
        let (cols, rows) = layout(max(1, sessions))
        let paneW = look ? 742 : (size.width - CGFloat(cols - 1) * 2) / CGFloat(cols)
        let paneH = look ? 900 - 41 - 3 : (size.height - CGFloat(rows - 1) * 2) / CGFloat(rows)
        let g = gridSize(CGSize(width: paneW, height: paneH))
        guard let h = vs_open(g.0, g.1) else {
            FileHandle.standardError.write("no grid endpoint (VORN_SPIKE_GRID/VORN_SPIKE_SESSIONS)\n".data(using: .utf8)!)
            exit(1)
        }
        let model = Model(h: h)
        views = model.panes.map { TermPaneView(core: $0) }
        var style: NSWindow.StyleMask = [.titled, .closable, .resizable, .miniaturizable]
        if look { style.insert(.fullSizeContentView) }
        window = BenchWindow(contentRect: NSRect(origin: .zero, size: size),
                             styleMask: style, backing: .buffered, defer: false)
        window.title = "Vorn spike: Apple (\(model.clientName))"
        if look {
            window.titlebarAppearsTransparent = true
            window.titleVisibility = .hidden
            window.appearance = NSAppearance(named: .darkAqua)
            if polish {
                window.isOpaque = false
                window.backgroundColor = .clear
            }
            window.contentView = NSHostingView(rootView: LookView(term: PaneRep(view: views[0]), polish: polish))
            window.setFrame(NSRect(origin: .zero, size: size), display: false)
        } else {
            window.contentView = NSHostingView(rootView: GridView(panes: views.map { PaneRep(view: $0) }, columns: cols))
        }
        window.setFrameTopLeftPoint(NSPoint(x: 40, y: (NSScreen.main?.frame.maxY ?? 940) - 40))
        if model.mode != 0 || env["VORN_SPIKE_DEMO"] != nil { window.level = .floating }
        window.orderFrontRegardless()
        if model.mode == 0 { NSApp.activate() }

        let d = Driver(model: model)
        let link = window.contentView!.displayLink(target: d, selector: #selector(Driver.frame(_:)))
        d.inject = { [weak self] c in self?.inject(c) }
        d.finish = { NSApp.terminate(nil) }
        d.start(link: link, fps: NSScreen.main?.maximumFramesPerSecond ?? 60)
        driver = d
        startDemo()
    }

    /// A key typed into pane 0 the way AppKit delivers one: an NSEvent
    /// through the window to its first responder.
    func inject(_ c: Character) {
        let code: UInt16 = c == "\r" ? 36 : letterKey[c] ?? 0
        guard let e = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [],
                                       timestamp: ProcessInfo.processInfo.systemUptime,
                                       windowNumber: window.windowNumber, context: nil,
                                       characters: String(c), charactersIgnoringModifiers: String(c),
                                       isARepeat: false, keyCode: code) else { return }
        window.sendEvent(e)
    }

    /// VORN_SPIKE_DEMO, for the recordings: "resize" animates the window's
    /// width, "focus" moves the focus to the next pane every second.
    func startDemo() {
        guard let mode = env["VORN_SPIKE_DEMO"] else { return }
        // An opaque backdrop over the recorded region, so a shrinking window
        // never reveals what is behind it.
        let b = NSWindow(contentRect: window.frame, styleMask: .borderless, backing: .buffered, defer: false)
        b.backgroundColor = .black
        b.level = .floating
        b.ignoresMouseEvents = true
        b.order(.below, relativeTo: window.windowNumber)
        backdrop = b
        switch mode {
        case "resize":
            let t0 = CACurrentMediaTime()
            let top = window.frame.maxY
            let full = window.frame.size
            demo = Timer.scheduledTimer(withTimeInterval: 1.0 / 60, repeats: true) { [weak self] _ in
                guard let w = self?.window else { return }
                let t = CACurrentMediaTime() - t0
                let width = full.width * (0.6 + 0.4 * (0.5 + 0.5 * cos(t * 2 * .pi / 4)))
                let height = full.height * (0.6 + 0.4 * (0.5 + 0.5 * cos(t * 2 * .pi / 5)))
                w.setFrame(NSRect(x: 40, y: top - height, width: width, height: height), display: true)
            }
        case "ime":
            // For the IME test (apple/imectl.swift types through the system
            // Japanese input method): pane 1 takes focus for the run, which
            // the input method needs, and hands it back at the end.
            guard let v = views.first else { return }
            let previous = NSWorkspace.shared.frontmostApplication
            NSApp.activate(ignoringOtherApps: true)
            window.makeKeyAndOrderFront(nil)
            window.makeFirstResponder(v)
            demo = Timer.scheduledTimer(withTimeInterval: 8, repeats: false) { _ in previous?.activate() }
        case "focus":
            var i = 0
            demo = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
                guard let self, !self.views.isEmpty else { return }
                i = (i + 1) % self.views.count
                self.window.makeFirstResponder(self.views[i])
                self.views[i].insertText("echo focus moved to pane \(i + 1)\r", replacementRange: NSRange(location: NSNotFound, length: 0))
            }
        default: break
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ a: NSApplication) -> Bool { true }
}

@main
enum Main {
    static func main() {
        let app = NSApplication.shared
        app.setActivationPolicy(.regular)
        let delegate = AppDelegate()
        app.delegate = delegate
        withExtendedLifetime(delegate) { app.run() }
    }
}
