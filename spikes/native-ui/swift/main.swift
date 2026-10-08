// The SwiftUI/AppKit prototype: a SwiftUI window holding a grid of AppKit
// terminal views drawn with CoreText. Every byte from vornd goes through
// the Rust grid client (libvorn_spike_ffi.a, see ffi/include/vorn_spike.h);
// this file only draws what it is handed and sends keys back.

import AppKit
import CoreText
import QuartzCore
import SwiftUI

// MARK: - Shared state

final class Model {
    let h: OpaquePointer
    let panes: [TermView]
    let mode: UInt32
    var focused = 0
    var firstFrameSent = false
    var typedAt: CFTimeInterval?
    let started = CACurrentMediaTime()

    init(h: OpaquePointer, cell: CGSize) {
        self.h = h
        mode = vs_bench_mode(h)
        let n = Int(vs_panes(h))
        panes = (0..<n).map { TermView(pane: UInt32($0), cell: cell) }
        for p in panes { p.model = self }
    }

    var ms: Double { (CACurrentMediaTime() - started) * 1000 }
}

// MARK: - Fonts and colors

struct Fonts {
    let regular: CTFont, bold: CTFont, italic: CTFont, boldItalic: CTFont
    let cell: CGSize
    let ascent: CGFloat

    init(size: CGFloat) {
        let base = CTFontCreateWithName("Menlo" as CFString, size, nil)
        func variant(_ t: CTFontSymbolicTraits) -> CTFont {
            CTFontCreateCopyWithSymbolicTraits(base, size, nil, t, t) ?? base
        }
        regular = base
        bold = variant(.traitBold)
        italic = variant(.traitItalic)
        boldItalic = variant([.traitBold, .traitItalic])
        var glyph = CGGlyph(0)
        var ch = UniChar(77) // "M"
        CTFontGetGlyphsForCharacters(base, &ch, &glyph, 1)
        var adv = CGSize.zero
        CTFontGetAdvancesForGlyphs(base, .horizontal, &glyph, &adv, 1)
        ascent = ceil(CTFontGetAscent(base))
        let h = ascent + ceil(CTFontGetDescent(base)) + ceil(CTFontGetLeading(base))
        cell = CGSize(width: adv.width, height: h)
    }

    func font(_ flags: UInt16) -> CTFont {
        let b = flags & UInt16(VS_BOLD) != 0, i = flags & UInt16(VS_ITALIC) != 0
        return b && i ? boldItalic : b ? bold : i ? italic : regular
    }

    /// Glyphs of printable ASCII, per font variant: plain runs skip
    /// typesetting entirely.
    lazy var ascii: [[CGGlyph]] = [regular, bold, italic, boldItalic].map { f in
        var chars = (0..<128).map { UniChar($0) }
        var glyphs = [CGGlyph](repeating: 0, count: 128)
        CTFontGetGlyphsForCharacters(f, &chars, &glyphs, 128)
        return glyphs
    }

    func variant(_ flags: UInt16) -> Int {
        (flags & UInt16(VS_BOLD) != 0 ? 1 : 0) + (flags & UInt16(VS_ITALIC) != 0 ? 2 : 0)
    }
}

var fonts = Fonts(size: 12)
var colorCache: [UInt32: CGColor] = [:]

func color(_ rgb: UInt32, alpha: CGFloat = 1) -> CGColor {
    if alpha == 1, let c = colorCache[rgb] { return c }
    let c = CGColor(srgbRed: CGFloat((rgb >> 16) & 0xff) / 255,
                    green: CGFloat((rgb >> 8) & 0xff) / 255,
                    blue: CGFloat(rgb & 0xff) / 255, alpha: alpha)
    if alpha == 1 { colorCache[rgb] = c }
    return c
}

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

final class TermView: NSView, NSTextInputClient {
    let pane: UInt32
    let cell: CGSize
    weak var model: Model?
    var view = VsView()
    var hasView = false
    var size: (UInt16, UInt16) = (0, 0)
    var marked = ""
    var current: NSEvent?
    var inserted = false
    var glyphBuf = [CGGlyph](repeating: 0, count: 1024)
    var posBuf = [CGPoint](repeating: .zero, count: 1024)

