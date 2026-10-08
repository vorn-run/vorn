// Shared by the macOS and iOS apps: fonts, option A's CoreText painter, the
// per-pane state behind either renderer, and the frame loop. Every byte from
// vornd goes through the Rust grid client (vorn_spike.h); option B's panes
// are drawn by the Rust GPU renderer (vorn_term.h) into a layer it adds to
// the pane's view, and this code only routes input and accessibility to it.

import CoreText
import QuartzCore
import SwiftUI
#if os(macOS)
import AppKit
#else
import UIKit
#endif

let env = ProcessInfo.processInfo.environment
let look = env["VORN_SPIKE_LOOK"] == "1"
let polish = env["VORN_SPIKE_POLISH"] == "1"
let fontSize = CGFloat(Double(env["VORN_SPIKE_FONT_SIZE"] ?? "") ?? 12)
/// Padding between a pane's edge and its cells, in points (both options).
let padX: CGFloat = 8, padY: CGFloat = 4

// MARK: - Fonts and colours

struct Fonts {
    let regular: CTFont, bold: CTFont, italic: CTFont, boldItalic: CTFont
    let cell: CGSize
    let ascent: CGFloat
    /// Glyphs of printable ASCII per variant: plain runs skip typesetting.
    let ascii: [[CGGlyph]]

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
        ascii = [regular, bold, italic, boldItalic].map { f in
            var chars = (0..<128).map { UniChar($0) }
            var glyphs = [CGGlyph](repeating: 0, count: 128)
            CTFontGetGlyphsForCharacters(f, &chars, &glyphs, 128)
            return glyphs
        }
    }

    func variant(_ flags: UInt16) -> Int {
        (flags & UInt16(VS_BOLD) != 0 ? 1 : 0) + (flags & UInt16(VS_ITALIC) != 0 ? 2 : 0)
    }

    func font(_ flags: UInt16) -> CTFont {
        [regular, bold, italic, boldItalic][variant(flags)]
    }
}

let fonts = Fonts(size: fontSize)
var colorCache: [UInt32: CGColor] = [:]

func color(_ rgb: UInt32, alpha: CGFloat = 1) -> CGColor {
    if alpha == 1, let c = colorCache[rgb] { return c }
    let c = CGColor(srgbRed: CGFloat((rgb >> 16) & 0xff) / 255,
                    green: CGFloat((rgb >> 8) & 0xff) / 255, blue: CGFloat(rgb & 0xff) / 255, alpha: alpha)
    if alpha == 1 { colorCache[rgb] = c }
    return c
}

/// The grid that fits `size` points with the padding.
func gridSize(_ size: CGSize) -> (UInt16, UInt16) {
    (UInt16(max(2, Int((size.width - 2 * padX) / fonts.cell.width))),
     UInt16(max(1, Int((size.height - 2 * padY) / fonts.cell.height))))
}

// MARK: - Option A: CoreText into the view's own backing store

final class Painter {
    var glyphs = [CGGlyph](repeating: 0, count: 1024)
    var positions = [CGPoint](repeating: .zero, count: 1024)

