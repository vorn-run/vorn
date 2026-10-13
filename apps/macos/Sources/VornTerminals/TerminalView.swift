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
public final class TerminalNSView: NSView, NSTextInputClient, NSMenuItemValidation, TerminalHost {
    let pane: TerminalPane
    private let painter = TerminalPainter()
    private var current: NSEvent?
    var onFocus: ((String) -> Void)?
    /// Sees the backing store's context after each draw; for tests.
    var didDraw: ((CGContext) -> Void)?

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

    private var fonts: TerminalFonts {
        TerminalFonts.of(size: pane.engine.fontSize, scale: window?.backingScaleFactor ?? 2)
    }

    func paneDidChange() { needsDisplay = true }

    public override func setFrameSize(_ s: NSSize) {
        super.setFrameSize(s)
        layoutGrid()
    }

    public override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        matchBackingScale()
        layoutGrid()
    }

    public override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        matchBackingScale()
        layoutGrid()
        needsDisplay = true
    }

    /// AppKit leaves a custom backing layer at 1x; the store follows the window's screen instead.
    private func matchBackingScale() {
        guard let layer, let scale = window?.backingScaleFactor, layer.contentsScale != scale else { return }
        layer.contentsScale = scale
        needsDisplay = true
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
        painter.draw(ctx, pane.view, fonts: fonts, bounds: bounds, focused: pane.focused, marked: pane.marked,
                     selection: pane.selection)
        didDraw?(ctx)
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

    /// How a drag grows the selection: by cell, word (double click) or line (triple).
    private enum Unit { case cell, word, line }
    private var unit = Unit.cell
    /// What the drag started on; the selection always keeps it.
    private var anchor: TerminalSelection?
    /// The press went to a program tracking the mouse.
    private var tracking = false
    private var wheelRest: CGFloat = 0

    /// Where an event is: the cell fraction the program hears, and the absolute
    /// cell boundary nearest it, kept on the screen.
    private func cell(at e: NSEvent) -> (x: Float, y: Float, point: GridPoint)? {
        guard let v = pane.view else { return nil }
        let p = convert(e.locationInWindow, from: nil)
        let x = max(0, (p.x - terminalPadX) / fonts.cell.width)
        let y = (p.y - terminalPadY) / fonts.cell.height
        let row = min(max(Int(y.rounded(.down)), 0), Int(v.rows) - 1)
        let col = min(UInt16(x.rounded()), v.cols)
        return (Float(x), Float(max(0, y)), GridPoint(line: v.first_line + UInt64(row), col: col))
    }

    private var tracksMouse: Bool { pane.view?.mouse == 1 }

    public override func mouseDown(with e: NSEvent) {
        window?.makeFirstResponder(self)
        guard let (x, y, p) = cell(at: e) else { return }
        // Option forces a selection over a program that tracks the mouse.
        tracking = tracksMouse && !e.modifierFlags.contains(.option)
        if tracking {
            pane.selection = nil
            pane.mouse(0, button: 1, mods: wireMods(e.modifierFlags), x: x, y: y)
            return
        }
        switch e.clickCount {
        case 2:
            unit = .word
            anchor = pane.word(at: p)
        case 3...:
            unit = .line
            anchor = line(p.line)
        default:
            unit = .cell
            // Shift-click moves the selection's nearer end to the click.
            if e.modifierFlags.contains(.shift), let s = pane.selection {
                let kept = p < s.from ? s.to : s.from
                anchor = TerminalSelection(kept, kept)
                pane.selection = TerminalSelection(kept, p)
                return
            }
            anchor = TerminalSelection(p, p)
        }
        pane.selection = anchor
    }

    public override func mouseDragged(with e: NSEvent) {
        guard let (x, y, p) = cell(at: e) else { return }
        if tracking {
            pane.mouse(2, button: 1, mods: wireMods(e.modifierFlags), x: x, y: y)
            return
        }
        guard let anchor, let v = pane.view else { return }
        // Dragging past an edge scrolls history a line at a time.
        let local = convert(e.locationInWindow, from: nil).y
        if local < 0 { pane.scroll(-1) } else if local > bounds.height { pane.scroll(1) }
        let at: TerminalSelection
        switch unit {
        case .cell: at = TerminalSelection(p, p)
        case .word: at = pane.word(at: p)
        case .line: at = line(p.line, cols: v.cols)
        }
        pane.selection = at.from < anchor.from ? TerminalSelection(at.from, anchor.to) : TerminalSelection(anchor.from, at.to)
    }

    public override func mouseUp(with e: NSEvent) {
        if tracking, let (x, y, _) = cell(at: e) {
            pane.mouse(1, button: 1, mods: wireMods(e.modifierFlags), x: x, y: y)
        }
        tracking = false
        if pane.selection?.isEmpty == true { pane.selection = nil }
    }

    private func line(_ l: UInt64, cols: UInt16? = nil) -> TerminalSelection {
        TerminalSelection(GridPoint(line: l, col: 0), GridPoint(line: l, col: cols ?? pane.view?.cols ?? 0))
    }

    public override func scrollWheel(with e: NSEvent) {
        guard let (x, y, _) = cell(at: e), let v = pane.view else { return super.scrollWheel(with: e) }
        // A trackpad reports points, a wheel notches; a notch moves three lines.
        wheelRest += e.hasPreciseScrollingDeltas ? e.scrollingDeltaY / fonts.cell.height : e.scrollingDeltaY * 3
        let lines = Int32(wheelRest.rounded(.towardZero))
        guard lines != 0 else { return }
        wheelRest -= CGFloat(lines)
        // Down at the live screen with nothing to take it: the page around scrolls instead.
        let local = v.mouse == 0 && v.screen & UInt8(VG_ALTERNATE) == 0
        if local, lines < 0, v.screen & UInt8(VG_SCROLLED) == 0 {
            wheelRest = 0
            return super.scrollWheel(with: e)
        }
        pane.wheel(-lines, x: x, y: y, mods: wireMods(e.modifierFlags))
    }

    // MARK: Copy and paste

    @objc func copy(_ sender: Any?) {
        guard let s = pane.selection, !s.isEmpty else { return }
        let text = pane.text(of: s)
        guard !text.isEmpty else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    @objc func paste(_ sender: Any?) {
        guard let text = NSPasteboard.general.string(forType: .string), !text.isEmpty else { return }
        pane.selection = nil
        pane.paste(text)
    }

    public override func selectAll(_ sender: Any?) {
        pane.selection = TerminalSelection(GridPoint(line: 0, col: 0), GridPoint(line: .max, col: .max))
    }

    @objc func fitToWindow(_ sender: Any?) { pane.takeSize() }

    public override func menu(for event: NSEvent) -> NSMenu? {
        window?.makeFirstResponder(self)
        let m = NSMenu()
        m.addItem(withTitle: "Copy", action: #selector(copy(_:)), keyEquivalent: "")
        m.addItem(withTitle: "Paste", action: #selector(paste(_:)), keyEquivalent: "")
        m.addItem(.separator())
        m.addItem(withTitle: "Fit to This Window", action: #selector(fitToWindow(_:)), keyEquivalent: "")
        for item in m.items { item.target = self }
        return m
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
        pane.selection = nil
        // Shift-Page Up and Down page through history, as the shell never sees them.
        if e.modifierFlags.contains(.shift), e.keyCode == 116 || e.keyCode == 121,
           let v = pane.view, v.screen & UInt8(VG_ALTERNATE) == 0 {
            let page = Int32(max(Int(v.rows) - 1, 1))
            pane.scroll(e.keyCode == 116 ? -page : page)
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

    public func validateMenuItem(_ item: NSMenuItem) -> Bool {
        switch item.action {
        case #selector(copy(_:)): return pane.selection.map { !$0.isEmpty } ?? false
        case #selector(paste(_:)): return !pane.engine.readOnly && NSPasteboard.general.string(forType: .string) != nil
        case #selector(fitToWindow(_:)): return !pane.engine.readOnly
        default: return true
        }
    }

    // MARK: Accessibility: the screen as a text area.

    public override func isAccessibilityElement() -> Bool { true }
    public override func accessibilityRole() -> NSAccessibility.Role? { .textArea }
    public override func accessibilityLabel() -> String? { "Terminal" }
    public override func accessibilityValue() -> Any? { pane.screenText() }
}

// MARK: - SwiftUI

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
                .id(ObjectIdentifier(engine))
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
