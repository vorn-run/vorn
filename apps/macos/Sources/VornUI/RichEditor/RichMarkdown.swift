import Foundation

/// The inline marks a run of text carries, in the order the serializer wraps them.
public struct RichMarks: OptionSet, Hashable, Sendable {
    public let rawValue: Int
    public init(rawValue: Int) { self.rawValue = rawValue }

    public static let bold = RichMarks(rawValue: 1)
    public static let italic = RichMarks(rawValue: 2)
    public static let strike = RichMarks(rawValue: 4)
    public static let code = RichMarks(rawValue: 8)
}

/// What kind of block a paragraph of the editor is.
public enum RichBlock: Hashable, Sendable {
    case paragraph
    case heading(Int)
    case bullet
    case ordered
    case task(checked: Bool)
    case code(language: String)
    case quote
    case rule

    var isList: Bool {
        switch self {
        case .bullet, .ordered, .task: true
        default: false
        }
    }

    /// Paragraphs of these kinds sharing a group form one node (a list, a code block, a quote).
    var groups: Bool {
        switch self {
        case .bullet, .ordered, .task, .code, .quote: true
        default: false
        }
    }

    func sameNode(_ other: RichBlock) -> Bool {
        switch (self, other) {
        case (.bullet, .bullet), (.ordered, .ordered), (.task, .task), (.code, .code), (.quote, .quote): true
        default: false
        }
    }
}

/// A run of text with the same marks; a hard break is a "\u{2028}" inside the text.
public struct RichRun: Hashable, Sendable {
    public var text: String
    public var marks: RichMarks = []
    public var link: String?

    public init(_ text: String, marks: RichMarks = [], link: String? = nil) {
        self.text = text
        self.marks = marks
        self.link = link
    }
}

/// One line of the editor: a textblock, or one line of a code block.
public struct RichParagraph: Hashable, Sendable {
    public var block: RichBlock
    /// Paragraphs in one list, code block or quote share a group; the next node starts a new one.
    public var group: Int
    public var runs: [RichRun]

    public init(_ block: RichBlock, group: Int = 0, runs: [RichRun] = []) {
        self.block = block
        self.group = group
        self.runs = runs
    }

    public var text: String { runs.map(\.text).joined() }
}

/// markdown.ts markdownToHtml and markdown-utils.ts editorJsonToMarkdown, over paragraphs instead of HTML.
public enum RichMarkdown {
    public static let hardBreak: Character = "\u{2028}"

    // MARK: Markdown to paragraphs

    public static func parse(_ md: String) -> [RichParagraph] {
        guard !md.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return [] }
        let lines = md.components(separatedBy: "\n")
        var out: [RichParagraph] = []
        var group = 0
        var i = 0
        func next() -> Int { group += 1; return group }