    /// Draws `v` into a top-down context covering `bounds`.
    func draw(_ ctx: CGContext, _ v: VsView?, bounds: CGRect, focused: Bool, marked: String) {
        ctx.setFillColor(color(v?.bg ?? 0x141416))
        ctx.fill(bounds)
        guard let v else { return }
        ctx.saveGState()
        defer { ctx.restoreGState() }
        ctx.translateBy(x: padX, y: padY)
        let cw = fonts.cell.width, ch = fonts.cell.height
        ctx.textMatrix = .identity
        ctx.setShouldSmoothFonts(true)
        let runs = UnsafeBufferPointer(start: v.runs, count: Int(v.nruns))
        // Backgrounds first, then text, so wide glyphs are not cut.
        for r in runs where r.bg != v.bg {
            ctx.setFillColor(color(r.bg))
            ctx.fill(CGRect(x: CGFloat(r.col) * cw, y: CGFloat(r.row) * ch, width: CGFloat(r.ncols) * cw, height: ch))
        }
        let fgKey = NSAttributedString.Key(kCTForegroundColorAttributeName as String)
        let fontKey = NSAttributedString.Key(kCTFontAttributeName as String)
        for r in runs {
            let bytes = UnsafeBufferPointer(start: v.text + Int(r.text_off), count: Int(r.text_len))
            let fg = color(r.fg, alpha: r.flags & UInt16(VS_FAINT) != 0 ? 0.6 : 1)
            let x = CGFloat(r.col) * cw, y = CGFloat(r.row) * ch
            if r.flags & UInt16(VS_CLUSTER) == 0 {
                let table = fonts.ascii[fonts.variant(r.flags)]
                var n = 0
                for (i, b) in bytes.prefix(glyphs.count).enumerated() where b > 32 && b < 127 {
                    glyphs[n] = table[Int(b)]
                    positions[n] = CGPoint(x: CGFloat(i) * cw, y: 0)
                    n += 1
                }
                if n > 0 {
                    ctx.saveGState()
                    ctx.translateBy(x: x, y: y + fonts.ascent)
                    ctx.scaleBy(x: 1, y: -1)
                    ctx.setFillColor(fg)
                    CTFontDrawGlyphs(fonts.font(r.flags), glyphs, positions, n, ctx)
                    ctx.restoreGState()
                }
            } else {
                // Anything else is typeset, so fallback fonts and emoji work.
                let s = NSAttributedString(string: String(decoding: bytes, as: UTF8.self),
                                           attributes: [fontKey: fonts.font(r.flags), fgKey: fg])
                line(ctx, CTLineCreateWithAttributedString(s), x, y)
            }
            if r.flags & UInt16(VS_UNDERLINE) != 0 {
                ctx.setFillColor(fg)
                ctx.fill(CGRect(x: x, y: y + ch - 1.5, width: CGFloat(r.ncols) * cw, height: 1))
            }
            if r.flags & UInt16(VS_STRIKE) != 0 {
                ctx.setFillColor(fg)
                ctx.fill(CGRect(x: x, y: y + ch / 2, width: CGFloat(r.ncols) * cw, height: 1))
            }
        }
        // The cursor, and the IME's composition at it.
        let cx = CGFloat(v.cursor_x) * cw, cy = CGFloat(v.cursor_y) * ch
        if !marked.isEmpty {
            let s = NSAttributedString(string: marked, attributes: [fontKey: fonts.regular, fgKey: color(v.fg)])
            let l = CTLineCreateWithAttributedString(s)
            let w = CTLineGetTypographicBounds(l, nil, nil, nil)
            ctx.setFillColor(color(v.bg))
            ctx.fill(CGRect(x: cx, y: cy, width: w, height: ch))
            line(ctx, l, cx, cy)
            ctx.setFillColor(color(v.fg))
            ctx.fill(CGRect(x: cx, y: cy + ch - 1, width: w, height: 1))
        } else if v.cursor_visible != 0 {
            ctx.setFillColor(color(v.cursor_color, alpha: 0.75))
            switch v.cursor_style {
            case 2: ctx.fill(CGRect(x: cx, y: cy, width: 2, height: ch))
            case 3: ctx.fill(CGRect(x: cx, y: cy + ch - 2, width: cw, height: 2))
            default:
                if focused && v.cursor_style == 0 {
                    ctx.fill(CGRect(x: cx, y: cy, width: cw, height: ch))
                } else {
                    ctx.setStrokeColor(color(v.cursor_color))
                    ctx.stroke(CGRect(x: cx + 0.5, y: cy + 0.5, width: cw - 1, height: ch - 1))
                }
            }
        }
    }

    private func line(_ ctx: CGContext, _ l: CTLine, _ x: CGFloat, _ y: CGFloat) {
        ctx.saveGState()
        ctx.translateBy(x: x, y: y + fonts.ascent)
        ctx.scaleBy(x: 1, y: -1)
        ctx.textPosition = .zero
        CTLineDraw(l, ctx)
        ctx.restoreGState()
    }
}

// MARK: - A pane, whichever renderer draws it

/// What the shared code needs from a pane's platform view.
protocol PaneHostView: AnyObject {
    var paneSize: CGSize { get }
    var scale: CGFloat { get }
    var handle: UnsafeMutableRawPointer { get }
    func redraw()
}

final class PaneCore {
    let index: UInt32
    unowned let model: Model
    weak var host: PaneHostView?
    var view = VsView()
    var hasView = false
    var size: (UInt16, UInt16) = (0, 0)
    var marked = ""
    var focused = false
    /// Option B's surface while the GPU renderer draws this pane.
    var gpu: OpaquePointer?
    var wantsGPU: Bool
    let painter = Painter()
    /// Accessibility's copy of the screen text; nil when stale.
    var a11y: String?

    init(model: Model, index: UInt32) {
        self.model = model
        self.index = index
        wantsGPU = vs_pane_renderer(model.h, index) == 1
    }

    deinit { if hasView { vs_view_free(&view) } }

