import AppKit
import CVornGrid
import SwiftUI
import VornCore
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
        registerForDraggedTypes([.fileURL])
        bottomButton.target = self
        bottomButton.action = #selector(scrollToBottom(_:))
        bottomButton.isHidden = true
        addSubview(bottomButton)
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

    func paneDidChange() {
        if let v = pane.view {
            // New output: a view scrolled back stays on its lines.
            if v.alternate || v.live_line < liveLine {
                scroll.reset()
            } else {
                scroll.grew(v.live_line - liveLine)
            }
            liveLine = v.live_line
        }
        if findBar != nil, !found.done { loadMatches() }
        needsDisplay = true
        updateBottomButton()
    }

    public override func layout() {
        super.layout()
        let s = ScrollToBottomButton.size
        bottomButton.frame = CGRect(x: bounds.maxX - s - 8, y: bounds.maxY - s - 8, width: s, height: s)
    }

    func fontDidChange() {
        layoutGrid()
        needsDisplay = true
    }

    /// Asks before a paste vornd held; answers true to send it. Replaceable for tests.
    var confirmPaste: ((String, @escaping (Bool) -> Void) -> Void)?

    func pasteHeld(_ text: String) {
        let send: (Bool) -> Void = { [weak self] ok in if ok { self?.pane.paste(text, confirmed: true) } }
        if let confirmPaste { return confirmPaste(text, send) }
        guard let window else { return }
        let lines = text.split(whereSeparator: \.isNewline).count
        let alert = NSAlert()
        alert.messageText = "Paste \(lines) \(lines == 1 ? "line" : "lines") into the terminal?"
        alert.informativeText = "Each line will run as a command."
        alert.addButton(withTitle: "Paste")
        alert.addButton(withTitle: "Cancel")
        alert.beginSheetModal(for: window) { send($0 == .alertFirstButtonReturn) }
    }

    public override func setFrameSize(_ s: NSSize) {
        super.setFrameSize(s)
        layoutGrid()
        layoutFindBar()
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
                     selection: pane.selection, matches: visibleMatches,
                     current: found.at.map { found.hits[$0] }, offset: scrollOffset)
        didDraw?(ctx)
    }

    public override func becomeFirstResponder() -> Bool {
        pane.setFocus(true)
        onFocus?(pane.sessionID)
        return true
    }

    public override func resignFirstResponder() -> Bool {
        pane.setFocus(false)
        updateBottomButton()
        return true
    }

    // MARK: Scroll

    private var scroll = TerminalScroll()
    private var rows = WheelRows()
    /// Where the gesture under way goes, and when it was last heard.
    private var gesture: (route: WheelRoute, at: TimeInterval)?
    private var liveLine: UInt64 = 0
    private let bottomButton = ScrollToBottomButton()

    /// Points to draw the view down by: the scroll's offset from the view's rows,
    /// on whole device pixels.
    private var scrollOffset: CGFloat {
        guard let v = pane.view else { return 0 }
        let k = window?.backingScaleFactor ?? 2
        return ((scroll.pos - CGFloat(v.back) * fonts.cell.height) * k).rounded() / k
    }

    private func syncScroll() {
        guard let v = pane.view else { return }
        scroll.setMetrics(row: fonts.cell.height, span: bounds.height, historyRows: v.alternate ? 0 : v.history_lines)
        scroll.reduced = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
    }

    /// Lines the view up with the scroll: the rows it starts at come from the client.
    private func applyScroll() {
        if let v = pane.view, scroll.split.back != v.back {
            pane.scrollTo(scroll.split.back)
            liveLine = pane.view?.live_line ?? liveLine
        }
        needsDisplay = true
        updateBottomButton()
    }

    func scrollFrame() -> Bool {
        guard window != nil, !isHiddenOrHasHiddenAncestor else { return false }
        syncScroll()
        let moving = scroll.tick(CACurrentMediaTime())
        applyScroll()
        return moving
    }

    /// Scrolls `n` rows with a notch's ease, negative up.
    private func scrollRows(_ n: Int) {
        guard let v = pane.view, !v.alternate else { return }
        syncScroll()
        scroll.wheel(CGFloat(n) / TerminalScroll.notchRows, .line, now: CACurrentMediaTime())
        pane.engine.animate(pane)
    }

    /// Back to the live screen at once, as typing does.
    private func returnToLive() {
        guard scroll.scrolled || !scroll.resting else { return }
        scroll.reset()
        applyScroll()
    }

    @objc func scrollToBottom(_ sender: Any?) { returnToLive() }

    private func updateBottomButton() {
        let show = scroll.scrolled && !pane.focused
        if bottomButton.isHidden == show { bottomButton.isHidden = !show }
    }

    // MARK: Input

    /// How a drag grows the selection: by cell, word (double click) or line (triple).
    private enum Unit { case cell, word, line }
    private var unit = Unit.cell
    /// What the drag started on; the selection always keeps it.
    private var anchor: TerminalSelection?
    /// The press went to a program tracking the mouse.
    private var tracking = false

    /// Where an event is: the cell fraction the program hears, and the absolute
    /// cell boundary nearest it, kept on the screen.
    private func cell(at e: NSEvent) -> (x: Float, y: Float, point: GridPoint)? {
        guard let v = pane.view else { return nil }
        let p = convert(e.locationInWindow, from: nil)
        let x = max(0, (p.x - terminalPadX) / fonts.cell.width)
        let y = (p.y - terminalPadY - scrollOffset) / fonts.cell.height
        let row = min(max(Int(y.rounded(.down)), 0), Int(v.drawnRows) - 1)
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
        if local < 0 { scrollRows(-1) } else if local > bounds.height { scrollRows(1) }
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

    /// A trackpad's touch and momentum phases, or a wheel's notches.
    static func phase(of e: NSEvent) -> TerminalScroll.Phase? {
        guard e.hasPreciseScrollingDeltas else { return .line }
        let p = e.momentumPhase.isEmpty ? e.phase : e.momentumPhase
        if p.isEmpty { return .moved }
        if p.contains(.began) { return .began }
        if p.contains(.changed) { return .moved }
        if p.contains(.ended) || p.contains(.cancelled) { return .ended }
        return nil
    }

    public override func scrollWheel(with e: NSEvent) {
        guard let (x, y, _) = cell(at: e), let v = pane.view, let phase = Self.phase(of: e) else {
            return super.scrollWheel(with: e)
        }
        let now = CACurrentMediaTime()
        // Points, or notches; positive towards the live screen.
        let dy = -e.scrollingDeltaY
        // A gesture stays where it began, momentum and all.
        let route: WheelRoute
        if phase != .line, let g = gesture, now - g.at < 0.2 {
            route = g.route
        } else {
            route = pane.engine.readOnly ? .history : WheelRoute(mouse: v.mouse == 1, alternate: v.alternate)
        }
        gesture = (route, now)
        guard route == .history else {
            let n = rows.take(dy, phase, row: fonts.cell.height, route: route)
            if n != 0 { pane.wheel(n, x: x, y: y, mods: wireMods(e.modifierFlags)) }
            return
        }
        if v.alternate { return }
        // A wheel down at the live screen: the page around scrolls instead.
        if phase == .line, dy > 0, !scroll.scrolled { return super.scrollWheel(with: e) }
        syncScroll()
        scroll.wheel(dy, phase, now: now)
        applyScroll()
        pane.engine.animate(pane)
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
        returnToLive()
        pane.paste(text)
    }

    /// Ctrl-L for the program: a shell clears its screen and draws the prompt again.
    @objc public func clearScreen(_ sender: Any?) {
        pane.selection = nil
        returnToLive()
        pane.text("\u{0c}")
    }

    // MARK: Drag and drop: files type their quoted paths.

    private func droppedFiles(_ info: NSDraggingInfo) -> [URL] {
        guard !pane.engine.readOnly else { return [] }
        return info.draggingPasteboard.readObjects(forClasses: [NSURL.self],
                                                   options: [.urlReadingFileURLsOnly: true]) as? [URL] ?? []
    }

    public override func draggingEntered(_ info: NSDraggingInfo) -> NSDragOperation {
        droppedFiles(info).isEmpty ? [] : .copy
    }

    public override func performDragOperation(_ info: NSDraggingInfo) -> Bool {
        let files = droppedFiles(info)
        guard !files.isEmpty else { return false }
        window?.makeFirstResponder(self)
        returnToLive()
        pane.paste(TerminalKeys.quotedPaths(files), confirmed: true)
        return true
    }

    // MARK: Find

    private var findBar: TerminalFindBar?
    /// The matches so far, oldest first, and the one shown.
    private var found: (hits: [VgHit], done: Bool, at: Int?) = ([], true, nil)
    private var query = ""

    var isFinding: Bool { findBar != nil }
    var findStatus: String { findBar?.countText ?? "" }

    /// The matches on the rows in view.
    private var visibleMatches: ArraySlice<VgHit> {
        guard findBar != nil, let v = pane.view, !found.hits.isEmpty else { return [] }
        let end = v.first_line + UInt64(v.rows)
        return found.hits[firstMatch(atOrAfter: v.first_line)..<firstMatch(atOrAfter: end)]
    }

    /// The index of the first match on `line` or below; the matches are in line order.
    private func firstMatch(atOrAfter line: UInt64) -> Int {
        var (lo, hi) = (0, found.hits.count)
        while lo < hi {
            let mid = (lo + hi) / 2
            if found.hits[mid].line < line { lo = mid + 1 } else { hi = mid }
        }
        return lo
    }

    /// Shows the find bar, or puts the cursor back in it.
    @objc public func findInTerminal(_ sender: Any?) {
        let bar = findBar ?? makeFindBar()
        window?.makeFirstResponder(bar.field)
        bar.field.currentEditor()?.selectAll(nil)
    }

    @objc public func findNext(_ sender: Any?) { findBar == nil ? findInTerminal(sender) : step(1) }
    @objc public func findPrevious(_ sender: Any?) { findBar == nil ? findInTerminal(sender) : step(-1) }

    /// The Edit menu's Find items.
    @objc public override func performTextFinderAction(_ sender: Any?) {
        switch NSTextFinder.Action(rawValue: (sender as? NSValidatedUserInterfaceItem)?.tag ?? 0) {
        case .nextMatch: findNext(sender)
        case .previousMatch: findPrevious(sender)
        case .hideFindInterface: closeFind()
        default: findInTerminal(sender)
        }
    }

    private func makeFindBar() -> TerminalFindBar {
        let bar = TerminalFindBar()
        bar.onQuery = { [weak self] in self?.search($0) }
        bar.onStep = { [weak self] in self?.step($0) }
        bar.onClose = { [weak self] in self?.closeFind() }
        addSubview(bar)
        findBar = bar
        layoutFindBar()
        if !query.isEmpty { bar.field.stringValue = query; search(query) }
        return bar
    }

    private func layoutFindBar() {
        guard let findBar else { return }
        let s = TerminalFindBar.size
        findBar.frame = NSRect(x: max(bounds.width - s.width - 8, 0), y: 8, width: min(s.width, bounds.width), height: s.height)
    }

    func search(_ q: String) {
        query = q
        found = ([], q.isEmpty, nil)
        pane.search(q)
        findBar?.show(current: nil, total: 0, query: q)
        needsDisplay = true
    }

    /// Takes the matches come so far; once all have, shows the newest.
    private func loadMatches() {
        let (hits, done) = pane.hits()
        found.hits = hits
        found.done = done
        if done, found.at == nil, !hits.isEmpty { show(hits.count - 1) }
        findBar?.show(current: found.at, total: hits.count, query: query)
    }

    /// Moves to the next match (1) or the previous one (-1), round the ends.
    func step(_ by: Int) {
        let n = found.hits.count
        guard n > 0 else { return }
        show(((found.at ?? (by > 0 ? -1 : n)) + by + n) % n)
    }

    private func show(_ i: Int) {
        found.at = i
        pane.reveal(line: found.hits[i].line)
        findBar?.show(current: i, total: found.hits.count, query: query)
        needsDisplay = true
    }

    func closeFind() {
        guard let bar = findBar else { return }
        bar.removeFromSuperview()
        findBar = nil
        found = ([], true, nil)
        pane.search("")
        window?.makeFirstResponder(self)
        needsDisplay = true
    }

    /// The terminal's own shortcuts, taken before the menus see them: find and clear.
    public func performShortcut(keyCode: UInt16, modifiers: NSEvent.ModifierFlags) -> Bool {
        switch TerminalKeys.shortcut(keyCode: keyCode, modifiers: modifiers) {
        case .find?: findInTerminal(nil)
        case .findNext?: findNext(nil)
        case .findPrevious?: findPrevious(nil)
        case .clear?: if pane.engine.readOnly { return false } else { clearScreen(nil) }
        case nil: return false
        }
        return true
    }

    /// The terminal whose screen or find bar has keyboard focus.
    public static func owning(_ responder: NSResponder?) -> TerminalNSView? {
        var view = ((responder as? NSTextView)?.delegate as? NSView) ?? responder as? NSView
        while let v = view {
            if let t = v as? TerminalNSView { return t }
            view = v.superview
        }
        return nil
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
        m.addItem(withTitle: "Select All", action: #selector(selectAll(_:)), keyEquivalent: "")
        m.addItem(.separator())
        m.addItem(withTitle: "Find…", action: #selector(findInTerminal(_:)), keyEquivalent: "")
        m.addItem(withTitle: "Clear Screen", action: #selector(clearScreen(_:)), keyEquivalent: "")
        m.addItem(.separator())
        m.addItem(withTitle: "Fit to This Window", action: #selector(fitToWindow(_:)), keyEquivalent: "")
        for item in m.items { item.target = self }
        return m
    }

    public override func performKeyEquivalent(with e: NSEvent) -> Bool {
        guard window?.firstResponder === self, e.type == .keyDown else { return super.performKeyEquivalent(with: e) }
        let f = e.modifierFlags
        // Control chords belong to the shell, except the app's own Ctrl+`; so do ⌘← ⌘→ ⌘⌫.
        let shells = f.contains(.control) && !f.contains(.command) && e.keyCode != 50
        guard shells || f.contains(.command) && route(e) != .encode else { return super.performKeyEquivalent(with: e) }
        keyDown(with: e)
        return true
    }

    private func route(_ e: NSEvent) -> TerminalKey {
        TerminalKeys.route(keyCode: e.keyCode, modifiers: e.modifierFlags, unmodified: e.charactersIgnoringModifiers,
                           alternateScreen: pane.view.map { $0.screen & UInt8(VG_ALTERNATE) != 0 } ?? false,
                           optionAsMeta: TerminalPreferences.optionAsMeta)
    }

    public override func keyDown(with e: NSEvent) {
        pane.selection = nil
        // While the input method composes, its keys are its own.
        switch pane.marked.isEmpty ? route(e) : .encode {
        case .page(let up):
            // Shift-Page Up and Down page through history, as the shell never sees them.
            let page = max(Int(pane.view?.rows ?? 2) - 1, 1)
            scrollRows(up ? -page : page)
            return
        case .send(let bytes):
            returnToLive()
            pane.text(bytes)
            return
        case .encode: break
        }
        returnToLive()
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
        returnToLive()
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
        case #selector(fitToWindow(_:)), #selector(clearScreen(_:)): return !pane.engine.readOnly
        default: return true
        }
    }

    // MARK: Accessibility: the screen as a text area.

    public override func isAccessibilityElement() -> Bool { true }
    public override func accessibilityRole() -> NSAccessibility.Role? { .textArea }
    public override func accessibilityLabel() -> String? { "Terminal" }
    public override func accessibilityValue() -> Any? { pane.screenText() }
}

// MARK: - Scroll to bottom

/// The arrow over a terminal scrolled back into its history, unfocused:
/// straight back to the live screen.
final class ScrollToBottomButton: NSButton {
    static let size: CGFloat = 32
    private var hovering = false { didSet { needsDisplay = true } }

    init() {
        super.init(frame: .zero)
        isBordered = false
        title = ""
        toolTip = "Scroll to bottom"
        setAccessibilityLabel("Scroll to bottom")
    }

    override var isFlipped: Bool { true }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        for t in trackingAreas { removeTrackingArea(t) }
        addTrackingArea(NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect],
                                       owner: self))
    }

    override func mouseEntered(with event: NSEvent) { hovering = true }
    override func mouseExited(with event: NSEvent) { hovering = false }

    override func draw(_ dirty: NSRect) {
        let fill = isHighlighted ? 0.2 : hovering ? 0.15 : 0.08
        NSColor.white.withAlphaComponent(fill).setFill()
        NSBezierPath(roundedRect: bounds, xRadius: 4, yRadius: 4).fill()
        // A 12 pt arrow down, centred.
        let o = CGPoint(x: (bounds.width - 12) / 2, y: (bounds.height - 12) / 2)
        let arrow = NSBezierPath()
        arrow.move(to: CGPoint(x: o.x + 6, y: o.y + 2.5))
        arrow.line(to: CGPoint(x: o.x + 6, y: o.y + 9.5))
        arrow.move(to: CGPoint(x: o.x + 3, y: o.y + 7))
        arrow.line(to: CGPoint(x: o.x + 6, y: o.y + 10))
        arrow.line(to: CGPoint(x: o.x + 9, y: o.y + 7))
        arrow.lineWidth = 1.5
        arrow.lineCapStyle = .round
        arrow.lineJoinStyle = .round
        (hovering ? NSColor.white : NSColor(Theme.gray400)).setStroke()
        arrow.stroke()
    }
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
