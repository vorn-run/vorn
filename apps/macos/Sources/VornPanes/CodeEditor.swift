import AppKit
import SwiftUI
import VornCore
import VornUI

/// find.tsx computeMatches: case-insensitive, non-overlapping, per line.
public struct FindMatch: Equatable, Sendable {
    public let line: Int
    public let start: Int
    public let end: Int

    public static func compute(_ lines: [String], _ query: String) -> [FindMatch] {
        guard !query.isEmpty else { return [] }
        let q = Array(query.lowercased().utf16)
        var out: [FindMatch] = []
        for (i, line) in lines.enumerated() {
            let l = Array(line.lowercased().utf16)
            var from = 0
            while from <= l.count - q.count {
                guard let idx = (from...(l.count - q.count)).first(where: { Array(l[$0..<$0 + q.count]) == q }) else { break }
                out.append(FindMatch(line: i, start: idx, end: idx + q.count))
                from = idx + q.count
            }
        }
        return out
    }
}

/// CodeEditor.tsx: a gutter of line numbers, Shiki colour that settles 150ms after typing, find marks, ⌘S.
public struct CodeEditor: NSViewRepresentable {
    @Binding var text: String
    let fileName: String
    var readOnly = false
    var findQuery = ""
    var findIndex = 0
    var onMatches: (Int) -> Void = { _ in }
    var onSave: () -> Void = {}

    static let lineHeight: CGFloat = 21
    static let maxHighlightLines = 2000
    static let font = NSFont.monospacedSystemFont(ofSize: 13, weight: .regular)

    public init(text: Binding<String>, fileName: String, readOnly: Bool = false, findQuery: String = "", findIndex: Int = 0,
                onMatches: @escaping (Int) -> Void = { _ in }, onSave: @escaping () -> Void = {}) {
        _text = text
        self.fileName = fileName
        self.readOnly = readOnly
        self.findQuery = findQuery
        self.findIndex = findIndex
        self.onMatches = onMatches
        self.onSave = onSave
    }

    public func makeCoordinator() -> Coordinator { Coordinator(self) }

    public func makeNSView(context: Context) -> EditorContainer {
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = false
        scroll.borderType = .noBorder
        let tv = EditorTextView()
        tv.onSave = { [weak coordinator = context.coordinator] in coordinator?.parent.onSave() }
        tv.isRichText = false
        tv.allowsUndo = true
        tv.isAutomaticQuoteSubstitutionEnabled = false
        tv.isAutomaticDashSubstitutionEnabled = false
        tv.isAutomaticTextReplacementEnabled = false
        tv.isAutomaticSpellingCorrectionEnabled = false
        tv.isContinuousSpellCheckingEnabled = false
        tv.drawsBackground = false
        tv.font = Self.font
        tv.textColor = NSColor(Theme.gray300)
        tv.insertionPointColor = NSColor(Theme.ink)
        tv.textContainerInset = NSSize(width: 0, height: 4)
        tv.textContainer?.lineFragmentPadding = 0
        tv.isHorizontallyResizable = true
        tv.isVerticallyResizable = true
        tv.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        tv.textContainer?.widthTracksTextView = false
        tv.textContainer?.containerSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        tv.autoresizingMask = [.width]
        let para = NSMutableParagraphStyle()
        para.minimumLineHeight = Self.lineHeight
        para.maximumLineHeight = Self.lineHeight
        tv.defaultParagraphStyle = para
        tv.typingAttributes = [.font: Self.font, .foregroundColor: NSColor(Theme.gray300), .paragraphStyle: para,
                               .baselineOffset: (Self.lineHeight - Self.font.ascender + Self.font.descender) / 2 - 1]
        tv.string = text
        tv.isEditable = !readOnly
        tv.delegate = context.coordinator
        tv.textStorage?.setAttributes(tv.typingAttributes, range: NSRange(location: 0, length: (tv.string as NSString).length))
        scroll.documentView = tv
        let container = EditorContainer(scroll: scroll, textView: tv)
        context.coordinator.textView = tv
        context.coordinator.scheduleHighlight(immediately: true)
        context.coordinator.applyFind()
        return container
    }

