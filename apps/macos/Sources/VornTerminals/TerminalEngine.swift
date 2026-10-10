import CVornGrid
import CoreText
import QuartzCore
import SwiftUI
import VornUI

/// Padding between a terminal's edge and its cells, in points.
public let terminalPadX: CGFloat = 8
public let terminalPadY: CGFloat = 4

// MARK: - Fonts and colours

/// The terminal font in its four variants, with the cell metrics they share.
final class TerminalFonts {
    let regular: CTFont, bold: CTFont, italic: CTFont, boldItalic: CTFont
    let cell: CGSize
    let ascent: CGFloat
    /// Glyphs of printable ASCII per variant: plain runs skip typesetting.
    let ascii: [[CGGlyph]]

    nonisolated(unsafe) private static var cache: [CGFloat: TerminalFonts] = [:]

    static func of(size: CGFloat) -> TerminalFonts {
        if let f = cache[size] { return f }
        let f = TerminalFonts(size: size)
        cache[size] = f
        return f
    }

    private init(size: CGFloat) {
        let base = CTFontCreateWithName(Theme.Terminal.fontName as CFString, size, nil)
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
        // Rows as tall as the browser's line box for the font (15pt for Menlo 13).
        ascent = round(CTFontGetAscent(base))
        let h = round(CTFontGetAscent(base) + CTFontGetDescent(base) + CTFontGetLeading(base))
        cell = CGSize(width: adv.width, height: h)
        ascii = [regular, bold, italic, boldItalic].map { f in
            var chars = (0..<128).map { UniChar($0) }
            var glyphs = [CGGlyph](repeating: 0, count: 128)
            CTFontGetGlyphsForCharacters(f, &chars, &glyphs, 128)
            return glyphs
        }
    }

    func variant(_ flags: UInt16) -> Int {
        (flags & UInt16(VG_BOLD) != 0 ? 1 : 0) + (flags & UInt16(VG_ITALIC) != 0 ? 2 : 0)
    }

    func font(_ flags: UInt16) -> CTFont {
        [regular, bold, italic, boldItalic][variant(flags)]
    }

    /// The grid that fits `size` points inside the padding.
    func grid(for size: CGSize) -> (cols: UInt16, rows: UInt16) {
        (UInt16(clamping: max(2, Int((size.width - 2 * terminalPadX) / cell.width))),
         UInt16(clamping: max(1, Int((size.height - 2 * terminalPadY) / cell.height))))
    }
}

nonisolated(unsafe) private var colorCache: [UInt32: CGColor] = [:]

func cgColor(_ rgb: UInt32, alpha: CGFloat = 1) -> CGColor {
    if alpha == 1, let c = colorCache[rgb] { return c }
    let c = CGColor(srgbRed: CGFloat((rgb >> 16) & 0xff) / 255, green: CGFloat((rgb >> 8) & 0xff) / 255,
                    blue: CGFloat(rgb & 0xff) / 255, alpha: alpha)
    if alpha == 1 { colorCache[rgb] = c }
    return c
}

// MARK: - Painter

/// Draws a grid view with CoreText into a top-down context.
final class TerminalPainter {
    private var glyphs = [CGGlyph](repeating: 0, count: 1024)
    private var positions = [CGPoint](repeating: .zero, count: 1024)

