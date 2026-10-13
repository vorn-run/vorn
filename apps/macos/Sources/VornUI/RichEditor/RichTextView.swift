#if os(macOS)
import AppKit

extension NSAttributedString.Key {
    static let richBlock = NSAttributedString.Key("vorn.richBlock")
    static let richGroup = NSAttributedString.Key("vorn.richGroup")
    static let richMarks = NSAttributedString.Key("vorn.richMarks")
    static let richLink = NSAttributedString.Key("vorn.richLink")
    static let richIndex = NSAttributedString.Key("vorn.richIndex")
    static let richBefore = NSAttributedString.Key("vorn.richBefore")
    static let richAfter = NSAttributedString.Key("vorn.richAfter")
    static let richLineHeight = NSAttributedString.Key("vorn.richLineHeight")
}

extension RichBlock {
    var code: String {
        switch self {
        case .paragraph: "p"
        case .heading(let l): "h\(l)"
        case .bullet: "ul"
        case .ordered: "ol"
        case .task(let checked): checked ? "taskx" : "task"
        case .code(let lang): "code:" + lang
        case .quote: "quote"
        case .rule: "hr"
        }
    }

    init(code: String?) {
        guard let code else { self = .paragraph; return }
        switch code {
        case "ul": self = .bullet
        case "ol": self = .ordered
        case "task": self = .task(checked: false)
        case "taskx": self = .task(checked: true)
        case "quote": self = .quote
        case "hr": self = .rule
        default:
            if code.hasPrefix("code:") { self = .code(language: String(code.dropFirst(5))) }
            else if code.hasPrefix("h"), let l = Int(code.dropFirst()) { self = .heading(l) }
            else { self = .paragraph }
        }
    }

    var isCode: Bool { if case .code = self { true } else { false } }
}

/// editor-theme.css, as numbers and colours.
enum RichStyle {
    static let text = NSColor(hex: 0xE5E7EB)
    static let strong = NSColor(hex: 0xF3F4F6)
    static let dim = NSColor(hex: 0xD1D5DB)
    static let quote = NSColor(hex: 0x9CA3AF)
    static let muted = NSColor(hex: 0x6B7280)
    static let placeholder = NSColor(hex: 0x4B5563)
    static let link = NSColor(hex: 0x60A5FA)
    static let checked = NSColor(hex: 0x22C55E)
    static let padX: CGFloat = 16
    static let padY: CGFloat = 12
    static let minHeight: CGFloat = 200

    static func font(_ size: CGFloat, weight: NSFont.Weight = .regular, italic: Bool = false, mono: Bool = false) -> NSFont {
        var font = mono ? NSFont.monospacedSystemFont(ofSize: size, weight: weight) : NSFont.systemFont(ofSize: size, weight: weight)
        if italic { font = NSFontManager.shared.convert(font, toHaveTrait: .italicFontMask) }
        return font
    }

    /// Font size, weight, colour and CSS margins (top, bottom) of a block.
    static func metrics(_ block: RichBlock) -> (size: CGFloat, weight: NSFont.Weight, color: NSColor, top: CGFloat, bottom: CGFloat) {
        switch block {
        case .heading(1): (20, .semibold, strong, 12, 4)
        case .heading(2): (15.2, .semibold, text, 12, 4)
        case .heading(3): (12.8, .semibold, dim, 8, 4)
        case .heading: (14, .regular, text, 0, 0)
        case .bullet, .ordered, .task: (14, .regular, dim, 2, 2)
        case .code: (12, .regular, dim, 6, 6)
        case .quote: (14, .regular, quote, 6, 6)
        case .rule: (14, .regular, text, 14, 14)
        case .paragraph: (14, .regular, text, 2, 2)
        }
    }

    /// Where a block's text starts, past its marker, bar or padding.
    static func indent(_ block: RichBlock) -> CGFloat {
        switch block {
        case .bullet, .ordered: 20
        case .task: 23
        case .quote: 14
        case .code: 12
        default: 0
        }
    }
}

extension NSColor {
    convenience init(hex: UInt32, alpha: CGFloat = 1) {
        self.init(srgbRed: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
                  blue: CGFloat(hex & 0xFF) / 255, alpha: alpha)
    }
}

/// Lays lines out at CSS line heights with half-leading, and draws markers, bars and block backgrounds.
final class RichLayoutManager: NSLayoutManager, NSLayoutManagerDelegate {
    override init() {
        super.init()
        delegate = self
    }

    required init?(coder: NSCoder) { fatalError() }

    func layoutManager(_ layoutManager: NSLayoutManager, shouldSetLineFragmentRect lineFragmentRect: UnsafeMutablePointer<NSRect>,
                       lineFragmentUsedRect: UnsafeMutablePointer<NSRect>, baselineOffset: UnsafeMutablePointer<CGFloat>,
                       in textContainer: NSTextContainer, forGlyphRange glyphRange: NSRange) -> Bool
    {
        guard let storage = textStorage, storage.length > 0 else { return false }
        let chars = characterRange(forGlyphRange: glyphRange, actualGlyphRange: nil)
        let at = min(chars.location, storage.length - 1)
        let ns = storage.string as NSString
        let para = ns.paragraphRange(for: NSRange(location: at, length: 0))
        let attrs = storage.attributes(at: at, effectiveRange: nil)
        let lh = (attrs[.richLineHeight] as? CGFloat) ?? 22.4
        let first = chars.location == para.location
        let last = NSMaxRange(chars) >= NSMaxRange(para) || NSMaxRange(chars) >= storage.length
        let before = first ? ((attrs[.richBefore] as? CGFloat) ?? 0) : 0
        let after = last ? ((attrs[.richAfter] as? CGFloat) ?? 0) : 0
        var ascent: CGFloat = 0, descent: CGFloat = 0
        storage.enumerateAttribute(.font, in: chars.length > 0 ? chars : NSRange(location: at, length: 1)) { value, _, _ in
            guard let f = value as? NSFont else { return }
            ascent = max(ascent, f.ascender)
            descent = max(descent, -f.descender)
        }
        if ascent == 0 { ascent = 14 * 0.95; descent = 14 * 0.25 }
        var rect = lineFragmentRect.pointee
        rect.size.height = before + lh + after
        lineFragmentRect.pointee = rect
        var used = lineFragmentUsedRect.pointee
        used.origin.y = rect.origin.y + before
        used.size.height = lh
        lineFragmentUsedRect.pointee = used
        baselineOffset.pointee = before + (lh - ascent - descent) / 2 + ascent
        return true
    }