    init(pane: UInt32, cell: CGSize) {
        self.pane = pane
        self.cell = cell
        super.init(frame: .zero)
        wantsLayer = true
        layerContentsRedrawPolicy = .onSetNeedsDisplay
    }

    // A wide-gamut display gets half-float backing stores by default, where
    // blitting glyphs costs about 7 ms a pane; 8-bit is plenty.
    override func makeBackingLayer() -> CALayer {
        let l = CALayer()
        l.contentsFormat = .RGBA8Uint
        l.isOpaque = true
        return l
    }

    required init?(coder: NSCoder) { fatalError() }
    deinit { if hasView { vs_view_free(&view) } }

    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override var isOpaque: Bool { true }

    /// Takes the pane's current view; true if it shows the probe's glyph.
    func refresh(_ h: OpaquePointer) -> Bool {
        var v = VsView()
        guard vs_view(h, pane, &v) else { return false }
        if hasView { vs_view_free(&view) }
        view = v
        hasView = true
        return v.probe_hit != 0
    }

    override func setFrameSize(_ s: NSSize) {
        super.setFrameSize(s)
        let c = UInt16(max(2, Int(s.width / cell.width)))
        let r = UInt16(max(1, Int(s.height / cell.height)))
        if (c, r) != size, s.width > 0, let m = model {
            size = (c, r)
            vs_resize(m.h, pane, c, r)
        }
    }

    override func draw(_ dirty: NSRect) {
        guard let ctx = NSGraphicsContext.current?.cgContext else { return }
        let bg = hasView ? view.bg : 0x1e1e1e
        ctx.setFillColor(color(bg))
        ctx.fill(bounds)
        guard hasView else { return }
        let cw = cell.width, ch = cell.height
        ctx.textMatrix = .identity
        ctx.setShouldSmoothFonts(true)
        let runs = UnsafeBufferPointer(start: view.runs, count: Int(view.nruns))
        // Backgrounds first, then text, so wide glyphs are not cut.
        for r in runs where r.bg != view.bg {
            ctx.setFillColor(color(r.bg))
            ctx.fill(CGRect(x: CGFloat(r.col) * cw, y: CGFloat(r.row) * ch,
                            width: CGFloat(r.ncols) * cw, height: ch))
        }
        let fgKey = NSAttributedString.Key(kCTForegroundColorAttributeName as String)
        for r in runs {
            let bytes = UnsafeBufferPointer(start: view.text + Int(r.text_off), count: Int(r.text_len))
            let alpha: CGFloat = r.flags & UInt16(VS_FAINT) != 0 ? 0.6 : 1
            let fg = color(r.fg, alpha: alpha)
            let x = CGFloat(r.col) * cw, y = CGFloat(r.row) * ch
            if r.flags & UInt16(VS_CLUSTER) == 0 {
                // ASCII: one glyph per cell at fixed advances.
                let table = fonts.ascii[fonts.variant(r.flags)]
                var n = 0
                for (i, b) in bytes.prefix(glyphBuf.count).enumerated() where b > 32 && b < 127 {
                    glyphBuf[n] = table[Int(b)]
                    posBuf[n] = CGPoint(x: CGFloat(i) * cw, y: 0)
                    n += 1
                }
                if n > 0 {
                    ctx.saveGState()
                    ctx.translateBy(x: x, y: y + fonts.ascent)
                    ctx.scaleBy(x: 1, y: -1)
                    ctx.setFillColor(fg)
                    CTFontDrawGlyphs(fonts.font(r.flags), glyphBuf, posBuf, n, ctx)
                    ctx.restoreGState()
                }
            } else {
                // Anything else is typeset, so fallback fonts and emoji work.
                let attr = NSAttributedString(string: String(decoding: bytes, as: UTF8.self), attributes: [
                    .font: fonts.font(r.flags), fgKey: fg,
                ])
                let line = CTLineCreateWithAttributedString(attr)
                ctx.saveGState()
                ctx.translateBy(x: x, y: y + fonts.ascent)
                ctx.scaleBy(x: 1, y: -1)
                ctx.textPosition = .zero
                CTLineDraw(line, ctx)
                ctx.restoreGState()
            }
            if r.flags & UInt16(VS_UNDERLINE | VS_STRIKE) != 0 {
                ctx.setFillColor(fg)
                if r.flags & UInt16(VS_UNDERLINE) != 0 {
                    ctx.fill(CGRect(x: x, y: y + ch - 1.5, width: CGFloat(r.ncols) * cw, height: 1))
                }
                if r.flags & UInt16(VS_STRIKE) != 0 {
                    ctx.fill(CGRect(x: x, y: y + ch / 2, width: CGFloat(r.ncols) * cw, height: 1))
                }
            }
        }
        // The cursor, and IME text being composed at it.
        let cx = CGFloat(view.cursor_x) * cw, cy = CGFloat(view.cursor_y) * ch
        if !marked.isEmpty {
            let attr = NSAttributedString(string: marked, attributes: [
                .font: fonts.regular, fgKey: color(view.fg),
            ])
            let line = CTLineCreateWithAttributedString(attr)
            let w = CTLineGetTypographicBounds(line, nil, nil, nil)
            ctx.setFillColor(color(view.bg))
            ctx.fill(CGRect(x: cx, y: cy, width: w, height: ch))
            ctx.saveGState()
            ctx.translateBy(x: cx, y: cy + fonts.ascent)
            ctx.scaleBy(x: 1, y: -1)
            ctx.textPosition = .zero
            CTLineDraw(line, ctx)
            ctx.restoreGState()
            ctx.setFillColor(color(view.fg))
            ctx.fill(CGRect(x: cx, y: cy + ch - 1, width: w, height: 1))
        } else if view.cursor_visible != 0 {
            let focused = window?.firstResponder === self
            ctx.setFillColor(color(view.cursor_color, alpha: 0.75))
            switch view.cursor_style {
            case 2: ctx.fill(CGRect(x: cx, y: cy, width: 2, height: ch))
            case 3: ctx.fill(CGRect(x: cx, y: cy + ch - 2, width: cw, height: 2))
            default:
                if focused && view.cursor_style == 0 {
                    ctx.fill(CGRect(x: cx, y: cy, width: cw, height: ch))
                } else {
                    ctx.setStrokeColor(color(view.cursor_color))
                    ctx.stroke(CGRect(x: cx + 0.5, y: cy + 0.5, width: cw - 1, height: ch - 1))
                }
            }
        }
    }