    func draw(_ ctx: CGContext, _ v: VgView?, fonts: TerminalFonts, bounds: CGRect, focused: Bool, marked: String) {
        ctx.setFillColor(cgColor(v?.bg ?? Theme.Terminal.background))
        ctx.fill(bounds)
        guard let v else { return }
        ctx.saveGState()
        defer { ctx.restoreGState() }
        ctx.translateBy(x: terminalPadX, y: terminalPadY)
        let cw = fonts.cell.width, ch = fonts.cell.height
        ctx.textMatrix = .identity
        ctx.setShouldSmoothFonts(true)
        let runs = UnsafeBufferPointer(start: v.runs, count: Int(v.nruns))
        // Backgrounds first, then text, so wide glyphs are not cut.
        for r in runs where r.bg != v.bg {
            ctx.setFillColor(cgColor(r.bg))
            ctx.fill(CGRect(x: CGFloat(r.col) * cw, y: CGFloat(r.row) * ch, width: CGFloat(r.ncols) * cw, height: ch))
        }
        let fgKey = NSAttributedString.Key(kCTForegroundColorAttributeName as String)
        let fontKey = NSAttributedString.Key(kCTFontAttributeName as String)
        for r in runs {
            guard let text = v.text else { break }
            let bytes = UnsafeBufferPointer(start: text + Int(r.text_off), count: Int(r.text_len))
            let fg = cgColor(r.fg, alpha: r.flags & UInt16(VG_FAINT) != 0 ? 0.6 : 1)
            let x = CGFloat(r.col) * cw, y = CGFloat(r.row) * ch
            if r.flags & UInt16(VG_CLUSTER) == 0 {
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
                line(ctx, CTLineCreateWithAttributedString(s), x, y, fonts)
            }
            if r.flags & UInt16(VG_UNDERLINE) != 0 {
                ctx.setFillColor(fg)
                ctx.fill(CGRect(x: x, y: y + ch - 1.5, width: CGFloat(r.ncols) * cw, height: 1))
            }
            if r.flags & UInt16(VG_STRIKE) != 0 {
                ctx.setFillColor(fg)
                ctx.fill(CGRect(x: x, y: y + ch / 2, width: CGFloat(r.ncols) * cw, height: 1))
            }
        }
        let cx = CGFloat(v.cursor_x) * cw, cy = CGFloat(v.cursor_y) * ch
        if !marked.isEmpty {
            let s = NSAttributedString(string: marked, attributes: [fontKey: fonts.regular, fgKey: cgColor(v.fg)])
            let l = CTLineCreateWithAttributedString(s)
            let w = CTLineGetTypographicBounds(l, nil, nil, nil)
            ctx.setFillColor(cgColor(v.bg))
            ctx.fill(CGRect(x: cx, y: cy, width: w, height: ch))
            line(ctx, l, cx, cy, fonts)
            ctx.setFillColor(cgColor(v.fg))
            ctx.fill(CGRect(x: cx, y: cy + ch - 1, width: w, height: 1))
        } else if v.cursor_visible != 0 {
            ctx.setFillColor(cgColor(v.cursor_color, alpha: 0.75))
            switch v.cursor_style {
            case 2: ctx.fill(CGRect(x: cx, y: cy, width: 2, height: ch))
            case 3: ctx.fill(CGRect(x: cx, y: cy + ch - 2, width: cw, height: 2))
            default:
                if focused && v.cursor_style == 0 {
                    ctx.fill(CGRect(x: cx, y: cy, width: cw, height: ch))
                } else {
                    ctx.setStrokeColor(cgColor(v.cursor_color))
                    ctx.stroke(CGRect(x: cx + 0.5, y: cy + 0.5, width: cw - 1, height: ch - 1))
                }
            }
        }
    }

    private func line(_ ctx: CGContext, _ l: CTLine, _ x: CGFloat, _ y: CGFloat, _ fonts: TerminalFonts) {
        ctx.saveGState()
        ctx.translateBy(x: x, y: y + fonts.ascent)
        ctx.scaleBy(x: 1, y: -1)
        ctx.textPosition = .zero
        CTLineDraw(l, ctx)
        ctx.restoreGState()
    }
}

// MARK: - Panes

/// What a pane needs from the view that shows it.
@MainActor
protocol TerminalHost: AnyObject {
    func paneDidChange()
}

/// One attached session: its pane in the grid client and the latest view of it.
@MainActor
public final class TerminalPane {
    public let sessionID: String
    let engine: TerminalEngine
    private(set) var id: UInt32 = 0
    weak var host: TerminalHost?
    private var current = VgView()
    private var hasView = false
    var size: (cols: UInt16, rows: UInt16) = (0, 0)
    var marked = ""
    var focused = false
    /// Whether this client has claimed the session's size since it took focus.
    var ownsSize = false
    /// Cached screen text for accessibility; nil when stale.
    private var screen: String?

    init(engine: TerminalEngine, sessionID: String) {
        self.engine = engine
        self.sessionID = sessionID
    }

    var view: VgView? { hasView ? current : nil }

    /// 0 attaching, 1 live, 2 failed, 3 closed.
    var state: Int32 { id == 0 ? 0 : engine.client.map { vg_pane_state($0, id) } ?? -1 }

