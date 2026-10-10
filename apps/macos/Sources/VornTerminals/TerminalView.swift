import AppKit
import CVornGrid
import SwiftUI
import VornUI

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

func wireMods(_ f: NSEvent.ModifierFlags) -> UInt16 {
    var m: UInt16 = 0
    if f.contains(.shift) { m |= 1 }
    if f.contains(.option) { m |= 2 }
    if f.contains(.control) { m |= 4 }
    if f.contains(.command) { m |= 8 }
    return m
}

// MARK: - The view

/// A session's terminal: draws the pane's grid with CoreText and sends keys,
/// text and IME composition to vornd.
public final class TerminalNSView: NSView, NSTextInputClient, TerminalHost {
    let pane: TerminalPane
    private let painter = TerminalPainter()
    private var current: NSEvent?
    var onFocus: ((String) -> Void)?

    init(pane: TerminalPane) {
        self.pane = pane
        super.init(frame: .zero)
        wantsLayer = true
        layerContentsRedrawPolicy = .onSetNeedsDisplay
        pane.host = self
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    // A wide-gamut display gets half-float backing stores by default, where
    // blitting glyphs is several times slower; 8-bit is plenty.
    public override func makeBackingLayer() -> CALayer {
        let l = CALayer()
        l.contentsFormat = .RGBA8Uint
        l.isOpaque = true
        return l
    }

    public override var isFlipped: Bool { true }
    public override var acceptsFirstResponder: Bool { true }
    public override var isOpaque: Bool { true }

    private var fonts: TerminalFonts { pane.engine.fonts }

    func paneDidChange() { needsDisplay = true }

    public override func setFrameSize(_ s: NSSize) {
        super.setFrameSize(s)
        layoutGrid()
    }

    public override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        layoutGrid()
    }

    /// Attaches on first layout, then follows the view's size.
    private func layoutGrid() {
        guard window != nil, bounds.width > 0, bounds.height > 0 else { return }
        let g = fonts.grid(for: bounds.size)
        if pane.id == 0 {
            pane.attach(cols: g.cols, rows: g.rows)
        } else {
            pane.resize(cols: g.cols, rows: g.rows)
        }
    }

    public override func draw(_ dirty: NSRect) {
        guard let ctx = NSGraphicsContext.current?.cgContext else { return }
        painter.draw(ctx, pane.view, fonts: fonts, bounds: bounds, focused: pane.focused, marked: pane.marked)
    }

    public override func becomeFirstResponder() -> Bool {
        pane.setFocus(true)
        onFocus?(pane.sessionID)
        return true
    }

    public override func resignFirstResponder() -> Bool {
        pane.setFocus(false)
        return true
    }

    // MARK: Input

    public override func mouseDown(with e: NSEvent) {
        window?.makeFirstResponder(self)
    }

    public override func performKeyEquivalent(with e: NSEvent) -> Bool {
        // Control chords belong to the shell, except the app's own Ctrl+`.
        guard window?.firstResponder === self, e.type == .keyDown,
              e.modifierFlags.contains(.control), !e.modifierFlags.contains(.command), e.keyCode != 50
        else { return super.performKeyEquivalent(with: e) }
        keyDown(with: e)
        return true
    }

    public override func keyDown(with e: NSEvent) {
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

    private func sendKey(_ e: NSEvent, text: String?) {
        pane.key(keyCodes[e.keyCode] ?? "Unidentified", wireMods(e.modifierFlags), text)
    }

    public func insertText(_ string: Any, replacementRange: NSRange) {
        let s = (string as? NSAttributedString)?.string ?? (string as? String) ?? ""
        let composed = !pane.marked.isEmpty
        if composed { pane.setMarked("") }
        if let e = current, !composed, s == e.characters {
            sendKey(e, text: s)
        } else {
            pane.text(s)
        }
    }

    public override func doCommand(by selector: Selector) {
        if let e = current { sendKey(e, text: nil) }
    }

    public func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        pane.setMarked((string as? NSAttributedString)?.string ?? (string as? String) ?? "")
    }