    /// Option A: takes the pane's current view; true if it shows the probe.
    func refresh() -> Bool {
        var v = VsView()
        guard vs_view(model.h, index, &v) else { return false }
        if hasView { vs_view_free(&view) }
        view = v
        hasView = true
        a11y = nil
        return v.probe_hit != 0
    }

    /// The host view is in a window (its scale is known) or was resized.
    func layout() {
        guard let host else { return }
        if wantsGPU && gpu == nil, let app = model.gpuApp() {
            gpu = vt_surface_new(app, index, host.handle, Double(host.scale), Double(fontSize))
            vt_surface_set_focus(gpu, focused)
        }
        let s = host.paneSize
        guard s.width > 0 else { return }
        if let gpu {
            vt_surface_set_size(gpu, s.width, s.height)
        } else {
            let g = gridSize(s)
            if g != size {
                size = g
                vs_resize(model.h, index, g.0, g.1)
            }
        }
    }

    func scaleChanged() {
        if let gpu, let host { vt_surface_set_scale(gpu, Double(host.scale)) }
    }

    /// Switches this pane between option A (0) and option B (1).
    func use(_ renderer: UInt32) {
        wantsGPU = renderer == 1
        if renderer == 0, let g = gpu {
            vs_set_pane_renderer(model.h, index, 0)
            vt_surface_free(g)
            gpu = nil
            size = (0, 0)
            layout()
            host?.redraw()
        } else if renderer == 1 {
            layout()
        }
        model.driver?.wake()
    }

    func key(_ code: String, _ mods: UInt16, _ text: String?) {
        if let gpu { vt_surface_key(gpu, code, mods, text) } else { vs_key(model.h, index, code, mods, text) }
    }

    func text(_ s: String) {
        if let gpu { vt_surface_text(gpu, s) } else { vs_text(model.h, index, s) }
    }

    func setMarked(_ s: String) {
        marked = s
        if let gpu { vt_surface_preedit(gpu, s) } else { host?.redraw() }
    }

    func setFocus(_ f: Bool) {
        focused = f
        if f { model.focused = Int(index) }
        if let gpu { vt_surface_set_focus(gpu, f) } else { host?.redraw() }
    }

    /// The cursor's cell in view points (top-left origin).
    func cursorRect() -> CGRect {
        var cx: UInt16 = 0, cy: UInt16 = 0
        var cw = fonts.cell.width, ch = fonts.cell.height
        if let gpu {
            var m = VtMetrics()
            vt_surface_metrics(gpu, &m)
            (cx, cy, cw, ch) = (m.cursor_x, m.cursor_y, m.cell_w, m.cell_h)
        } else if hasView {
            (cx, cy) = (view.cursor_x, view.cursor_y)
        }
        return CGRect(x: padX + CGFloat(cx) * cw, y: padY + CGFloat(cy) * ch, width: cw, height: ch)
    }

    // MARK: Accessibility: the screen as lines of text, cached.

    func screenText() -> String {
        if let a11y { return a11y }
        let p = gpu.map { vt_surface_read_text($0) } ?? vs_read_text(model.h, index)
        let s = p.map { String(cString: $0) } ?? ""
        vs_free_text(p)
        a11y = s
        return s
    }

    var lines: [Substring] { screenText().split(separator: "\n", omittingEmptySubsequences: false) }

    /// The cursor's row.
    var cursorLine: Int {
        let r = cursorRect()
        return max(0, Int(((r.minY - padY) / r.height).rounded()))
    }

    /// UTF-16 range of line `n` in `screenText()`.
    func range(ofLine n: Int) -> NSRange {
        var loc = 0
        for (i, l) in lines.enumerated() {
            let len = l.utf16.count
            if i == n { return NSRange(location: loc, length: len) }
            loc += len + 1
        }
        return NSRange(location: loc, length: 0)
    }

    func line(at index: Int) -> Int {
        var loc = 0
        let ls = lines
        for (i, l) in ls.enumerated() {
            loc += l.utf16.count + 1
            if index < loc { return i }
        }
        return max(0, ls.count - 1)
    }

    /// The cursor as a UTF-16 offset into `screenText()`.
    var cursorIndex: Int {
        let r = range(ofLine: cursorLine)
        let col = Int(((cursorRect().minX - padX) / cursorRect().width).rounded())
        return r.location + min(col, r.length)
    }

    func gpuAction(_ tag: UInt32) {
        if tag == UInt32(VT_CONTENT_CHANGED) { a11y = nil }
    }
}

// MARK: - The model and the frame loop