    // MARK: Input

    override func mouseDown(with e: NSEvent) {
        window?.makeFirstResponder(self)
        model?.focused = Int(pane)
    }

    override func keyDown(with e: NSEvent) {
        guard let m = model else { return }
        // A synthesized event in a window that is not key has no input
        // context to compose with: send it as the key it is.
        if inputContext == nil || window?.isKeyWindow != true {
            sendKey(m, e, text: e.characters)
            return
        }
        current = e
        inserted = false
        interpretKeyEvents([e])
        current = nil
    }

    func sendKey(_ m: Model, _ e: NSEvent, text: String?) {
        let code = keyCodes[e.keyCode] ?? "Unidentified"
        vs_key(m.h, pane, code, wireMods(e.modifierFlags), text)
    }

    func insertText(_ string: Any, replacementRange: NSRange) {
        guard let m = model else { return }
        let s = (string as? NSAttributedString)?.string ?? (string as? String) ?? ""
        let composed = !marked.isEmpty
        marked = ""
        if let e = current, !composed, s == e.characters {
            sendKey(m, e, text: s)
        } else {
            vs_text(m.h, pane, s)
        }
        inserted = true
        needsDisplay = true
    }

    override func doCommand(by selector: Selector) {
        if let m = model, let e = current { sendKey(m, e, text: nil) }
    }