    public func updateNSView(_ container: EditorContainer, context: Context) {
        let c = context.coordinator
        let old = c.parent
        c.parent = self
        guard let tv = c.textView else { return }
        tv.isEditable = !readOnly
        if tv.string != text {
            let sel = tv.selectedRanges
            tv.string = text
            tv.textStorage?.setAttributes(tv.typingAttributes, range: NSRange(location: 0, length: (text as NSString).length))
            tv.selectedRanges = sel.filter { $0.rangeValue.upperBound <= (text as NSString).length }
            c.applied = nil
            c.scheduleHighlight(immediately: true)
        }
        if old.findQuery != findQuery || old.findIndex != findIndex || old.fileName != fileName { c.applyFind() }
        container.gutter.refresh()
    }

    @MainActor
    public final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: CodeEditor
        weak var textView: NSTextView?
        var applied: [SyntaxHighlighter.Line]?
        private var pending: Task<Void, Never>?
        private var marked: [NSRange] = []

        init(_ parent: CodeEditor) { self.parent = parent }

        public func textDidChange(_ notification: Notification) {
            guard let tv = textView else { return }
            parent.text = tv.string
            scheduleHighlight(immediately: false)
            applyFind()
            (tv.enclosingScrollView?.superview as? EditorContainer)?.gutter.refresh()
        }

        func scheduleHighlight(immediately: Bool) {
            pending?.cancel()
            guard let lang = SyntaxHighlighter.language(for: parent.fileName) else { return }
            pending = Task { [weak self] in
                if !immediately { try? await Task.sleep(for: .milliseconds(150)) }
                guard !Task.isCancelled, let self, let tv = self.textView else { return }
                let code = tv.string
                guard code.split(separator: "\n", omittingEmptySubsequences: false).count <= CodeEditor.maxHighlightLines else { return }
                let lines = await SyntaxHighlighter.shared.tokens(code, language: lang)
                guard !Task.isCancelled, let lines else { return }
                self.applied = lines
                self.paint()
            }
        }

        /// Colours each line whose tokens still spell it, as CodeEditor.tsx's `inStep` check does.
        func paint() {
            guard let tv = textView, let storage = tv.textStorage, let applied else { return }
            let ns = tv.string as NSString
            storage.beginEditing()
            storage.addAttribute(.foregroundColor, value: NSColor(Theme.gray300), range: NSRange(location: 0, length: ns.length))
            var lineStart = 0
            for (i, line) in tv.string.components(separatedBy: "\n").enumerated() {
                let len = (line as NSString).length
                if let tokens = applied[safe: i], tokens.map(\.content).joined() == line {
                    var at = lineStart
                    for t in tokens {
                        let tl = (t.content as NSString).length
                        if let c = t.color { storage.addAttribute(.foregroundColor, value: NSColor(Color(hex: c, opacity: t.alpha)), range: NSRange(location: at, length: tl)) }
                        at += tl
                    }
                }
                lineStart += len + 1
            }
            storage.endEditing()
            reapplyMarks()
        }

        /// The find bar's marks: all of them faint, the active one solid (find.tsx renderLineWithMarks).
        func applyFind() {
            guard let tv = textView else { return }
            let lines = tv.string.components(separatedBy: "\n")
            let matches = FindMatch.compute(lines, parent.findQuery)
            parent.onMatches(matches.count)
            var starts: [Int] = []
            var acc = 0
            for l in lines { starts.append(acc); acc += (l as NSString).length + 1 }
            let active = matches.isEmpty ? nil : matches[parent.findIndex % matches.count]
            marks = matches.map { (NSRange(location: starts[$0.line] + $0.start, length: $0.end - $0.start), $0 == active) }
            reapplyMarks()
            if let active, let lm = tv.layoutManager, let tc = tv.textContainer {
                let r = NSRange(location: starts[active.line] + active.start, length: active.end - active.start)
                let glyphs = lm.glyphRange(forCharacterRange: r, actualCharacterRange: nil)
                var rect = lm.boundingRect(forGlyphRange: glyphs, in: tc)
                if let clip = tv.enclosingScrollView?.contentView {
                    rect.origin.y -= clip.bounds.height / 2
                    rect.size.height = clip.bounds.height
                }
                tv.scrollToVisible(rect)
            }
        }

        private var marks: [(NSRange, Bool)] = []