    public func unmarkText() { pane.setMarked("") }
    public func selectedRange() -> NSRange { NSRange(location: 0, length: 0) }
    public func markedRange() -> NSRange {
        pane.marked.isEmpty ? NSRange(location: NSNotFound, length: 0)
            : NSRange(location: 0, length: (pane.marked as NSString).length)
    }
    public func hasMarkedText() -> Bool { !pane.marked.isEmpty }
    public func attributedSubstring(forProposedRange r: NSRange, actualRange: NSRangePointer?) -> NSAttributedString? { nil }
    public func validAttributesForMarkedText() -> [NSAttributedString.Key] { [] }
    public func characterIndex(for point: NSPoint) -> Int { 0 }
    public func firstRect(forCharacterRange r: NSRange, actualRange: NSRangePointer?) -> NSRect {
        let local = pane.cursorRect(fonts: fonts)
        guard let w = window else { return local }
        return w.convertToScreen(convert(local, to: nil))
    }

    // MARK: Accessibility: the screen as a text area.

    public override func isAccessibilityElement() -> Bool { true }
    public override func accessibilityRole() -> NSAccessibility.Role? { .textArea }
    public override func accessibilityLabel() -> String? { "Terminal" }
    public override func accessibilityValue() -> Any? { pane.screenText() }
}

// MARK: - SwiftUI

/// Whether terminals draw as static snapshots (offscreen renders, where an NSView cannot draw).
public struct TerminalSnapshotKey: EnvironmentKey {
    public static let defaultValue = false
}

public extension EnvironmentValues {
    var terminalSnapshot: Bool {
        get { self[TerminalSnapshotKey.self] }
        set { self[TerminalSnapshotKey.self] = newValue }
    }
}

/// A live session's terminal, or a drawing of its last frame in snapshot mode.
public struct TerminalSurface: View {
    let engine: TerminalEngine
    let sessionID: String
    var focusRequest: Int
    var onFocus: (String) -> Void
    @Environment(\.terminalSnapshot) private var snapshot

    public init(engine: TerminalEngine, sessionID: String, focusRequest: Int = 0, onFocus: @escaping (String) -> Void = { _ in }) {
        self.engine = engine
        self.sessionID = sessionID
        self.focusRequest = focusRequest
        self.onFocus = onFocus
    }

    public var body: some View {
        if snapshot {
            Canvas { gc, size in
                let pane = engine.snapshotPane(sessionID)
                gc.withCGContext { ctx in
                    TerminalPainter().draw(ctx, pane?.view, fonts: engine.fonts,
                                           bounds: CGRect(origin: .zero, size: size), focused: false, marked: "")
                }
            }
        } else {
            TerminalRepresentable(engine: engine, sessionID: sessionID, focusRequest: focusRequest, onFocus: onFocus)
        }
    }
}

struct TerminalRepresentable: NSViewRepresentable {
    let engine: TerminalEngine
    let sessionID: String
    let focusRequest: Int
    let onFocus: (String) -> Void

    final class Coordinator {
        var focusRequest = 0
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> TerminalNSView {
        let v = TerminalNSView(pane: engine.makePane(sessionID: sessionID))
        v.onFocus = onFocus
        context.coordinator.focusRequest = focusRequest
        if focusRequest > 0 {
            DispatchQueue.main.async { v.window?.makeFirstResponder(v) }
        }
        return v
    }

    func updateNSView(_ v: TerminalNSView, context: Context) {
        v.onFocus = onFocus
        if focusRequest != context.coordinator.focusRequest {
            context.coordinator.focusRequest = focusRequest
            DispatchQueue.main.async { v.window?.makeFirstResponder(v) }
        }
    }

    static func dismantleNSView(_ v: TerminalNSView, coordinator: Coordinator) {
        v.pane.detach()
    }
}