    func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        marked = (string as? NSAttributedString)?.string ?? (string as? String) ?? ""
        needsDisplay = true
    }

    func unmarkText() { marked = ""; needsDisplay = true }
    func selectedRange() -> NSRange { NSRange(location: 0, length: 0) }
    func markedRange() -> NSRange {
        marked.isEmpty ? NSRange(location: NSNotFound, length: 0)
            : NSRange(location: 0, length: (marked as NSString).length)
    }
    func hasMarkedText() -> Bool { !marked.isEmpty }
    func attributedSubstring(forProposedRange r: NSRange, actualRange: NSRangePointer?) -> NSAttributedString? { nil }
    func validAttributesForMarkedText() -> [NSAttributedString.Key] { [] }
    func characterIndex(for point: NSPoint) -> Int { 0 }
    func firstRect(forCharacterRange r: NSRange, actualRange: NSRangePointer?) -> NSRect {
        let local = NSRect(x: CGFloat(view.cursor_x) * cell.width, y: CGFloat(view.cursor_y) * cell.height,
                           width: cell.width, height: cell.height)
        guard let w = window else { return local }
        return w.convertToScreen(convert(local, to: nil))
    }

    // MARK: Accessibility: the pane reads as a text area holding its screen.

    override func isAccessibilityElement() -> Bool { true }
    override func accessibilityRole() -> NSAccessibility.Role? { .textArea }
    override func accessibilityLabel() -> String? { "Terminal \(pane + 1)" }
    override func accessibilityValue() -> Any? { screenText() }

    func screenText() -> String {
        guard hasView else { return "" }
        var rows = [[Character]](repeating: Array(repeating: " ", count: Int(view.cols)),
                                 count: Int(view.rows))
        for r in UnsafeBufferPointer(start: view.runs, count: Int(view.nruns)) where Int(r.row) < rows.count {
            let bytes = UnsafeBufferPointer(start: view.text + Int(r.text_off), count: Int(r.text_len))
            for (i, c) in String(decoding: bytes, as: UTF8.self).enumerated() {
                let x = Int(r.col) + (r.flags & UInt16(VS_CLUSTER) != 0 ? 0 : i)
                if x < rows[Int(r.row)].count { rows[Int(r.row)][x] = c }
            }
        }
        return rows.map { String($0).replacingOccurrences(of: "\\s+$", with: "", options: .regularExpression) }
            .joined(separator: "\n")
    }
}

// MARK: - SwiftUI: the window's content

struct PaneHost: NSViewRepresentable {
    let view: TermView
    func makeNSView(context: Context) -> TermView { view }
    func updateNSView(_ v: TermView, context: Context) {}
}

struct ContentView: View {
    let model: Model
    let columns: Int

    var body: some View {
        let rows = (model.panes.count + columns - 1) / columns
        Grid(horizontalSpacing: 2, verticalSpacing: 2) {
            ForEach(0..<rows, id: \.self) { r in
                GridRow {
                    ForEach(0..<columns, id: \.self) { c in
                        let i = r * columns + c
                        if i < model.panes.count {
                            PaneHost(view: model.panes[i])
                        } else {
                            Color.clear
                        }
                    }
                }
            }
        }
        .background(Color(white: 0.25))
    }
}

// MARK: - The frame loop and the bench driver

final class Driver: NSObject {
    let model: Model
    let window: NSWindow
    var link: CADisplayLink?
    var timer: Timer?
    var dirty = [UInt32](repeating: 0, count: 64)