        while i < lines.count {
            let line = lines[i]
            if line.hasPrefix("```") {
                let lang = String(line.dropFirst(3)).trimmingCharacters(in: .whitespaces)
                var code: [String] = []
                i += 1
                while i < lines.count, !lines[i].hasPrefix("```") {
                    code.append(lines[i])
                    i += 1
                }
                i += 1
                let g = next()
                for l in code.isEmpty ? [""] : code { out.append(RichParagraph(.code(language: lang), group: g, runs: l.isEmpty ? [] : [RichRun(l)])) }
                continue
            }
            if let m = line.firstMatch(of: #/^(#{1,3}) (.+)$/#) {
                out.append(RichParagraph(.heading(m.1.count), runs: inline(String(m.2))))
                i += 1
                continue
            }
            if line.trimmingCharacters(in: .whitespaces).wholeMatch(of: #/---+/#) != nil {
                out.append(RichParagraph(.rule))
                i += 1
                continue
            }
            if line.hasPrefix("> ") {
                var quoted: [String] = []
                while i < lines.count, lines[i].hasPrefix("> ") {
                    quoted.append(String(lines[i].dropFirst(2)))
                    i += 1
                }
                var runs: [RichRun] = []
                for (n, q) in quoted.enumerated() {
                    if n > 0 { runs.append(RichRun(String(hardBreak))) }
                    runs += inline(q)
                }
                out.append(RichParagraph(.quote, group: next(), runs: merge(runs)))
                continue
            }
            if line.wholeMatch(of: #/- \[([ x])\] (.+)/#) != nil {
                let g = next()
                while i < lines.count, let m = lines[i].wholeMatch(of: #/- \[([ x])\] (.+)/#) {
                    out.append(RichParagraph(.task(checked: m.1 == "x"), group: g, runs: inline(String(m.2))))
                    i += 1
                }
                continue
            }
            if line.firstMatch(of: #/^- .+/#) != nil {
                let g = next()
                while i < lines.count, lines[i].firstMatch(of: #/^- .+/#) != nil {
                    out.append(RichParagraph(.bullet, group: g, runs: inline(String(lines[i].dropFirst(2)))))
                    i += 1
                }
                continue
            }
            if line.firstMatch(of: #/^\d+\. .+/#) != nil {
                let g = next()
                while i < lines.count, lines[i].firstMatch(of: #/^\d+\. .+/#) != nil {
                    out.append(RichParagraph(.ordered, group: g, runs: inline(lines[i].replacing(#/^\d+\. /#, with: ""))))
                    i += 1
                }
                continue
            }
            if line.trimmingCharacters(in: .whitespaces).isEmpty {
                i += 1
                continue
            }
            out.append(RichParagraph(.paragraph, runs: inline(line)))
            i += 1
        }
        return out
    }

    /// inlineMarkdown's regexes, applied in the same order, then read back as marked runs.
    public static func inline(_ text: String) -> [RichRun] {
        let (bo, bc, io, ic, so, sc, co, cc, lo, lm, lc): (Character, Character, Character, Character, Character, Character, Character, Character, Character, Character, Character) =
            ("\u{E000}", "\u{E001}", "\u{E002}", "\u{E003}", "\u{E004}", "\u{E005}", "\u{E006}", "\u{E007}", "\u{E008}", "\u{E009}", "\u{E00A}")
        var s = text
        s = s.replacing(#/\*\*(.+?)\*\*/#) { "\(bo)\($0.1)\(bc)" }
        s = s.replacing(#/\*(.+?)\*/#) { "\(io)\($0.1)\(ic)" }
        s = s.replacing(#/~~(.+?)~~/#) { "\(so)\($0.1)\(sc)" }
        s = s.replacing(#/`([^`]+)`/#) { "\(co)\($0.1)\(cc)" }
        s = s.replacing(#/\[([^\]]+)\]\(([^)]+)\)/#) { m in
            let href = String(m.2)
            let allowed = href.firstMatch(of: #/^(?i:https?|mailto):/#) != nil || href.firstMatch(of: #/^(?i)[a-z][a-z0-9+.-]*:/#) == nil
            return allowed ? "\(lo)\(href)\(lm)\(m.1)\(lc)" : String(m.1)
        }

        var runs: [RichRun] = []
        var marks: RichMarks = []
        var link: String?
        var buffer = ""
        var readingHref = false
        var href = ""
        func flush() {
            guard !buffer.isEmpty else { return }
            runs.append(RichRun(buffer, marks: marks.contains(.code) ? .code : marks, link: marks.contains(.code) ? nil : link))
            buffer = ""
        }
        for ch in s {
            if readingHref {
                if ch == lm { readingHref = false; link = href } else { href.append(ch) }
                continue
            }
            switch ch {
            case bo: flush(); marks.insert(.bold)
            case bc: flush(); marks.remove(.bold)
            case io: flush(); marks.insert(.italic)
            case ic: flush(); marks.remove(.italic)
            case so: flush(); marks.insert(.strike)
            case sc: flush(); marks.remove(.strike)
            case co: flush(); marks.insert(.code)
            case cc: flush(); marks.remove(.code)
            case lo: flush(); readingHref = true; href = ""
            case lc: flush(); link = nil
            default: buffer.append(ch)
            }
        }
        flush()
        return merge(runs)
    }

    /// Joins neighbouring runs that carry the same marks, as ProseMirror keeps text nodes.
    static func merge(_ runs: [RichRun]) -> [RichRun] {
        var out: [RichRun] = []
        for run in runs where !run.text.isEmpty {
            if let last = out.last, last.marks == run.marks, last.link == run.link {
                out[out.count - 1].text += run.text
            } else {
                out.append(run)
            }
        }
        return out
    }

    // MARK: Paragraphs to markdown

    /// editorJsonToMarkdown: blank lines between blocks and between list items, trimmed, one trailing newline.
    public static func serialize(_ paragraphs: [RichParagraph]) -> String {
        let doc = paragraphs.isEmpty ? [RichParagraph(.paragraph)] : paragraphs
        return serializeNodes(nodes(doc)).trimmingCharacters(in: .whitespacesAndNewlines) + "\n"
    }

    /// Paragraphs gathered into the document's nodes: lists, code blocks and quotes by group.
    static func nodes(_ paragraphs: [RichParagraph]) -> [[RichParagraph]] {
        var out: [[RichParagraph]] = []
        for p in paragraphs {
            if p.block.groups, let last = out.last?.last, last.block.sameNode(p.block), last.group == p.group {
                out[out.count - 1].append(p)
            } else {
                out.append([p])
            }
        }
        return out
    }

    static func serializeNodes(_ nodes: [[RichParagraph]]) -> String {
        var parts: [String] = []
        for node in nodes {
            guard let first = node.first else { continue }
            switch first.block {
            case .paragraph:
                parts.append(serializeInline(first.runs) + "\n")
            case .heading(let level):
                parts.append(String(repeating: "#", count: level) + " " + serializeInline(first.runs) + "\n")
            case .bullet:
                for item in node { parts.append("- \(serializeInline(item.runs))\n") }
            case .ordered:
                for (n, item) in node.enumerated() { parts.append("\(n + 1). \(serializeInline(item.runs))\n") }
            case .task:
                for item in node {
                    let checked = item.block == .task(checked: true) ? "x" : " "
                    parts.append("- [\(checked)] \(serializeInline(item.runs))\n")
                }
            case .code(let lang):
                let code = node.map(\.text).joined(separator: "\n")
                parts.append("```\(lang)\n\(code)\n```\n")
            case .quote:
                let inner = serializeNodes(node.map { [RichParagraph(.paragraph, runs: $0.runs)] })
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                parts.append(inner.components(separatedBy: "\n").map { "> \($0)" }.joined(separator: "\n") + "\n")
            case .rule:
                parts.append("---\n")
            }
        }
        return parts.joined(separator: "\n").replacing(#/\n{3,}/#, with: "\n\n")
    }

    static func serializeInline(_ runs: [RichRun]) -> String {
        runs.map { run in
            var text = run.text.replacingOccurrences(of: String(hardBreak), with: "\n")
            if let link = run.link { text = "[\(text)](\(link))" }
            if run.marks.contains(.bold) { text = "**\(text)**" }
            if run.marks.contains(.code) { text = "`\(text)`" }
            if run.marks.contains(.italic) { text = "*\(text)*" }
            if run.marks.contains(.strike) { text = "~~\(text)~~" }
            return text
        }.joined()
    }
}

/// slash-commands.ts: what typing "/" offers.
public struct RichSlashCommand: Hashable, Sendable, Identifiable {
    public let title: String
    public let description: String
    public let glyph: String
    public let block: RichBlock
    public var id: String { title }

    public static let all: [RichSlashCommand] = [
        .init(title: "Heading 1", description: "Large section heading", glyph: "heading1", block: .heading(1)),
        .init(title: "Heading 2", description: "Medium section heading", glyph: "heading2", block: .heading(2)),
        .init(title: "Heading 3", description: "Small section heading", glyph: "heading3", block: .heading(3)),
        .init(title: "Bullet List", description: "Unordered list of items", glyph: "list", block: .bullet),
        .init(title: "Numbered List", description: "Ordered list of items", glyph: "listOrdered", block: .ordered),
        .init(title: "Task List", description: "Checklist with checkboxes", glyph: "squareCheckBig", block: .task(checked: false)),
        .init(title: "Code Block", description: "Fenced code snippet", glyph: "code", block: .code(language: "")),
        .init(title: "Blockquote", description: "Quoted text block", glyph: "quote", block: .quote),
        .init(title: "Divider", description: "Horizontal separator line", glyph: "minus", block: .rule),
    ]

    /// filterSlashCommands: every command, or those whose title or description holds the query.
    public static func filter(_ query: String) -> [RichSlashCommand] {
        guard !query.isEmpty else { return all }
        let q = query.lowercased()
        return all.filter { $0.title.lowercased().contains(q) || $0.description.lowercased().contains(q) }
    }
}
