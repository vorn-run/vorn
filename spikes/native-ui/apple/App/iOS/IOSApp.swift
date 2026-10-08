// iOS and iPadOS: a SwiftUI app whose panes are UIViews that own input
// (UITextInput, with the IME's composition drawn by the pane's renderer),
// accessibility, and either draw themselves with CoreText (option A) or
// hand themselves to the Rust GPU renderer (option B).

import SwiftUI
import UIKit

// MARK: - Text positions: the pane's "document" is the IME's composition

final class Pos: UITextPosition {
    let i: Int
    init(_ i: Int) { self.i = i }
}

final class Span: UITextRange {
    let a: Int, b: Int
    init(_ a: Int, _ b: Int) { self.a = a; self.b = b }
    override var start: UITextPosition { Pos(a) }
    override var end: UITextPosition { Pos(b) }
    override var isEmpty: Bool { a == b }
}

/// UIKey HID usages to W3C `code` names, for hardware keyboards.
func code(for k: UIKey) -> String? {
    switch k.keyCode {
    case .keyboardReturnOrEnter: return "Enter"
    case .keyboardTab: return "Tab"
    case .keyboardDeleteOrBackspace: return "Backspace"
    case .keyboardEscape: return "Escape"
    case .keyboardLeftArrow: return "ArrowLeft"
    case .keyboardRightArrow: return "ArrowRight"
    case .keyboardUpArrow: return "ArrowUp"
    case .keyboardDownArrow: return "ArrowDown"
    case .keyboardHome: return "Home"
    case .keyboardEnd: return "End"
    case .keyboardPageUp: return "PageUp"
    case .keyboardPageDown: return "PageDown"
    case .keyboardDeleteForward: return "Delete"
    default: return nil
    }
}

final class TermPaneView: UIView, UITextInput, PaneHostView {
    let core: PaneCore

