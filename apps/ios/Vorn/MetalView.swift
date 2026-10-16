import QuartzCore
import SwiftUI
import UIKit

/// A CAMetalLayer the Rust app draws into, with the system keyboard behind a hidden text input.
final class MetalView: UIView, UITextInput {
    override class var layerClass: AnyClass { CAMetalLayer.self }

    let core: Core
    let terminalOnly: Bool
    private var link: CADisplayLink?
    private var attached = false
    private var keyboard: CGFloat = 0
    /// What was typed since the keyboard came up, so autocorrect and dictation have context.
    private var typed = ""
    private var marked: NSRange?

    init(core: Core, terminalOnly: Bool) {
        self.core = core
        self.terminalOnly = terminalOnly
        super.init(frame: .zero)
        isMultipleTouchEnabled = false
        backgroundColor = UIColor(red: 0x0b / 255, green: 0x0b / 255, blue: 0x0b / 255, alpha: 1)
        let nc = NotificationCenter.default
        nc.addObserver(self, selector: #selector(keyboardChanged), name: UIResponder.keyboardWillChangeFrameNotification, object: nil)
        nc.addObserver(self, selector: #selector(keyboardChanged), name: UIResponder.keyboardWillHideNotification, object: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    deinit {
        link?.invalidate()
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window == nil {
            link?.invalidate()
            link = nil
            if attached { vp_detach_layer(core.handle); attached = false }
            return
        }
        let l = CADisplayLink(target: self, selector: #selector(tick))
        l.preferredFrameRateRange = CAFrameRateRange(minimum: 60, maximum: 120, preferred: 120)
        l.add(to: .main, forMode: .common)
        link = l
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        let scale = traitCollection.displayScale
        let size = bounds.size
        guard size.width > 0, size.height > 0 else { return }
        layer.contentsScale = scale
        if !attached {
            let ptr = Unmanaged.passUnretained(layer).toOpaque()
            attached = vp_attach_layer(core.handle, ptr, Float(size.width), Float(size.height), Float(scale), terminalOnly)
        }
        resize()
    }

    override func safeAreaInsetsDidChange() {
        super.safeAreaInsetsDidChange()
        resize()
    }

    private func resize() {
        let scale = traitCollection.displayScale
        let s = terminalOnly ? .zero : safeAreaInsets
        let kb = terminalOnly ? 0 : keyboard
        vp_resize(core.handle, Float(bounds.width), Float(bounds.height), Float(scale),
                  Float(s.top), Float(s.bottom), Float(s.left), Float(s.right), Float(kb))
    }

    @objc private func keyboardChanged(_ n: Notification) {
        guard let end = n.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect, let window else { return }
        let mine = convert(bounds, to: window)
        keyboard = n.name == UIResponder.keyboardWillHideNotification ? 0 : max(0, mine.maxY - end.minY)
        resize()
    }

    @objc private func tick() {
        guard attached else { return }
        _ = vp_frame(core.handle)
        syncKeyboard()
    }

    private func syncKeyboard() {
        let wants = vp_wants_keyboard(core.handle)
        if wants && !isFirstResponder {
            typed = ""
            becomeFirstResponder()
        } else if !wants && isFirstResponder {
            resignFirstResponder()
        }
    }

    // Touches: the first finger only, in points.

    private func send(_ phase: Int32, _ touches: Set<UITouch>) {
        guard let t = touches.first else { return }
        let p = t.location(in: self)
        vp_touch(core.handle, phase, Float(p.x), Float(p.y))
    }

    override func touchesBegan(_ t: Set<UITouch>, with e: UIEvent?) { send(0, t) }
    override func touchesMoved(_ t: Set<UITouch>, with e: UIEvent?) {
        guard let touch = t.first else { return }
        for c in e?.coalescedTouches(for: touch) ?? [touch] {
            let p = c.location(in: self)
            vp_touch(core.handle, 1, Float(p.x), Float(p.y))
        }
    }
    override func touchesEnded(_ t: Set<UITouch>, with e: UIEvent?) { send(2, t) }
    override func touchesCancelled(_ t: Set<UITouch>, with e: UIEvent?) { vp_touch(core.handle, 3, 0, 0) }

    // The keyboard.

    override var canBecomeFirstResponder: Bool { true }

    override func resignFirstResponder() -> Bool {
        let r = super.resignFirstResponder()
        if r { vp_keyboard_hidden(core.handle) }
        return r
    }

    var hasText: Bool { !typed.isEmpty }

    func insertText(_ text: String) {
        if let m = marked {
            typed = (typed as NSString).replacingCharacters(in: m, with: "")
            marked = nil
            vp_preedit(core.handle, "")
        }
        typed += text
        vp_text(core.handle, text)
    }

    func deleteBackward() {
        if !typed.isEmpty { typed.removeLast() }
        vp_key(core.handle, "Backspace", 0)
    }

    private func commitMarked() {
        guard let m = marked else { return }
        let s = (typed as NSString).substring(with: m)
        typed = (typed as NSString).replacingCharacters(in: m, with: "")
        marked = nil
        vp_preedit(core.handle, "")
        if !s.isEmpty { typed += s; vp_text(core.handle, s) }
    }

    var autocorrectionType: UITextAutocorrectionType = .default
    var autocapitalizationType: UITextAutocapitalizationType = .none
    var smartQuotesType: UITextSmartQuotesType = .no
    var smartDashesType: UITextSmartDashesType = .no
    var keyboardAppearance: UIKeyboardAppearance = .dark
    var returnKeyType: UIReturnKeyType = .default

    override var keyCommands: [UIKeyCommand]? {
        let arrows = [UIKeyCommand.inputUpArrow, UIKeyCommand.inputDownArrow, UIKeyCommand.inputLeftArrow,
                      UIKeyCommand.inputRightArrow, UIKeyCommand.inputEscape]
        return arrows.map {
            let c = UIKeyCommand(input: $0, modifierFlags: [], action: #selector(hardKey(_:)))
            c.wantsPriorityOverSystemBehavior = true
            return c
        }
    }

    @objc private func hardKey(_ c: UIKeyCommand) {
        let code: String
        switch c.input {
        case UIKeyCommand.inputUpArrow: code = "ArrowUp"
        case UIKeyCommand.inputDownArrow: code = "ArrowDown"
        case UIKeyCommand.inputLeftArrow: code = "ArrowLeft"
        case UIKeyCommand.inputRightArrow: code = "ArrowRight"
        default: code = "Escape"
        }
        vp_key(core.handle, code, 0)
    }

    // UITextInput over `typed`, offsets in UTF-16 units.

    final class Pos: UITextPosition {
        let at: Int
        init(_ at: Int) { self.at = at }
    }

    final class Span: UITextRange {
        let lo: Int, hi: Int
        init(_ lo: Int, _ hi: Int) { self.lo = lo; self.hi = hi }
        override var start: UITextPosition { Pos(lo) }
        override var end: UITextPosition { Pos(hi) }
        override var isEmpty: Bool { lo == hi }
    }

    private var len: Int { (typed as NSString).length }
    private func clamp(_ i: Int) -> Int { min(max(i, 0), len) }
    private func span(_ r: UITextRange) -> NSRange {
        guard let r = r as? Span else { return NSRange(location: len, length: 0) }
        return NSRange(location: clamp(r.lo), length: clamp(r.hi) - clamp(r.lo))
    }

    func text(in range: UITextRange) -> String? { (typed as NSString).substring(with: span(range)) }

    func replace(_ range: UITextRange, withText text: String) {
        let r = span(range)
        let tail = len - r.location - r.length
        // Autocorrect rewrites words behind the caret: move to it, delete, type, move back.
        for _ in 0..<tail { vp_key(core.handle, "ArrowLeft", 0) }
        for _ in 0..<r.length { vp_key(core.handle, "Backspace", 0) }
        if !text.isEmpty { vp_text(core.handle, text) }
        for _ in 0..<tail { vp_key(core.handle, "ArrowRight", 0) }
        typed = (typed as NSString).replacingCharacters(in: r, with: text)
    }

    var selectedTextRange: UITextRange? {
        get { Span(len, len) }
        set {}
    }

    var markedTextRange: UITextRange? { marked.map { Span($0.location, $0.location + $0.length) } }
    var markedTextStyle: [NSAttributedString.Key: Any]?

    func setMarkedText(_ markedText: String?, selectedRange: NSRange) {
        let s = markedText ?? ""
        let r = marked ?? NSRange(location: len, length: 0)
        typed = (typed as NSString).replacingCharacters(in: r, with: s)
        marked = s.isEmpty ? nil : NSRange(location: r.location, length: (s as NSString).length)
        vp_preedit(core.handle, s)
    }

    func unmarkText() { commitMarked() }

    var beginningOfDocument: UITextPosition { Pos(0) }
    var endOfDocument: UITextPosition { Pos(len) }

    func textRange(from a: UITextPosition, to b: UITextPosition) -> UITextRange? {
        guard let a = a as? Pos, let b = b as? Pos else { return nil }
        return Span(min(a.at, b.at), max(a.at, b.at))
    }

    func position(from p: UITextPosition, offset: Int) -> UITextPosition? {
        guard let p = p as? Pos else { return nil }
        let i = p.at + offset
        return i < 0 || i > len ? nil : Pos(i)
    }

    func position(from p: UITextPosition, in d: UITextLayoutDirection, offset: Int) -> UITextPosition? {
        position(from: p, offset: d == .left || d == .up ? -offset : offset)
    }

    func compare(_ a: UITextPosition, to b: UITextPosition) -> ComparisonResult {
        let (x, y) = ((a as? Pos)?.at ?? 0, (b as? Pos)?.at ?? 0)
        return x < y ? .orderedAscending : x > y ? .orderedDescending : .orderedSame
    }

    func offset(from a: UITextPosition, to b: UITextPosition) -> Int {
        ((b as? Pos)?.at ?? 0) - ((a as? Pos)?.at ?? 0)
    }

    weak var inputDelegate: UITextInputDelegate?
    lazy var tokenizer: UITextInputTokenizer = UITextInputStringTokenizer(textInput: self)

    func position(within r: UITextRange, farthestIn d: UITextLayoutDirection) -> UITextPosition? {
        d == .left || d == .up ? r.start : r.end
    }

    func characterRange(byExtending p: UITextPosition, in d: UITextLayoutDirection) -> UITextRange? {
        guard let p = p as? Pos else { return nil }
        return d == .left || d == .up ? Span(0, p.at) : Span(p.at, len)
    }

    func baseWritingDirection(for p: UITextPosition, in d: UITextStorageDirection) -> NSWritingDirection { .leftToRight }
    func setBaseWritingDirection(_ w: NSWritingDirection, for r: UITextRange) {}

    /// The caret sits at the bottom of the view so candidate bars and dictation stay out of the way.
    func firstRect(for r: UITextRange) -> CGRect { caretRect(for: r.end) }
    func caretRect(for p: UITextPosition) -> CGRect { CGRect(x: 8, y: bounds.maxY - keyboard - 24, width: 2, height: 20) }
    func selectionRects(for r: UITextRange) -> [UITextSelectionRect] { [] }
    func closestPosition(to p: CGPoint) -> UITextPosition? { Pos(len) }
    func closestPosition(to p: CGPoint, within r: UITextRange) -> UITextPosition? { r.end }
    func characterRange(at p: CGPoint) -> UITextRange? { nil }
}

/// A MetalView in SwiftUI.
struct Metal: UIViewRepresentable {
    let core: Core
    let terminalOnly: Bool

    func makeUIView(context: Context) -> MetalView { MetalView(core: core, terminalOnly: terminalOnly) }
    func updateUIView(_ v: MetalView, context: Context) {}
}