    init(model: Model, window: NSWindow) {
        self.model = model
        self.window = window
        super.init()
        let link = window.contentView!.displayLink(target: self, selector: #selector(frame(_:)))
        link.add(to: .main, forMode: .common)
        link.isPaused = true
        self.link = link
        let me = Unmanaged.passUnretained(self).toOpaque()
        vs_set_waker(model.h, { ctx in
            let d = Unmanaged<Driver>.fromOpaque(ctx!).takeUnretainedValue()
            DispatchQueue.main.async { d.link?.isPaused = false }
        }, me)
        let fps = NSScreen.main?.maximumFramesPerSecond ?? 60
        vs_bench_set_period(model.h, 1000 / Double(fps))
        if model.mode != 0 {
            timer = Timer.scheduledTimer(withTimeInterval: 0.004, repeats: true) { [weak self] _ in
                self?.benchTick()
            }
            RunLoop.main.add(timer!, forMode: .common)
        }
    }

    /// One display refresh: draw the panes that changed, now.
    @objc func frame(_ l: CADisplayLink) {
        let h = model.h
        let n = vs_take_dirty(h, &dirty, UInt32(dirty.count))
        if n == 0 { l.isPaused = true; return }
        let t0 = CACurrentMediaTime()
        var hit = false
        for i in 0..<Int(n) {
            let p = model.panes[Int(dirty[i])]
            if p.refresh(h) { hit = true }
            p.display()
        }
        CATransaction.flush()
        let t1 = CACurrentMediaTime()
        vs_bench_frame(h, (t1 - model.started) * 1000, (t1 - t0) * 1000)
        if hit, let at = model.typedAt {
            vs_bench_latency(h, (t1 - at) * 1000)
            model.typedAt = nil
        }
        if !model.firstFrameSent && vs_all_snapshotted(h)
            && model.panes.allSatisfy({ $0.hasView }) {
            model.firstFrameSent = true
            vs_bench_first_frame(h)
        }
    }

    func benchTick() {
        var ch: UInt32 = 0
        switch vs_bench_tick(model.h, &ch) {
        case 1: inject(Character(UnicodeScalar(ch)!), keyCode: letterKey[Character(UnicodeScalar(ch)!)] ?? 0)
        case 2: inject("\r", keyCode: 36)
        case 3:
            timer?.invalidate()
            vs_bench_write(model.h)
            NSApp.terminate(nil)
        default: break
        }
    }

    /// A key typed into pane 0 the way AppKit delivers one: an NSEvent
    /// through the window to its first responder.
    func inject(_ c: Character, keyCode: UInt16) {
        guard let e = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [],
                                       timestamp: ProcessInfo.processInfo.systemUptime,
                                       windowNumber: window.windowNumber, context: nil,
                                       characters: String(c), charactersIgnoringModifiers: String(c),
                                       isARepeat: false, keyCode: keyCode) else { return }
        if c != "\r" { model.typedAt = CACurrentMediaTime() }
        window.sendEvent(e)
    }
}

// MARK: - The app

final class AppDelegate: NSObject, NSApplicationDelegate {
    var window: NSWindow!
    var driver: Driver?
    var activity: NSObjectProtocol?

    func applicationDidFinishLaunching(_ n: Notification) {
        activity = ProcessInfo.processInfo.beginActivity(
            options: [.userInitiated, .latencyCritical], reason: "terminal frames")
        let size = NSSize(width: 1440, height: 900)
        let sessions = (ProcessInfo.processInfo.environment["VORN_SPIKE_SESSIONS"] ?? "")
            .split(separator: ",").count
        let (cols, rows) = layout(max(1, sessions))
        let paneW = (size.width - CGFloat(cols - 1) * 2) / CGFloat(cols)
        let paneH = (size.height - CGFloat(rows - 1) * 2) / CGFloat(rows)
        guard let h = vs_open(UInt16(paneW / fonts.cell.width), UInt16(paneH / fonts.cell.height)) else {
            FileHandle.standardError.write("no grid endpoint (VORN_SPIKE_GRID/VORN_SPIKE_SESSIONS)\n".data(using: .utf8)!)
            exit(1)
        }
        let model = Model(h: h, cell: fonts.cell)
        window = NSWindow(contentRect: NSRect(origin: .zero, size: size),
                          styleMask: [.titled, .closable, .resizable, .miniaturizable],
                          backing: .buffered, defer: false)
        window.title = "Vorn spike: SwiftUI/AppKit"
        window.contentView = NSHostingView(rootView: ContentView(model: model, columns: cols))
        window.setFrameTopLeftPoint(NSPoint(x: 40, y: (NSScreen.main?.visibleFrame.maxY ?? 900) - 20))
        if model.mode != 0 { window.level = .floating }
        window.orderFrontRegardless()
        window.makeFirstResponder(model.panes.first)
        if model.mode == 0 { NSApp.activate() }
        driver = Driver(model: model, window: window)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ a: NSApplication) -> Bool { true }
}

func layout(_ n: Int) -> (Int, Int) {
    let cols = min(n, Int(ceil(sqrt(Double(n) * 2))))
    return (cols, (n + cols - 1) / cols)
}

let app = NSApplication.shared
app.setActivationPolicy(.regular)
let delegate = AppDelegate()
app.delegate = delegate
app.run()