    override func drawBackground(forGlyphRange glyphsToShow: NSRange, at origin: NSPoint) {
        super.drawBackground(forGlyphRange: glyphsToShow, at: origin)
        guard let storage = textStorage, storage.length > 0, let container = textContainers.first else { return }
        let ns = storage.string as NSString
        let chars = characterRange(forGlyphRange: glyphsToShow, actualGlyphRange: nil)
        let width = container.size.width
        var drawnCodeGroups = Set<Int>()
        var loc = ns.paragraphRange(for: NSRange(location: min(chars.location, storage.length - 1), length: 0)).location
        while loc < min(NSMaxRange(chars) + 1, storage.length) {
            let para = ns.paragraphRange(for: NSRange(location: loc, length: 0))
            defer { loc = NSMaxRange(para) }
            let attrs = storage.attributes(at: para.location, effectiveRange: nil)
            let block = RichBlock(code: attrs[.richBlock] as? String)
            let group = attrs[.richGroup] as? Int ?? 0
            let firstGlyph = glyphIndexForCharacter(at: para.location)
            var lineRange = NSRange()
            let used = lineFragmentUsedRect(forGlyphAt: firstGlyph, effectiveRange: &lineRange)
            let lineTop = used.minY + origin.y
            let lh = (attrs[.richLineHeight] as? CGFloat) ?? 22.4
            switch block {
            case .bullet:
                let d: CGFloat = 5
                let path = NSBezierPath(ovalIn: CGRect(x: origin.x + 20 - 13.5, y: lineTop + lh / 2 - d / 2 + 0.5, width: d, height: d))
                RichStyle.dim.setFill()
                path.fill()
            case .ordered:
                let n = attrs[.richIndex] as? Int ?? 1
                let label = NSAttributedString(string: "\(n).", attributes: [.font: RichStyle.font(14), .foregroundColor: RichStyle.dim])
                let size = label.size()
                label.draw(at: CGPoint(x: origin.x + 20 - 6 - size.width, y: lineTop + (lh - size.height) / 2))
            case .task(let checked):
                let box = CGRect(x: origin.x + 0.75, y: lineTop + lh / 2 - 7.5 + 2.5 + 0.75, width: 13.5, height: 13.5)
                let path = NSBezierPath(roundedRect: box, xRadius: 2.25, yRadius: 2.25)
                path.lineWidth = 1.5
                if checked {
                    RichStyle.checked.setFill()
                    RichStyle.checked.setStroke()
                    path.fill()
                    path.stroke()
                    let tick = NSBezierPath()
                    tick.move(to: CGPoint(x: box.minX + 3.6, y: box.minY + 7.2))
                    tick.line(to: CGPoint(x: box.minX + 6, y: box.minY + 9.6))
                    tick.line(to: CGPoint(x: box.minX + 10.2, y: box.minY + 3.6))
                    tick.lineWidth = 1.5
                    NSColor.white.setStroke()
                    tick.stroke()
                } else {
                    NSColor.white.withAlphaComponent(0.2).setStroke()
                    path.stroke()
                }
            case .quote:
                let lastGlyph = glyphIndexForCharacter(at: max(para.location, NSMaxRange(para) - 1))
                let bottom = lineFragmentUsedRect(forGlyphAt: lastGlyph, effectiveRange: nil).maxY + origin.y
                NSColor.white.withAlphaComponent(0.12).setFill()
                CGRect(x: origin.x, y: lineTop, width: 2, height: bottom - lineTop).fill()
            case .code:
                guard !drawnCodeGroups.contains(group) else { break }
                drawnCodeGroups.insert(group)
                var top = lineTop, bottom = used.maxY + origin.y
                var scan = para.location
                while scan > 0 {
                    let prev = ns.paragraphRange(for: NSRange(location: scan - 1, length: 0))
                    let a = storage.attributes(at: prev.location, effectiveRange: nil)
                    guard RichBlock(code: a[.richBlock] as? String).isCode, a[.richGroup] as? Int == group else { break }
                    top = lineFragmentUsedRect(forGlyphAt: glyphIndexForCharacter(at: prev.location), effectiveRange: nil).minY + origin.y
                    scan = prev.location
                }
                scan = NSMaxRange(para)
                while scan < storage.length {
                    let next = ns.paragraphRange(for: NSRange(location: scan, length: 0))
                    let a = storage.attributes(at: next.location, effectiveRange: nil)
                    guard RichBlock(code: a[.richBlock] as? String).isCode, a[.richGroup] as? Int == group else { break }
                    let g = glyphIndexForCharacter(at: max(next.location, NSMaxRange(next) - 1))
                    bottom = lineFragmentUsedRect(forGlyphAt: g, effectiveRange: nil).maxY + origin.y
                    scan = NSMaxRange(next)
                }
                NSColor.white.withAlphaComponent(0.04).setFill()
                NSBezierPath(roundedRect: CGRect(x: origin.x, y: top - 10, width: width, height: bottom - top + 20), xRadius: 6, yRadius: 6).fill()
            case .rule:
                NSColor.white.withAlphaComponent(0.08).setFill()
                CGRect(x: origin.x, y: lineTop, width: width, height: 1).fill()
            default: break
            }
            // Inline code: a rounded tint under each run.
            storage.enumerateAttribute(.richMarks, in: para) { value, range, _ in
                guard let m = value as? Int, RichMarks(rawValue: m).contains(.code), !block.isCode else { return }
                let glyphs = glyphRange(forCharacterRange: range, actualCharacterRange: nil)
                enumerateEnclosingRects(forGlyphRange: glyphs, withinSelectedGlyphRange: NSRange(location: NSNotFound, length: 0), in: container) { rect, _ in
                    let r = CGRect(x: rect.minX + origin.x - 5, y: rect.midY + origin.y - 9, width: rect.width + 5, height: 18)
                    NSColor.white.withAlphaComponent(0.06).setFill()
                    NSBezierPath(roundedRect: r, xRadius: 4, yRadius: 4).fill()
                }
            }
        }
    }
}