    init(core: PaneCore) {
        self.core = core
        super.init(frame: .zero)
        isOpaque = true
        contentMode = .redraw
        isAccessibilityElement = true
        accessibilityTraits = [.staticText, .causesPageTurn]
        core.host = self
        addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(tap)))
    }

    required init?(coder: NSCoder) { fatalError() }

    var paneSize: CGSize { bounds.size }
    var scale: CGFloat { window?.screen.scale ?? traitCollection.displayScale }
    var handle: UnsafeMutableRawPointer { Unmanaged.passUnretained(self).toOpaque() }
    func redraw() { if core.gpu == nil { setNeedsDisplay(); layer.displayIfNeeded() } }

    override func layoutSubviews() {
        super.layoutSubviews()
        if window != nil { core.layout() }
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window != nil { core.layout() }
    }

    override func draw(_ r: CGRect) {
        guard core.gpu == nil, let ctx = UIGraphicsGetCurrentContext() else { return }
        core.painter.draw(ctx, core.hasView ? core.view : nil, bounds: bounds,
                          focused: core.focused, marked: core.marked)
    }

    @objc func tap() { becomeFirstResponder() }

    override var canBecomeFirstResponder: Bool { true }

    override func becomeFirstResponder() -> Bool {
        let ok = super.becomeFirstResponder()
        if ok { core.setFocus(true) }
        return ok
    }

    override func resignFirstResponder() -> Bool {
        let ok = super.resignFirstResponder()
        if ok { core.setFocus(false) }
        return ok
    }

    // MARK: Hardware keys that carry no text

    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        var rest = Set<UIPress>()
        for p in presses {
            if core.marked.isEmpty, let k = p.key, let c = code(for: k) {
                core.key(c, wireMods(k.modifierFlags), nil)
            } else {
                rest.insert(p)
            }
        }
        if !rest.isEmpty { super.pressesBegan(rest, with: event) }
    }

    func wireMods(_ f: UIKeyModifierFlags) -> UInt16 {
        (f.contains(.shift) ? 1 : 0) | (f.contains(.alternate) ? 2 : 0)
            | (f.contains(.control) ? 4 : 0) | (f.contains(.command) ? 8 : 0)
    }

    // MARK: UIKeyInput

    var hasText: Bool { true }
    func insertText(_ s: String) {
        if !core.marked.isEmpty { core.setMarked("") }
        if s == "\n" { core.key("Enter", 0, "\r") } else { core.text(s) }
    }
    func deleteBackward() { core.key("Backspace", 0, nil) }

    // MARK: UITextInput: only the composition is editable

    var autocorrectionType: UITextAutocorrectionType = .no
    var autocapitalizationType: UITextAutocapitalizationType = .none
    var spellCheckingType: UITextSpellCheckingType = .no
    var smartQuotesType: UITextSmartQuotesType = .no
    var smartDashesType: UITextSmartDashesType = .no
    var keyboardType: UIKeyboardType = .asciiCapable
    var markedTextStyle: [NSAttributedString.Key: Any]?
    var inputDelegate: UITextInputDelegate?
    lazy var tokenizer: UITextInputTokenizer = UITextInputStringTokenizer(textInput: self)
    var markedSel = 0

    var len: Int { (core.marked as NSString).length }

    func text(in r: UITextRange) -> String? {
        guard let r = r as? Span else { return nil }
        let s = core.marked as NSString
        let a = min(r.a, s.length), b = min(r.b, s.length)
        return s.substring(with: NSRange(location: a, length: max(0, b - a)))
    }
    func replace(_ r: UITextRange, withText t: String) { insertText(t) }
    var selectedTextRange: UITextRange? {
        get { Span(markedSel, markedSel) }
        set { markedSel = (newValue as? Span)?.a ?? 0 }
    }
    var markedTextRange: UITextRange? { core.marked.isEmpty ? nil : Span(0, len) }
    func setMarkedText(_ t: String?, selectedRange: NSRange) {
        markedSel = selectedRange.location
        core.setMarked(t ?? "")
    }
    func setAttributedMarkedText(_ t: NSAttributedString?, selectedRange: NSRange) {
        setMarkedText(t?.string, selectedRange: selectedRange)
    }
    func unmarkText() {
        let s = core.marked
        core.setMarked("")
        if !s.isEmpty { core.text(s) }
    }
    var beginningOfDocument: UITextPosition { Pos(0) }
    var endOfDocument: UITextPosition { Pos(len) }
    func textRange(from a: UITextPosition, to b: UITextPosition) -> UITextRange? {
        Span((a as! Pos).i, (b as! Pos).i)
    }
    func position(from p: UITextPosition, offset: Int) -> UITextPosition? {
        let i = (p as! Pos).i + offset
        return i < 0 || i > len ? nil : Pos(i)
    }
    func position(from p: UITextPosition, in d: UITextLayoutDirection, offset: Int) -> UITextPosition? {
        position(from: p, offset: d == .left || d == .up ? -offset : offset)
    }
    func compare(_ a: UITextPosition, to b: UITextPosition) -> ComparisonResult {
        let x = (a as! Pos).i, y = (b as! Pos).i
        return x < y ? .orderedAscending : x > y ? .orderedDescending : .orderedSame
    }
    func offset(from a: UITextPosition, to b: UITextPosition) -> Int { (b as! Pos).i - (a as! Pos).i }
    func position(within r: UITextRange, farthestIn d: UITextLayoutDirection) -> UITextPosition? {
        d == .left || d == .up ? r.start : r.end
    }
    func characterRange(byExtending p: UITextPosition, in d: UITextLayoutDirection) -> UITextRange? {
        Span((p as! Pos).i, d == .left || d == .up ? 0 : len)
    }
    func baseWritingDirection(for p: UITextPosition, in d: UITextStorageDirection) -> NSWritingDirection { .leftToRight }
    func setBaseWritingDirection(_ w: NSWritingDirection, for r: UITextRange) {}
    func firstRect(for r: UITextRange) -> CGRect { core.cursorRect() }
    func caretRect(for p: UITextPosition) -> CGRect {
        var c = core.cursorRect()
        c.size.width = 2
        return c
    }
    func selectionRects(for r: UITextRange) -> [UITextSelectionRect] { [] }
    func closestPosition(to p: CGPoint) -> UITextPosition? { Pos(len) }
    func closestPosition(to p: CGPoint, within r: UITextRange) -> UITextPosition? { r.end }
    func characterRange(at p: CGPoint) -> UITextRange? { nil }

    // MARK: Accessibility: the screen's text, line by line

    override var accessibilityLabel: String? {
        get { "Terminal \(core.index + 1)" }
        set {}
    }
    override var accessibilityValue: String? {
        get { core.lines.indices.contains(core.cursorLine) ? String(core.lines[core.cursorLine]) : nil }
        set {}
    }
}