final class Model {
    let h: OpaquePointer
    var panes: [PaneCore] = []
    var focused = 0
    var app: OpaquePointer?
    weak var driver: Driver?
    let mode: UInt32

    init(h: OpaquePointer) {
        self.h = h
        mode = vs_bench_mode(h)
        panes = (0..<vs_panes(h)).map { PaneCore(model: self, index: $0) }
    }

    /// The GPU renderer's app handle, made on first use.
    func gpuApp() -> OpaquePointer? {
        if app == nil {
            let me = Unmanaged.passUnretained(self).toOpaque()
            app = vt_app_new(h, { ud in
                let m = Unmanaged<Model>.fromOpaque(ud!).takeUnretainedValue()
                DispatchQueue.main.async { if let a = m.app { vt_app_tick(a) } }
            }, { ud, pane, tag in
                let m = Unmanaged<Model>.fromOpaque(ud!).takeUnretainedValue()
                m.panes[Int(pane)].gpuAction(tag)
            }, me)
        }
        return app
    }

    /// The name results are written under: <platform>-a, -b or -mixed.
    var clientName: String {
        if let n = env["VORN_SPIKE_CLIENT"] { return n }
        #if os(macOS)
        let p = "apple"
        #else
        let p = "ios"
        #endif
        let b = panes.filter { $0.wantsGPU }.count
        return p + (b == 0 ? "-a" : b == panes.count ? "-b" : "-mixed")
    }
}

/// Option A's frame loop: a display link that runs while host-drawn panes
/// change, draws them and flushes. Option B's panes pace themselves.
final class Driver: NSObject {
    let model: Model
    var link: CADisplayLink?
    var timer: Timer?
    var idle = 0
    var dirty = [UInt32](repeating: 0, count: 256)
    /// Types a character into pane 0 the way the platform delivers keys.
    var inject: (Character) -> Void = { _ in }
    var finish: () -> Void = { exit(0) }

    init(model: Model) {
        self.model = model
        super.init()
        model.driver = self
    }

    func start(link: CADisplayLink, fps: Int) {
        link.add(to: .main, forMode: .common)
        self.link = link
        let me = Unmanaged.passUnretained(self).toOpaque()
        vs_set_waker(model.h, { ctx in
            let d = Unmanaged<Driver>.fromOpaque(ctx!).takeUnretainedValue()
            DispatchQueue.main.async { d.wake() }
        }, me)
        vs_bench_set_period(model.h, 1000 / Double(max(fps, 1)))
        if model.mode != 0 {
            let t = Timer(timeInterval: 0.004, repeats: true) { [weak self] _ in self?.benchTick() }
            RunLoop.main.add(t, forMode: .common)
            timer = t
        }
    }

    func wake() {
        idle = 0
        link?.isPaused = false
    }

    /// One display refresh: draw the host-drawn panes that changed, now. The
    /// link idles a few frames before pausing, so a wake that lands while
    /// it runs is never lost.
    @objc func frame(_ l: CADisplayLink) {
        let h = model.h
        let n = Int(vs_take_dirty(h, &dirty, UInt32(dirty.count)))
        if n == 0 {
            idle += 1
            if idle > 30 { l.isPaused = true }
            return
        }
        idle = 0
        let t0 = CACurrentMediaTime()
        var hit = false
        var drawn: [UInt32] = []
        for i in 0..<n {
            let p = model.panes[Int(dirty[i])]
            if p.gpu != nil { continue }
            if p.refresh() { hit = true }
            p.host?.redraw()
            drawn.append(p.index)
        }
        if drawn.isEmpty { return }
        CATransaction.flush()
        let t1 = CACurrentMediaTime()
        vs_bench_frame(h, vs_now_ms(h), (t1 - t0) * 1000)
        if hit { vs_bench_hit(h) }
        for p in drawn where model.panes[Int(p)].hasView { vs_pane_shown(h, p) }
    }

    func benchTick() {
        var ch: UInt32 = 0
        switch vs_bench_tick(model.h, &ch) {
        case 1:
            vs_bench_typed(model.h)
            inject(Character(UnicodeScalar(ch)!))
        case 2: inject("\r")
        case 3:
            timer?.invalidate()
            vs_bench_shoot(model.h)
            vs_bench_write_as(model.h, model.clientName)
            finish()
        default: break
        }
    }
}

/// Columns and rows of the bench's pane grid.
func layout(_ n: Int) -> (Int, Int) {
    let cols = min(n, Int(ceil(sqrt(Double(n) * 2))))
    return (cols, (n + cols - 1) / cols)
}