/// What the editor shows floating over itself: the slash menu or the selection toolbar.
struct RichFloating: Equatable {
    enum Kind: Equatable { case slash(query: String, items: [RichSlashCommand], selected: Int), toolbar }
    var kind: Kind
    /// In the text view's coordinates: the caret (slash) or the selection's top centre (toolbar).
    var anchor: CGRect
}

/// The editor's text view: markdown in and out, blocks as paragraph attributes, TipTap's rules and keys.
final class RichTextView: NSTextView {
    var onMarkdown: ((String) -> Void)?
    var onFloating: ((RichFloating?) -> Void)?
    var onFocus: ((Bool) -> Void)?
    var placeholder = ""
    private(set) var markdown = ""
    private var nextGroup = 1
    private var normalizing = false
    private var lastRule: (range: NSRange, before: NSAttributedString, block: Bool)?
    private var slashDismissedAt: Int?
    private(set) var floating: RichFloating?
    var linkEditing = false

    static func make() -> RichTextView {
        let storage = NSTextStorage()
        let layout = RichLayoutManager()
        storage.addLayoutManager(layout)
        let container = NSTextContainer(size: CGSize(width: 300, height: CGFloat.greatestFiniteMagnitude))
        container.widthTracksTextView = true
        container.lineFragmentPadding = 0
        layout.addTextContainer(container)
        let view = RichTextView(frame: .zero, textContainer: container)
        view.drawsBackground = false
        view.isRichText = true
        view.allowsUndo = true
        view.isAutomaticQuoteSubstitutionEnabled = false
        view.isAutomaticDashSubstitutionEnabled = false
        view.isAutomaticTextReplacementEnabled = false
        view.isAutomaticSpellingCorrectionEnabled = false
        view.textContainerInset = CGSize(width: RichStyle.padX, height: RichStyle.padY)
        view.insertionPointColor = RichStyle.text
        view.selectedTextAttributes = [.backgroundColor: NSColor(hex: 0x60A5FA, alpha: 0.25)]
        view.isVerticallyResizable = true
        view.isHorizontallyResizable = false
        view.autoresizingMask = [.width]
        return view
    }

    // MARK: Markdown in and out

    func load(_ md: String) {
        markdown = md
        var paras = RichMarkdown.parse(md)
        if paras.isEmpty { paras = [RichParagraph(.paragraph)] }
        nextGroup = (paras.map(\.group).max() ?? 0) + 1
        let out = NSMutableAttributedString()
        for p in paras {
            var base: [NSAttributedString.Key: Any] = [.richBlock: p.block.code, .richGroup: p.group]
            base[.richMarks] = 0
            for run in p.runs {
                var a = base
                a[.richMarks] = run.marks.rawValue
                if let link = run.link { a[.richLink] = link }
                out.append(NSAttributedString(string: run.text, attributes: a))
            }
            out.append(NSAttributedString(string: "\n", attributes: base))
        }
        normalizing = true
        textStorage?.setAttributedString(out)
        normalizing = false
        normalize()
        undoManager?.removeAllActions()
        setSelectedRange(NSRange(location: 0, length: 0))
    }

    func paragraphs() -> [RichParagraph] {
        guard let storage = textStorage else { return [] }
        let ns = storage.string as NSString
        var out: [RichParagraph] = []
        var loc = 0
        while loc < storage.length {
            let para = ns.paragraphRange(for: NSRange(location: loc, length: 0))
            let a = storage.attributes(at: para.location, effectiveRange: nil)
            var p = RichParagraph(RichBlock(code: a[.richBlock] as? String), group: a[.richGroup] as? Int ?? 0)
            let body = NSRange(location: para.location, length: para.length - (ns.substring(with: para).hasSuffix("\n") ? 1 : 0))
            storage.enumerateAttributes(in: body) { attrs, range, _ in
                p.runs.append(RichRun(ns.substring(with: range), marks: RichMarks(rawValue: attrs[.richMarks] as? Int ?? 0),
                                      link: attrs[.richLink] as? String))
            }
            p.runs = RichMarkdown.merge(p.runs)
            out.append(p)
            loc = NSMaxRange(para)
        }
        return out
    }

    private func emit() {
        let md = RichMarkdown.serialize(paragraphs())
        guard md != markdown else { return }
        markdown = md
        onMarkdown?(md)
    }

    // MARK: Normalising: block attributes per paragraph, then fonts, colours and spacing