extension TermPaneView: UIAccessibilityReadingContent {
    func accessibilityLineNumber(for point: CGPoint) -> Int {
        max(0, Int((point.y - padY) / core.cursorRect().height))
    }
    func accessibilityContent(forLineNumber n: Int) -> String? {
        let ls = core.lines
        return ls.indices.contains(n) ? String(ls[n]) : nil
    }
    func accessibilityFrame(forLineNumber n: Int) -> CGRect {
        let h = core.cursorRect().height
        return UIAccessibility.convertToScreenCoordinates(
            CGRect(x: padX, y: padY + CGFloat(n) * h, width: bounds.width - 2 * padX, height: h), in: self)
    }
    func accessibilityPageContent() -> String? { core.screenText() }
}

struct PaneRep: UIViewRepresentable {
    let view: TermPaneView
    func makeUIView(context: Context) -> TermPaneView { view }
    func updateUIView(_ v: TermPaneView, context: Context) {}
}

// MARK: - The app

final class Session {
    static let shared = Session()
    let model: Model?
    let views: [TermPaneView]
    let driver: Driver?
    let columns: Int

    init() {
        let size = UIScreen.main.bounds.size
        let n = max(1, (env["VORN_SPIKE_SESSIONS"] ?? "").split(separator: ",").count)
        let (cols, rows) = layout(n)
        columns = cols
        let g = gridSize(CGSize(width: size.width / CGFloat(cols), height: size.height / CGFloat(rows)))
        guard let h = vs_open(g.0, g.1) else {
            model = nil; views = []; driver = nil
            return
        }
        let m = Model(h: h)
        model = m
        views = m.panes.map { TermPaneView(core: $0) }
        let d = Driver(model: m)
        driver = d
        let link = CADisplayLink(target: d, selector: #selector(Driver.frame(_:)))
        link.preferredFrameRateRange = CAFrameRateRange(minimum: 60, maximum: 120, preferred: 120)
        d.inject = { [views] c in
            guard let v = views.first else { return }
            if c == "\r" { v.insertText("\n") } else { v.insertText(String(c)) }
        }
        d.finish = { exit(0) }
        d.start(link: link, fps: UIScreen.main.maximumFramesPerSecond)
        if !look { DispatchQueue.main.async { [views] in _ = views.first?.becomeFirstResponder() as Bool? } }
    }
}

@main
struct VornSpikeApp: App {
    let s = Session.shared

    var body: some Scene {
        WindowGroup {
            Group {
                if s.model == nil {
                    Text("No grid endpoint (VORN_SPIKE_GRID / VORN_SPIKE_SESSIONS)")
                        .foregroundStyle(.white)
                } else if look {
                    LookView(term: PaneRep(view: s.views[0]), polish: polish)
                } else {
                    GridView(panes: s.views.map { PaneRep(view: $0) }, columns: s.columns)
                        .ignoresSafeArea()
                }
            }
            .preferredColorScheme(.dark)
            .statusBarHidden(look)
        }
    }
}