        private func reapplyMarks() {
            guard let tv = textView, let storage = tv.textStorage else { return }
            let full = NSRange(location: 0, length: storage.length)
            storage.beginEditing()
            storage.removeAttribute(.backgroundColor, range: full)
            for (r, active) in marks where r.upperBound <= storage.length {
                storage.addAttribute(.backgroundColor, value: NSColor(Theme.amber300.opacity(active ? 0.7 : 0.25)), range: r)
                if active { storage.addAttribute(.foregroundColor, value: NSColor.black, range: r) }
            }
            storage.endEditing()
            marked = marks.map(\.0)
        }
    }
}

/// ⌘S saves rather than reaching the menu.
final class EditorTextView: NSTextView {
    var onSave: () -> Void = {}
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        if event.modifierFlags.intersection(.deviceIndependentFlagsMask) == .command, event.charactersIgnoringModifiers == "s" {
            onSave()
            return true
        }
        return super.performKeyEquivalent(with: event)
    }
}

/// The scroll view with the gutter pinned to its left, as the sticky `<pre>` in CodeEditor.tsx.
public final class EditorContainer: NSView {
    let scroll: NSScrollView
    let gutter: LineGutter

    init(scroll: NSScrollView, textView: NSTextView) {
        self.scroll = scroll
        gutter = LineGutter(textView: textView, clip: scroll.contentView)
        super.init(frame: .zero)
        addSubview(gutter)
        addSubview(scroll)
        scroll.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(gutter, selector: #selector(LineGutter.redraw),
                                               name: NSView.boundsDidChangeNotification, object: scroll.contentView)
    }

    required init?(coder: NSCoder) { fatalError() }

    public override func layout() {
        super.layout()
        gutter.refresh()
        let w = gutter.width
        gutter.frame = NSRect(x: 0, y: 0, width: w, height: bounds.height)
        scroll.frame = NSRect(x: w, y: 0, width: max(0, bounds.width - w), height: bounds.height)
    }

    public override var isFlipped: Bool { true }
}

/// The gutter: right-aligned 12px numbers in gray-600 on the pane surface (`pr-3 pl-2`).
final class LineGutter: NSView {
    private weak var textView: NSTextView?
    private weak var clip: NSClipView?
    private(set) var width: CGFloat = 30

    init(textView: NSTextView, clip: NSClipView) {
        self.textView = textView
        self.clip = clip
        super.init(frame: .zero)
    }

    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    @objc func redraw() { needsDisplay = true }

    func refresh() {
        let lines = max(1, (textView?.string.components(separatedBy: "\n").count) ?? 1)
        let next = 8 + CGFloat(String(lines).count) * 7.3 + 12
        if abs(next - width) > 0.5 {
            width = next
            superview?.needsLayout = true
        }
        needsDisplay = true
    }

    override func draw(_ dirtyRect: NSRect) {
        NSColor(Theme.surfacePanel).setFill()
        bounds.fill()
        guard let tv = textView, let lm = tv.layoutManager, let tc = tv.textContainer, let clip else { return }
        lm.ensureLayout(for: tc)
        let attrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedSystemFont(ofSize: 12, weight: .regular), .foregroundColor: NSColor(Theme.gray600),
        ]
        let ns = tv.string as NSString
        var starts = [0]
        var i = 0
        while i < ns.length {
            if ns.character(at: i) == 10 { starts.append(i + 1) }
            i += 1
        }
        let top = clip.bounds.minY
        for (n, start) in starts.enumerated() {
            let r: NSRect
            if start >= ns.length {
                r = lm.extraLineFragmentRect.height > 0 ? lm.extraLineFragmentRect
                    : NSRect(x: 0, y: CGFloat(n) * CodeEditor.lineHeight, width: 1, height: CodeEditor.lineHeight)
            } else {
                r = lm.lineFragmentRect(forGlyphAt: lm.glyphIndexForCharacter(at: start), effectiveRange: nil)
            }
            let y = r.minY + tv.textContainerInset.height - top
            if y > bounds.height { break }
            if y + r.height < 0 { continue }
            let label = "\(n + 1)" as NSString
            let size = label.size(withAttributes: attrs)
            label.draw(at: NSPoint(x: width - 12 - size.width, y: y + (r.height - size.height) / 2), withAttributes: attrs)
        }
    }
}