    func attach(cols: UInt16, rows: UInt16) {
        guard let c = engine.client else { return }
        if id != 0 { vg_detach(c, id) }
        size = (cols, rows)
        id = vg_attach(c, sessionID, cols, rows)
        if id != 0 { engine.panes[id] = self }
        engine.wake()
        watchAttach()
    }

    private var watcher: Task<Void, Never>?

    /// A session just created joins vornd's engine once its program is up, so
    /// an attach refused meanwhile is tried again for a few seconds.
    private func watchAttach() {
        watcher?.cancel()
        watcher = Task { @MainActor [weak self] in
            for _ in 0..<40 {
                try? await Task.sleep(for: .milliseconds(150))
                guard let self, !Task.isCancelled, self.id != 0 else { return }
                switch self.state {
                case 0: continue
                case 2:
                    guard let c = self.engine.client else { return }
                    vg_detach(c, self.id)
                    self.engine.panes[self.id] = nil
                    self.id = vg_attach(c, self.sessionID, self.size.cols, self.size.rows)
                    if self.id != 0 { self.engine.panes[self.id] = self }
                    self.engine.wake()
                default: return
                }
            }
        }
    }

    func detach() {
        watcher?.cancel()
        if id != 0, let c = engine.client { vg_detach(c, id) }
        engine.panes[id] = nil
        id = 0
        dropView()
    }

    /// Lost with the connection; attach again on the next one.
    func reset() {
        id = 0
        dropView()
    }

    private func dropView() {
        if hasView { vg_view_free(&current) }
        hasView = false
        screen = nil
    }

    /// Takes the pane's latest view; false if there is none.
    @discardableResult
    func refresh() -> Bool {
        guard let c = engine.client, id != 0 else { return false }
        var v = VgView()
        guard vg_view(c, id, &v) else { return false }
        dropView()
        current = v
        hasView = true
        return true
    }

    func resize(cols: UInt16, rows: UInt16) {
        guard (cols, rows) != size else { return }
        size = (cols, rows)
        if id != 0, let c = engine.client { vg_viewport(c, id, cols, rows) }
    }

    // MARK: Input (never sent on a read-only connection)

    private func claimSize() {
        guard !ownsSize, id != 0, let c = engine.client else { return }
        ownsSize = true
        vg_take_size(c, id)
    }

    func key(_ code: String, _ mods: UInt16, _ text: String?) {
        guard !engine.readOnly, id != 0, let c = engine.client else { return }
        claimSize()
        vg_key(c, id, code, mods, text)
    }

    func text(_ s: String) {
        guard !engine.readOnly, id != 0, let c = engine.client else { return }
        claimSize()
        vg_text(c, id, s)
    }

    func setFocus(_ f: Bool) {
        focused = f
        if !f { ownsSize = false }
        if id != 0, let c = engine.client, !engine.readOnly { vg_presence(c, id, f ? 0 : 1) }
        host?.paneDidChange()
    }

    func setMarked(_ s: String) {
        marked = s
        host?.paneDidChange()
    }

    // MARK: Accessibility

    func screenText() -> String {
        if let screen { return screen }
        guard let c = engine.client, id != 0, let p = vg_read_text(c, id) else { return "" }
        let s = String(cString: p)
        vg_free_string(p)
        screen = s
        return s
    }

    func contentChanged() { screen = nil }

    /// The cursor's cell in view points (top-left origin).
    func cursorRect(fonts: TerminalFonts) -> CGRect {
        let (cx, cy) = hasView ? (current.cursor_x, current.cursor_y) : (0, 0)
        return CGRect(x: terminalPadX + CGFloat(cx) * fonts.cell.width, y: terminalPadY + CGFloat(cy) * fonts.cell.height,
                      width: fonts.cell.width, height: fonts.cell.height)
    }
}

// MARK: - Engine

/// The connection to vornd's grid socket and the frame loop that redraws the
/// panes that changed. One per vornd.
@MainActor
public final class TerminalEngine: NSObject {
    public let socket: String
    public let readOnly: Bool
    public let fontSize: CGFloat
    var client: OpaquePointer?
    var panes: [UInt32: TerminalPane] = [:]
    private var link: CADisplayLink?
    private var idle = 0
    private var dirty = [UInt32](repeating: 0, count: 256)