    func normalize() {
        guard let storage = textStorage, !normalizing else { return }
        normalizing = true
        defer { normalizing = false }
        let selection = selectedRanges
        storage.beginEditing()
        if storage.length == 0 || !storage.string.hasSuffix("\n") {
            let attrs = storage.length > 0 ? storage.attributes(at: storage.length - 1, effectiveRange: nil) : [.richBlock: "p", .richGroup: 0, .richMarks: 0]
            var a = attrs
            a[.richMarks] = 0
            a.removeValue(forKey: .richLink)
            storage.append(NSAttributedString(string: "\n", attributes: a))
        }
        let ns = storage.string as NSString
        var ranges: [(NSRange, RichBlock, Int)] = []
        var loc = 0
        while loc < storage.length {
            let para = ns.paragraphRange(for: NSRange(location: loc, length: 0))
            let a = storage.attributes(at: para.location, effectiveRange: nil)
            ranges.append((para, RichBlock(code: a[.richBlock] as? String), a[.richGroup] as? Int ?? 0))
            loc = NSMaxRange(para)
        }
        var prevBottom: CGFloat = 0
        var index = 0
        for (i, (para, block, group)) in ranges.enumerated() {
            let prev = i > 0 ? ranges[i - 1] : nil
            let next = i + 1 < ranges.count ? ranges[i + 1] : nil
            let samePrev = prev.map { block.groups && $0.1.sameNode(block) && $0.2 == group } ?? false
            let sameNext = next.map { block.groups && $0.1.sameNode(block) && $0.2 == group } ?? false
            index = samePrev ? index + 1 : 1
            let m = RichStyle.metrics(block)
            var top = m.top, bottom = m.bottom
            switch block {
            case .code:
                top = samePrev ? 0 : max(prevBottom, m.top) + 10
                bottom = sameNext ? 0 : 10
            case .quote:
                top = samePrev ? 0 : max(prevBottom, m.top)
                bottom = sameNext ? 0 : m.bottom
            default:
                top = max(prevBottom, m.top)
            }
            if i == 0 && !block.isCode { top = 0 } else if i == 0 { top = 10 }
            if block.isCode && !sameNext { prevBottom = m.bottom } else if block.isCode { prevBottom = 0 }
            else if block == .quote && sameNext { prevBottom = 0 } else { prevBottom = m.bottom }
            // A task row is as tall as its checkbox with the input's default margins.
            let lh: CGFloat = block == .rule ? 1 : { if case .task = block { 25.4 } else { m.size * 1.6 } }()
            let style = NSMutableParagraphStyle()
            let indent = RichStyle.indent(block)
            style.firstLineHeadIndent = indent
            style.headIndent = indent
            style.tailIndent = block.isCode ? -12 : 0
            let checkedTask = block == .task(checked: true)
            var color = checkedTask ? RichStyle.muted : m.color
            if block == .rule { color = .clear }
            storage.addAttributes([.richBlock: block.code, .richGroup: group, .richIndex: index, .richBefore: top,
                                   .richAfter: block.isCode ? bottom : (i == ranges.count - 1 ? m.bottom : 0),
                                   .richLineHeight: lh, .paragraphStyle: style], range: para)
            storage.enumerateAttributes(in: para) { attrs, range, _ in
                let marks = block.isCode ? RichMarks() : RichMarks(rawValue: attrs[.richMarks] as? Int ?? 0)
                let link = block.isCode ? nil : attrs[.richLink] as? String
                var font: NSFont
                var fg = color
                if block.isCode {
                    font = RichStyle.font(12, mono: true)
                } else if marks.contains(.code) {
                    font = RichStyle.font(12, mono: true)
                    fg = checkedTask ? RichStyle.muted : RichStyle.dim
                } else {
                    let bold = marks.contains(.bold)
                    font = RichStyle.font(m.size, weight: bold ? .semibold : m.weight, italic: marks.contains(.italic))
                    if bold && !checkedTask { fg = RichStyle.strong }
                }
                if marks.contains(.strike), !checkedTask { fg = RichStyle.muted }
                if link != nil { fg = RichStyle.link }
                var add: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: fg, .richMarks: marks.rawValue]
                let strike = marks.contains(.strike) || checkedTask
                add[.strikethroughStyle] = strike ? NSUnderlineStyle.single.rawValue : 0
                add[.underlineStyle] = link != nil ? NSUnderlineStyle.single.rawValue : 0
                if link != nil { add[.underlineColor] = RichStyle.link.withAlphaComponent(0.4) }
                storage.addAttributes(add, range: range)
                if link == nil { storage.removeAttribute(.richLink, range: range) }
                storage.removeAttribute(.kern, range: range)
            }
            // Inline code's 5px side padding, as kerning around the run.
            storage.enumerateAttribute(.richMarks, in: para) { value, range, _ in
                guard !block.isCode, let v = value as? Int, RichMarks(rawValue: v).contains(.code), range.length > 0 else { return }
                storage.addAttribute(.kern, value: 5, range: NSRange(location: NSMaxRange(range) - 1, length: 1))
                if range.location > para.location {
                    storage.addAttribute(.kern, value: 5, range: NSRange(location: range.location - 1, length: 1))
                }
            }
        }
        storage.endEditing()
        selectedRanges = selection
        updateTypingAttributes()
        invalidateIntrinsicContentSize()
        needsDisplay = true
    }

    private func updateTypingAttributes() {
        guard let storage = textStorage, storage.length > 0 else { return }
        let loc = min(selectedRange().location, storage.length - 1)
        let para = (storage.string as NSString).paragraphRange(for: NSRange(location: loc, length: 0))
        var attrs = storage.attributes(at: para.location, effectiveRange: nil)
        if loc > para.location {
            let prev = storage.attributes(at: loc - 1, effectiveRange: nil)
            for key in [NSAttributedString.Key.richMarks, .richLink, .font, .foregroundColor, .strikethroughStyle, .underlineStyle, .underlineColor] {
                attrs[key] = prev[key]
            }
            // Code ends at its run, as ProseMirror's code mark is not inclusive.
            if let m = prev[.richMarks] as? Int, RichMarks(rawValue: m).contains(.code) {
                attrs[.richMarks] = 0
                attrs[.font] = RichStyle.font(RichStyle.metrics(RichBlock(code: attrs[.richBlock] as? String)).size)
            }
            if prev[.richLink] != nil, loc == NSMaxRange(para) - 1 || loc >= storage.length - 1 { attrs.removeValue(forKey: .richLink) }
        }
        attrs.removeValue(forKey: .kern)
        typingAttributes = attrs
    }

    // MARK: Size

    func height(forWidth width: CGFloat) -> CGFloat {
        guard let layout = layoutManager, let container = textContainer else { return RichStyle.minHeight }
        container.size = CGSize(width: max(1, width - 2 * RichStyle.padX), height: .greatestFiniteMagnitude)
        layout.ensureLayout(for: container)
        // The empty line fragment after the closing newline is not part of the document.
        var used = layout.usedRect(for: container).height
        if layout.extraLineFragmentTextContainer != nil { used -= layout.extraLineFragmentUsedRect.height }
        return max(RichStyle.minHeight, ceil(used + 2 * RichStyle.padY))
    }

    override var intrinsicContentSize: NSSize {
        NSSize(width: NSView.noIntrinsicMetric, height: height(forWidth: bounds.width > 0 ? bounds.width : 300))
    }

    // MARK: Placeholder

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard let storage = textStorage, storage.string == "\n", RichBlock(code: storage.attribute(.richBlock, at: 0, effectiveRange: nil) as? String) == .paragraph,
              !placeholder.isEmpty else { return }
        let text = NSAttributedString(string: placeholder, attributes: [.font: RichStyle.font(14), .foregroundColor: RichStyle.placeholder])
        let size = text.size()
        text.draw(at: CGPoint(x: RichStyle.padX, y: RichStyle.padY + (22.4 - size.height) / 2))
    }

    // MARK: Editing hooks

    override func didChangeText() {
        super.didChangeText()
        guard !normalizing else { return }
        normalize()
        emit()
        refreshFloating()
    }

    override func shouldChangeText(in affectedCharRange: NSRange, replacementString: String?) -> Bool {
        lastRule = nil
        return super.shouldChangeText(in: affectedCharRange, replacementString: replacementString)
    }

    override func setSelectedRanges(_ ranges: [NSValue], affinity: NSSelectionAffinity, stillSelecting: Bool) {
        var ranges = ranges
        if let storage = textStorage, storage.length > 0, let first = ranges.first?.rangeValue {
            // The final newline only closes the last paragraph; the caret never goes past it.
            let maxLoc = storage.length - 1
            if first.location > maxLoc { ranges = [NSValue(range: NSRange(location: maxLoc, length: 0))] }
            else if NSMaxRange(first) > maxLoc { ranges = [NSValue(range: NSRange(location: first.location, length: maxLoc - first.location))] }
        }
        super.setSelectedRanges(ranges, affinity: affinity, stillSelecting: stillSelecting)
        if !normalizing {
            updateTypingAttributes()
            refreshFloating()
        }
    }

    override func becomeFirstResponder() -> Bool {
        let ok = super.becomeFirstResponder()
        if ok { onFocus?(true) }
        return ok
    }

    override func resignFirstResponder() -> Bool {
        let ok = super.resignFirstResponder()
        if ok {
            onFocus?(false)
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { [weak self] in
                guard let self, self.window?.firstResponder !== self, !self.linkEditing else { return }
                self.setFloating(nil)
            }
        }
        return ok
    }

    // MARK: Paragraph helpers

    private var storageString: NSString { (textStorage?.string ?? "") as NSString }

    func currentParagraph() -> NSRange {
        let loc = min(selectedRange().location, max(0, (textStorage?.length ?? 1) - 1))
        return storageString.paragraphRange(for: NSRange(location: loc, length: 0))
    }

    func block(at para: NSRange) -> RichBlock {
        guard let storage = textStorage, para.location < storage.length else { return .paragraph }
        return RichBlock(code: storage.attribute(.richBlock, at: para.location, effectiveRange: nil) as? String)
    }

    private func group(at para: NSRange) -> Int {
        guard let storage = textStorage, para.location < storage.length else { return 0 }
        return storage.attribute(.richGroup, at: para.location, effectiveRange: nil) as? Int ?? 0
    }

    private func bodyText(_ para: NSRange) -> String {
        let s = storageString.substring(with: para)
        return s.hasSuffix("\n") ? String(s.dropLast()) : s
    }

    /// Changes a paragraph's block, joining a list or quote directly above it as wrapping input rules do.
    func setBlock(_ block: RichBlock, of para: NSRange, undoName: String? = nil) {
        guard let storage = textStorage else { return }
        var g = 0
        if block.groups {
            let prevLoc = para.location
            if prevLoc > 0 {
                let prev = storageString.paragraphRange(for: NSRange(location: prevLoc - 1, length: 0))
                if self.block(at: prev).sameNode(block) { g = group(at: prev) }
            }
            if g == 0 {
                g = nextGroup
                nextGroup += 1
            }
        }
        if shouldChangeText(in: para, replacementString: nil) {
            storage.addAttributes([.richBlock: block.code, .richGroup: g], range: para)
            if block.isCode { storage.addAttribute(.richMarks, value: 0, range: para) }
            didChangeText()
        }
    }

    /// toggleHeading, toggleBulletList and the rest: on when it is off, back to a paragraph when it is on.
    func toggleBlock(_ block: RichBlock) {
        let para = currentParagraph()
        let current = self.block(at: para)
        if block == .rule {
            insertRule()
            return
        }
        let on = current.sameNode(block) && (current == block || block.isList || block.isCode || block == .quote)
        setBlock(on ? .paragraph : block, of: para)
    }

    /// setHorizontalRule: a divider at the caret, with a paragraph after it to keep typing in.
    func insertRule() {
        guard let storage = textStorage else { return }
        let para = currentParagraph()
        let caret = selectedRange().location
        let text = bodyText(para)
        let base: [NSAttributedString.Key: Any] = [.richBlock: "p", .richGroup: 0, .richMarks: 0]
        let rule = NSAttributedString(string: "\n", attributes: [.richBlock: "hr", .richGroup: 0, .richMarks: 0])
        if text.isEmpty {
            guard shouldChangeText(in: NSRange(location: para.location, length: 0), replacementString: "\n") else { return }
            storage.insert(rule, at: para.location)
            storage.addAttributes(base, range: NSRange(location: para.location + 1, length: max(1, para.length)))
            didChangeText()
            setSelectedRange(NSRange(location: para.location + 1, length: 0))
        } else {
            guard shouldChangeText(in: NSRange(location: caret, length: 0), replacementString: "\n\n") else { return }
            let insert = NSMutableAttributedString(string: "\n", attributes: storage.attributes(at: para.location, effectiveRange: nil))
            insert.append(rule)
            storage.insert(insert, at: caret)
            let after = storageString.paragraphRange(for: NSRange(location: caret + 2, length: 0))
            storage.addAttributes(base, range: after)
            didChangeText()
            setSelectedRange(NSRange(location: caret + 2, length: 0))
        }
    }

    // MARK: Keys

    override func doCommand(by selector: Selector) {
        if case .slash(_, let items, let selected)? = floating?.kind {
            switch selector {
            case #selector(moveUp(_:)):
                if !items.isEmpty { setSlashSelection((selected - 1 + items.count) % items.count) }
                return
            case #selector(moveDown(_:)):
                if !items.isEmpty { setSlashSelection((selected + 1) % items.count) }
                return
            case #selector(insertNewline(_:)):
                if items.indices.contains(selected) { runSlash(items[selected]) }
                return
            case #selector(cancelOperation(_:)):
                slashDismissedAt = slashRange()?.location
                setFloating(nil)
                return
            default: break
            }
        }
        switch selector {
        case #selector(insertNewline(_:)):
            if handleEnter() { return }
        case #selector(insertLineBreak(_:)), #selector(insertNewlineIgnoringFieldEditor(_:)):
            insertText(String(RichMarkdown.hardBreak), replacementRange: selectedRange())
            return
        case #selector(deleteBackward(_:)):
            if handleBackspace() { return }
        case #selector(moveDown(_:)):
            if exitCodeDown() { return }
        default: break
        }
        super.doCommand(by: selector)
    }

    private func handleEnter() -> Bool {
        guard let storage = textStorage, selectedRange().length == 0 else { return false }
        let para = currentParagraph()
        let block = self.block(at: para)
        let text = bodyText(para)
        let caret = selectedRange().location
        switch block {
        case .heading:
            // A new line after a heading is a paragraph.
            insertText("\n", replacementRange: selectedRange())
            let next = currentParagraph()
            setBlock(.paragraph, of: next)
            return true
        case .bullet, .ordered, .task:
            if text.isEmpty {
                setBlock(.paragraph, of: para)
                return true
            }
            insertText("\n", replacementRange: selectedRange())
            if case .task = block {
                let next = currentParagraph()
                if shouldChangeText(in: next, replacementString: nil) {
                    storage.addAttribute(.richBlock, value: RichBlock.task(checked: false).code, range: next)
                    didChangeText()
                }
            }
            return true
        case .quote:
            if text.isEmpty {
                setBlock(.paragraph, of: para)
                return true
            }
            return false
        case .code:
            // exitOnTripleEnter: two empty lines at the end of the block, then Enter, leaves it.
            let isLast = caret == NSMaxRange(para) - 1
            guard isLast, text.isEmpty, para.location > 0 else { return false }
            let prev = storageString.paragraphRange(for: NSRange(location: para.location - 1, length: 0))
            guard self.block(at: prev).isCode, group(at: prev) == group(at: para), bodyText(prev).isEmpty else { return false }
            let nextLoc = NSMaxRange(para)
            if nextLoc < storage.length {
                let next = storageString.paragraphRange(for: NSRange(location: nextLoc, length: 0))
                if self.block(at: next).isCode, group(at: next) == group(at: para) { return false }
            }
            guard shouldChangeText(in: NSRange(location: prev.location, length: 1), replacementString: "") else { return true }
            storage.deleteCharacters(in: NSRange(location: prev.location, length: 1))
            didChangeText()
            let now = storageString.paragraphRange(for: NSRange(location: prev.location, length: 0))
            setBlock(.paragraph, of: now)
            setSelectedRange(NSRange(location: now.location, length: 0))
            return true
        case .rule:
            return true
        case .paragraph:
            return false
        }
    }

    private func handleBackspace() -> Bool {
        guard let storage = textStorage else { return false }
        let range = selectedRange()
        // undoInputRule: the first Backspace after a rule fired puts back what was typed.
        if range.length == 0, let rule = lastRule, NSMaxRange(rule.range) == range.location {
            lastRule = nil
            if shouldChangeText(in: rule.range, replacementString: rule.before.string) {
                storage.replaceCharacters(in: rule.range, with: rule.before)
                if rule.block {
                    storage.addAttributes([.richBlock: "p", .richGroup: 0], range: storageString.paragraphRange(for: NSRange(location: rule.range.location, length: 0)))
                }
                didChangeText()
                setSelectedRange(NSRange(location: rule.range.location + rule.before.length, length: 0))
                return true
            }
        }
        guard range.length == 0 else { return false }
        let para = currentParagraph()
        let block = self.block(at: para)
        guard range.location == para.location else { return false }
        switch block {
        case .bullet, .ordered, .task, .quote, .heading:
            setBlock(.paragraph, of: para)
            return true
        case .code:
            if para.location == 0 || !self.block(at: storageString.paragraphRange(for: NSRange(location: para.location - 1, length: 0))).isCode {
                if bodyText(para).isEmpty {
                    setBlock(.paragraph, of: para)
                    return true
                }
                return para.location == 0
            }
            return false
        default:
            if para.location > 0 {
                let prev = storageString.paragraphRange(for: NSRange(location: para.location - 1, length: 0))
                if self.block(at: prev) == .rule {
                    // selectNodeBackward then delete: the divider goes.
                    if shouldChangeText(in: prev, replacementString: "") {
                        storage.deleteCharacters(in: prev)
                        didChangeText()
                        setSelectedRange(NSRange(location: prev.location, length: 0))
                    }
                    return true
                }
            }
            return false
        }
    }

    private func exitCodeDown() -> Bool {
        guard let storage = textStorage else { return false }
        let para = currentParagraph()
        guard self.block(at: para).isCode, selectedRange().location == NSMaxRange(para) - 1, NSMaxRange(para) >= storage.length else { return false }
        let base: [NSAttributedString.Key: Any] = [.richBlock: "p", .richGroup: 0, .richMarks: 0]
        guard shouldChangeText(in: NSRange(location: storage.length, length: 0), replacementString: "\n") else { return true }
        storage.append(NSAttributedString(string: "\n", attributes: base))
        didChangeText()
        setSelectedRange(NSRange(location: storage.length - 1, length: 0))
        return true
    }

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard window?.firstResponder === self, event.type == .keyDown else { return super.performKeyEquivalent(with: event) }
        let flags = event.modifierFlags.intersection([.command, .shift, .option, .control])
        let key = event.charactersIgnoringModifiers?.lowercased() ?? ""
        switch (flags, key) {
        case ([.command], "b"): toggleMark(.bold)
        case ([.command], "i"): toggleMark(.italic)
        case ([.command], "e"): toggleMark(.code)
        case ([.command, .shift], "s"): toggleMark(.strike)
        case ([.command, .shift], "7"), ([.command, .shift], "&"): toggleBlock(.ordered)
        case ([.command, .shift], "8"), ([.command, .shift], "*"): toggleBlock(.bullet)
        case ([.command, .shift], "9"), ([.command, .shift], "("): toggleBlock(.task(checked: false))
        case ([.command, .shift], "b"): toggleBlock(.quote)
        case ([.command, .option], "c"), ([.command, .option], "ç"): toggleBlock(.code(language: ""))
        case ([.command, .option], "0"): setBlock(.paragraph, of: currentParagraph())
        case ([.command, .option], "1"), ([.command, .option], "¡"): toggleBlock(.heading(1))
        case ([.command, .option], "2"), ([.command, .option], "™"): toggleBlock(.heading(2))
        case ([.command, .option], "3"), ([.command, .option], "£"): toggleBlock(.heading(3))
        default: return super.performKeyEquivalent(with: event)
        }
        return true
    }

    // MARK: Marks

    func isMarkActive(_ mark: RichMarks) -> Bool {
        guard let storage = textStorage else { return false }
        let range = selectedRange()
        guard range.length > 0 else {
            return RichMarks(rawValue: typingAttributes[.richMarks] as? Int ?? 0).contains(mark)
        }
        var all = true
        storage.enumerateAttribute(.richMarks, in: range) { value, sub, stop in
            if (storage.string as NSString).substring(with: sub) == "\n" { return }
            if !RichMarks(rawValue: value as? Int ?? 0).contains(mark) { all = false; stop.pointee = true }
        }
        return all
    }

    var isLinkActive: Bool {
        guard let storage = textStorage, storage.length > 0 else { return false }
        let range = selectedRange()
        let loc = range.length > 0 ? range.location : max(0, range.location - 1)
        return storage.attribute(.richLink, at: min(loc, storage.length - 1), effectiveRange: nil) != nil
    }

    var currentLink: String? {
        guard let storage = textStorage, storage.length > 0 else { return nil }
        return storage.attribute(.richLink, at: min(selectedRange().location, storage.length - 1), effectiveRange: nil) as? String
    }

    func toggleMark(_ mark: RichMarks) {
        guard let storage = textStorage else { return }
        let range = selectedRange()
        let on = !isMarkActive(mark)
        guard range.length > 0 else {
            var marks = RichMarks(rawValue: typingAttributes[.richMarks] as? Int ?? 0)
            if on { marks = mark == .code ? .code : marks.subtracting(.code).union(mark) } else { marks.remove(mark) }
            typingAttributes[.richMarks] = marks.rawValue
            return
        }
        guard shouldChangeText(in: range, replacementString: nil) else { return }
        storage.enumerateAttribute(.richMarks, in: range) { value, sub, _ in
            var marks = RichMarks(rawValue: value as? Int ?? 0)
            if on { marks = mark == .code ? .code : marks.subtracting(.code).union(mark) } else { marks.remove(mark) }
            storage.addAttribute(.richMarks, value: marks.rawValue, range: sub)
            if on && mark == .code { storage.removeAttribute(.richLink, range: sub) }
        }
        didChangeText()
        setSelectedRange(range)
    }

    func setLink(_ href: String?) {
        guard let storage = textStorage else { return }
        let range = selectedRange()
        guard range.length > 0, shouldChangeText(in: range, replacementString: nil) else { return }
        if let href { storage.addAttribute(.richLink, value: href, range: range) } else { storage.removeAttribute(.richLink, range: range) }
        didChangeText()
        setSelectedRange(range)
    }

    // MARK: Input rules

    override func insertText(_ string: Any, replacementRange: NSRange) {
        super.insertText(string, replacementRange: replacementRange)
        guard let typed = (string as? String) ?? (string as? NSAttributedString)?.string, typed.count == 1 else { return }
        applyInputRules(typed)
    }

    private func applyInputRules(_ typed: String) {
        guard let storage = textStorage else { return }
        let caret = selectedRange().location
        let para = currentParagraph()
        let block = self.block(at: para)
        guard !block.isCode || typed == " " || typed == "\n" else { return }
        let before = storageString.substring(with: NSRange(location: para.location, length: caret - para.location))
        let replaceToCaret = NSRange(location: para.location, length: caret - para.location)

        func blockRule(_ new: RichBlock) {
            let saved = storage.attributedSubstring(from: replaceToCaret)
            guard shouldChangeText(in: replaceToCaret, replacementString: "") else { return }
            storage.deleteCharacters(in: replaceToCaret)
            didChangeText()
            let p = storageString.paragraphRange(for: NSRange(location: para.location, length: 0))
            setBlock(new, of: p)
            setSelectedRange(NSRange(location: para.location, length: 0))
            lastRule = (NSRange(location: para.location, length: 0), saved, true)
        }

        if !block.isCode {
            if let m = before.wholeMatch(of: #/(#{1,6})\s/#), block != .heading(m.1.count) { return blockRule(.heading(m.1.count)) }
            if before.wholeMatch(of: #/\s*[-+*]\s/#) != nil, !block.isList { return blockRule(.bullet) }
            if before.wholeMatch(of: #/\d+\.\s/#) != nil, block != .ordered { return blockRule(.ordered) }
            if let m = before.wholeMatch(of: #/\s*\[([( |x])?\]\s/#) { return blockRule(.task(checked: m.1 == "x")) }
            if before.wholeMatch(of: #/\s*>\s/#) != nil, block != .quote { return blockRule(.quote) }
            if let m = before.wholeMatch(of: #/```([a-z]+)?[\s\n]/#) ?? before.wholeMatch(of: #/~~~([a-z]+)?[\s\n]/#) {
                return blockRule(.code(language: m.1.map(String.init) ?? ""))
            }
            if before.wholeMatch(of: #/---|—-|___\s|\*\*\*\s/#) != nil {
                guard shouldChangeText(in: replaceToCaret, replacementString: "") else { return }
                storage.deleteCharacters(in: replaceToCaret)
                didChangeText()
                setSelectedRange(NSRange(location: para.location, length: 0))
                insertRule()
                return
            }
        }
        guard !block.isCode else { return }
        markRule(before, para: para, pattern: #/(?:^|\s)(\*\*(?!\s+\*\*)([^*]+)\*\*(?!\s+\*\*))$/#, mark: .bold)
        markRule(before, para: para, pattern: #/(?:^|\s)(__(?!\s+__)([^_]+)__(?!\s+__))$/#, mark: .bold)
        markRule(before, para: para, pattern: #/(?:^|\s)(\*(?!\s+\*)([^*]+)\*(?!\s+\*))$/#, mark: .italic)
        markRule(before, para: para, pattern: #/(?:^|\s)(_(?!\s+_)([^_]+)_(?!\s+_))$/#, mark: .italic)
        markRule(before, para: para, pattern: #/(?:^|\s)(~~(?!\s+~~)([^~]+)~~(?!\s+~~))$/#, mark: .strike)
        markRule(before, para: para, pattern: #/(?:^|[^`])(`([^`]+)`)$/#, mark: .code)
    }

    /// markInputRule: the delimiters go and the text between them takes the mark.
    private func markRule(_ before: String, para: NSRange, pattern: Regex<(Substring, Substring, Substring)>, mark: RichMarks) {
        guard let storage = textStorage, let m = before.firstMatch(of: pattern) else { return }
        let whole = NSRange(m.1.startIndex..<m.1.endIndex, in: before)
        let inner = NSRange(m.2.startIndex..<m.2.endIndex, in: before)
        let fullRange = NSRange(location: para.location + whole.location, length: whole.length)
        let saved = storage.attributedSubstring(from: fullRange)
        let text = (before as NSString).substring(with: inner)
        var attrs = storage.attributes(at: fullRange.location, effectiveRange: nil)
        var marks = RichMarks(rawValue: attrs[.richMarks] as? Int ?? 0)
        marks = mark == .code ? .code : marks.subtracting(.code).union(mark)
        attrs[.richMarks] = marks.rawValue
        guard shouldChangeText(in: fullRange, replacementString: text) else { return }
        storage.replaceCharacters(in: fullRange, with: NSAttributedString(string: text, attributes: attrs))
        didChangeText()
        let end = fullRange.location + (text as NSString).length
        setSelectedRange(NSRange(location: end, length: 0))
        var typing = typingAttributes
        typing[.richMarks] = RichMarks(rawValue: marks.rawValue).subtracting(mark).rawValue
        typingAttributes = typing
        lastRule = (NSRange(location: fullRange.location, length: (text as NSString).length), saved, false)
    }

    // MARK: Checkboxes

    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        if let para = taskBox(at: point), case .task(let checked) = block(at: para) {
            if shouldChangeText(in: para, replacementString: nil) {
                textStorage?.addAttribute(.richBlock, value: RichBlock.task(checked: !checked).code, range: para)
                didChangeText()
            }
            return
        }
        super.mouseDown(with: event)
    }

    private func taskBox(at point: CGPoint) -> NSRange? {
        guard let layout = layoutManager, let container = textContainer, let storage = textStorage, storage.length > 0 else { return nil }
        let p = CGPoint(x: point.x - textContainerOrigin.x, y: point.y - textContainerOrigin.y)
        guard p.x >= -2, p.x <= 17 else { return nil }
        let glyph = layout.glyphIndex(for: p, in: container)
        let char = layout.characterIndexForGlyph(at: glyph)
        let para = storageString.paragraphRange(for: NSRange(location: min(char, storage.length - 1), length: 0))
        guard case .task = block(at: para) else { return nil }
        let used = layout.lineFragmentUsedRect(forGlyphAt: layout.glyphIndexForCharacter(at: para.location), effectiveRange: nil)
        return p.y >= used.minY && p.y <= used.minY + 21 ? para : nil
    }

    // MARK: Floating UI

    private func setFloating(_ f: RichFloating?) {
        guard f != floating || f?.kind == .toolbar else { return }
        floating = f
        onFloating?(f)
    }

    /// The "/" and query before the caret, as Suggestion finds it: after a space or at the line start, no spaces in it.
    func slashRange() -> NSRange? {
        let range = selectedRange()
        guard range.length == 0 else { return nil }
        let para = currentParagraph()
        let before = storageString.substring(with: NSRange(location: para.location, length: range.location - para.location))
        guard let m = before.firstMatch(of: #/(?:^|\s)(\/[^\s\/]*)$/#) else { return nil }
        let r = NSRange(m.1.startIndex..<m.1.endIndex, in: before)
        return NSRange(location: para.location + r.location, length: r.length)
    }

    private func caretRect(at loc: Int) -> CGRect {
        guard let layout = layoutManager, let container = textContainer, let storage = textStorage, storage.length > 0 else { return .zero }
        let glyph = layout.glyphIndexForCharacter(at: min(loc, storage.length - 1))
        let line = layout.lineFragmentUsedRect(forGlyphAt: glyph, effectiveRange: nil)
        let x = layout.location(forGlyphAt: glyph).x
        let rect = layout.boundingRect(forGlyphRange: NSRange(location: glyph, length: 0), in: container)
        return CGRect(x: (rect.width == 0 ? x + line.minX : rect.minX) + textContainerOrigin.x, y: line.minY + textContainerOrigin.y, width: 1, height: line.height)
    }

    func refreshFloating() {
        let range = selectedRange()
        if let slash = slashRange(), slash.location != slashDismissedAt {
            let query = String(storageString.substring(with: slash).dropFirst())
            let items = RichSlashCommand.filter(query)
            var selected = 0
            if case .slash(let q, _, let s)? = floating?.kind, q == query { selected = s }
            let caret = caretRect(at: slash.location)
            setFloating(items.isEmpty ? nil : RichFloating(kind: .slash(query: query, items: items, selected: min(selected, max(0, items.count - 1))), anchor: caret))
            return
        }
        if slashRange() == nil { slashDismissedAt = nil }
        if range.length > 0, !block(at: storageString.paragraphRange(for: NSRange(location: range.location + range.length - 1, length: 0))).isCode,
           !(textStorage?.attributedSubstring(from: range).string.allSatisfy { $0 == "\n" } ?? true)
        {
            let start = caretRect(at: range.location)
            let end = caretRect(at: NSMaxRange(range))
            let top = min(start.minY, end.minY)
            setFloating(RichFloating(kind: .toolbar, anchor: CGRect(x: (start.minX + end.minX) / 2, y: top, width: 0, height: 0)))
            return
        }
        if !linkEditing { setFloating(nil) }
    }

    private func setSlashSelection(_ i: Int) {
        guard case .slash(let q, let items, _)? = floating?.kind, let f = floating else { return }
        setFloating(RichFloating(kind: .slash(query: q, items: items, selected: i), anchor: f.anchor))
    }

    func runSlash(_ command: RichSlashCommand) {
        guard let storage = textStorage, let range = slashRange() else { return }
        if shouldChangeText(in: range, replacementString: "") {
            storage.deleteCharacters(in: range)
            didChangeText()
            setSelectedRange(NSRange(location: range.location, length: 0))
        }
        setFloating(nil)
        toggleBlock(command.block)
    }
}
#endif