    /// Today's terminal theme, as the grid client resolves colours.
    private static var theme: VgTheme {
        var t = VgTheme()
        t.fg = Theme.Terminal.foreground
        t.bg = Theme.Terminal.background
        t.cursor = Theme.Terminal.cursor
        withUnsafeMutableBytes(of: &t.ansi) { raw in
            let a = raw.bindMemory(to: UInt32.self)
            for (i, c) in Theme.Terminal.ansi.prefix(16).enumerated() { a[i] = c }
        }
        return t
    }

    public init(socket: String, readOnly: Bool, fontSize: CGFloat = Theme.Terminal.fontSize) {
        self.socket = socket
        self.readOnly = readOnly
        self.fontSize = fontSize
        super.init()
        var theme = Self.theme
        let build = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "dev"
        client = vg_connect(socket, "vorn-mac/\(build)", &theme)
        if let client {
            vg_set_waker(client, { ctx in
                let engine = Unmanaged<TerminalEngine>.fromOpaque(ctx!).takeUnretainedValue()
                DispatchQueue.main.async { MainActor.assumeIsolated { engine.wake() } }
            }, Unmanaged.passUnretained(self).toOpaque())
        }
    }

    public var isConnected: Bool { client.map { !vg_closed($0) } ?? false }

    public var lastError: String? {
        guard let client, let p = vg_last_error(client) else { return nil }
        defer { vg_free_string(p) }
        return String(cString: p)
    }

    var fonts: TerminalFonts { TerminalFonts.of(size: fontSize) }

    /// Stops the frame loop and closes the connection; panes keep their sessions to attach again elsewhere.
    public func shutdown() {
        link?.invalidate()
        link = nil
        for p in panes.values { p.reset() }
        panes = [:]
        if let client {
            vg_set_waker(client, nil, nil)
            vg_close(client)
        }
        client = nil
    }

    func makePane(sessionID: String) -> TerminalPane {
        TerminalPane(engine: self, sessionID: sessionID)
    }

    private var snapshots: [String: TerminalPane] = [:]

    /// Attaches a session for an offscreen render, sized to a card body of `size` points.
    public func attachSnapshot(_ sessionID: String, size: CGSize) {
        let g = fonts.grid(for: size)
        let p = snapshots[sessionID] ?? makePane(sessionID: sessionID)
        snapshots[sessionID] = p
        p.attach(cols: g.cols, rows: g.rows)
    }

    func snapshotPane(_ sessionID: String) -> TerminalPane? { snapshots[sessionID] }

    /// The visible text of a snapshot pane, for checks.
    public func snapshotText(_ sessionID: String) -> String? { snapshots[sessionID]?.screenText() }

    /// Something changed: run the frame loop until it goes idle.
    func wake() {
        idle = 0
        if link == nil, let screen = NSScreen.main {
            let l = screen.displayLink(target: self, selector: #selector(frame(_:)))
            l.add(to: .main, forMode: .common)
            link = l
        }
        link?.isPaused = false
    }

    /// One display refresh: redraws the panes that changed. The link idles a few
    /// frames before pausing, so a wake that lands while it runs is not lost.
    @objc private func frame(_ l: CADisplayLink) {
        guard let client else { l.isPaused = true; return }
        var total = 0
        while true {
            let n = Int(vg_take_dirty(client, &dirty, UInt32(dirty.count)))
            total += n
            for i in 0..<n {
                guard let p = panes[dirty[i]] else { continue }
                p.refresh()
                p.contentChanged()
                p.host?.paneDidChange()
            }
            if n < dirty.count { break }
        }
        if total == 0 {
            idle += 1
            if idle > 30 { l.isPaused = true }
        } else {
            idle = 0
        }
    }

    /// Waits until the pane has a view (or `timeout` passes); for snapshots and tests.
    public func settle(timeout: TimeInterval = 3) async {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let client {
                var n = 0
                repeat {
                    n = Int(vg_take_dirty(client, &dirty, UInt32(dirty.count)))
                    for i in 0..<n { panes[dirty[i]]?.refresh() }
                } while n == dirty.count
            }
            if !panes.isEmpty && panes.values.allSatisfy({ $0.view != nil }) { return }
            try? await Task.sleep(for: .milliseconds(50))
        }
    }
}
