import CoreGraphics
import Foundation

/// A published deliverable, as `artifact:get` returns it.
public struct Artifact: Decodable, Equatable, Sendable {
    public var id: String
    public var kind: String
    public var title: String
    public var latestVersion: Int
}

public struct ArtifactVersion: Decodable, Equatable, Sendable {
    public var version: Int
    public var author: String
    public var answersBatchId: String?
    public var createdAt: String

    enum CodingKeys: String, CodingKey { case version, author, answersBatchId, createdAt }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        version = try c.decode(Int.self, forKey: .version)
        author = try c.decode(String.self, forKey: .author)
        answersBatchId = try c.decodeIfPresent(String.self, forKey: .answersBatchId)
        createdAt = try c.decodeIfPresent(String.self, forKey: .createdAt) ?? ""
    }
}

/// Where a comment points.
public enum ArtifactAnchor: Equatable, Sendable {
    case quote(quote: String, prefix: String, suffix: String)
    case point(artboard: String, x: Double, y: Double, element: String)
    case edit(before: String, after: String)

    public init?(_ v: JSONValue) {
        switch v["kind"]?.stringValue {
        case "quote":
            guard let q = v["quote"]?.stringValue else { return nil }
            self = .quote(quote: q, prefix: v["prefix"]?.stringValue ?? "", suffix: v["suffix"]?.stringValue ?? "")
        case "point":
            guard let a = v["artboard"]?.stringValue, let x = v["x"]?.numberValue, let y = v["y"]?.numberValue else { return nil }
            self = .point(artboard: a, x: x, y: y, element: v["element"]?.stringValue ?? "")
        case "edit":
            self = .edit(before: v["before"]?.stringValue ?? "", after: v["after"]?.stringValue ?? "")
        default:
            return nil
        }
    }

    public var json: JSONValue {
        switch self {
        case .quote(let q, let p, let s): ["kind": "quote", "quote": .string(q), "prefix": .string(p), "suffix": .string(s)]
        case .point(let a, let x, let y, let e): ["kind": "point", "artboard": .string(a), "x": .number(x), "y": .number(y), "element": .string(e)]
        case .edit(let b, let a): ["kind": "edit", "before": .string(b), "after": .string(a)]
        }
    }
}

public struct ArtifactComment: Equatable, Sendable, Identifiable {
    public var id: String
    public var version: Int
    public var anchor: ArtifactAnchor?
    public var body: String
    public var state: String
    public var batchId: String?
    public var sentAt: String?

    public init(id: String, version: Int, anchor: ArtifactAnchor?, body: String, state: String, batchId: String? = nil, sentAt: String? = nil) {
        self.id = id; self.version = version; self.anchor = anchor; self.body = body
        self.state = state; self.batchId = batchId; self.sentAt = sentAt
    }

    public init?(_ v: JSONValue) {
        guard let id = v["id"]?.stringValue, let version = v["version"]?.numberValue,
              let body = v["body"]?.stringValue, let state = v["state"]?.stringValue else { return nil }
        self.init(id: id, version: Int(version), anchor: v["anchor"].flatMap(ArtifactAnchor.init), body: body,
                  state: state, batchId: v["batchId"]?.stringValue, sentAt: v["sentAt"]?.stringValue)
    }

    public var isDraft: Bool { state == "draft" }

    /// What the rail quotes for it.
    public var target: String {
        switch anchor {
        case .quote(let q, _, _): q
        case .edit(let b, let a): "\(b) \u{2192} \(a)"
        case .point(let board, _, _, let e): "\(board) \u{b7} \(e)"
        case nil: "The whole version"
        }
    }
}

/// `artifact:get`'s answer: the artifact with its versions and comments.
public struct ArtifactState: Equatable, Sendable {
    public var artifact: Artifact
    public var versions: [ArtifactVersion]
    public var comments: [ArtifactComment]
    public var queued: Bool

    public init?(_ v: JSONValue) {
        guard let a = v["artifact"], let artifact = try? a.decode(Artifact.self) else { return nil }
        self.artifact = artifact
        versions = (v["versions"]?.arrayValue ?? []).compactMap { try? $0.decode(ArtifactVersion.self) }
        comments = (v["comments"]?.arrayValue ?? []).compactMap(ArtifactComment.init)
        queued = v["queued"]?.boolValue ?? false
    }

    public func drafts(_ version: Int) -> [ArtifactComment] {
        comments.filter { $0.isDraft && $0.version == version }
    }

    public func version(_ n: Int) -> ArtifactVersion? { versions.first { $0.version == n } }

    /// The comments `version` answers, and the version they were on.
    public func answered(_ version: Int) -> (comments: [ArtifactComment], on: Int?) {
        guard let batch = self.version(version)?.answersBatchId else { return ([], nil) }
        let answered = comments.filter { $0.batchId == batch }
        return (answered, answered.map(\.version).max())
    }
}

public enum Artifacts {
    public static let popoverWidth = 260.0

    /// Under the selection, or over it when there is no room.
    public static func placePopover(_ rect: CGRect, area: CGSize) -> CGPoint {
        let x = max(min(rect.minX, area.width - popoverWidth - 8), 8)
        let below = rect.minY + rect.height + 8
        let y = below + 150 > area.height ? max(rect.minY - 158, 8) : below
        return CGPoint(x: x, y: y)
    }

    /// The comments sent last, together.
    public static func latestSentBatch(_ comments: [ArtifactComment]) -> [ArtifactComment] {
        var latest: ArtifactComment?
        for c in comments where c.state == "sent" && c.batchId != nil {
            if latest == nil || (c.sentAt ?? "") > (latest?.sentAt ?? "") { latest = c }
        }
        guard let batch = latest?.batchId else { return [] }
        return comments.filter { $0.batchId == batch }
    }

    public struct Mark: Equatable, Sendable {
        public var id, quote, prefix, suffix, state: String
        public var json: JSONValue {
            ["id": .string(id), "quote": .string(quote), "prefix": .string(prefix), "suffix": .string(suffix), "state": .string(state)]
        }
    }

    /// Sent first, so a draft on the same words paints over it.
    public static func marksFor(drafts: [ArtifactComment], sent: [ArtifactComment], focus: String?) -> [Mark] {
        (sent + drafts).compactMap { c in
            guard case .quote(let q, let p, let s) = c.anchor else { return nil }
            let state = c.id == focus ? "focus" : c.isDraft ? "draft" : "sent"
            return Mark(id: c.id, quote: q, prefix: p, suffix: s, state: state)
        }
    }

    public struct DocEdit: Equatable, Sendable {
        public var before: String
        public var after: String
        public init(before: String, after: String) { self.before = before; self.after = after }
        public var json: JSONValue { ["before": .string(before), "after": .string(after)] }
    }

    /// Blocks split on blank lines, trimmed.
    public static func paragraphs(_ md: String) -> [String] {
        var out: [String] = []
        var cur = ""
        func push() {
            let p = cur.trimmingCharacters(in: .whitespacesAndNewlines)
            if !p.isEmpty { out.append(p) }
            cur = ""
        }
        for line in md.split(separator: "\n", omittingEmptySubsequences: false) {
            if line.trimmingCharacters(in: .whitespaces).isEmpty { push(); continue }
            if !cur.isEmpty { cur.append("\n") }
            cur.append(contentsOf: line)
        }
        push()
        return out
    }

    /// Markup-blind: a paragraph re-emphasised is not a paragraph rewritten.
    static func paraKey(_ p: String) -> String {
        let stripped = p.filter { !"*_`#>~\\-".contains($0) }
        return stripped.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
    }

    /// The edited doc as the new version's body, and each run of changed paragraphs as one edit.
    public static func mergeDocEdit(original: String, edited: String) -> (body: String, edits: [DocEdit]) {
        let a = paragraphs(original), b = paragraphs(edited)
        let ka = a.map(paraKey), kb = b.map(paraKey)
        var lcs = Array(repeating: Array(repeating: 0, count: b.count + 1), count: a.count + 1)
        for i in stride(from: a.count - 1, through: 0, by: -1) {
            for j in stride(from: b.count - 1, through: 0, by: -1) {
                lcs[i][j] = ka[i] == kb[j] ? lcs[i + 1][j + 1] + 1 : max(lcs[i + 1][j], lcs[i][j + 1])
            }
        }
        var out: [String] = [], edits: [DocEdit] = [], removed: [String] = [], added: [String] = []
        func flush() {
            if !removed.isEmpty || !added.isEmpty {
                edits.append(DocEdit(before: removed.joined(separator: "\n\n"), after: added.joined(separator: "\n\n")))
            }
            removed = []; added = []
        }
        var i = 0, j = 0
        while i < a.count || j < b.count {
            if i < a.count, j < b.count, ka[i] == kb[j] {
                flush(); out.append(a[i]); i += 1; j += 1
            } else if j < b.count, i == a.count || lcs[i][j + 1] >= lcs[i + 1][j] {
                added.append(b[j]); out.append(b[j]); j += 1
            } else {
                removed.append(a[i]); i += 1
            }
        }
        flush()
        return (out.joined(separator: "\n\n") + "\n", edits)
    }
}

/// A design's canvas: its zoom limits and spacing.
public enum DesignCanvas {
    public static let minZoom = 0.25
    public static let maxZoom = 1.0
    public static let pad = 24.0
    public static let labelHeight = 20.0

    /// Side by side at `zoom`, and the canvas's size.
    public static func place(_ boards: [Artboard], zoom: Double) -> (placed: [CGPoint], width: Double, height: Double) {
        var left = pad, tallest = 0.0
        let placed = boards.map { b -> CGPoint in
            let at = CGPoint(x: left, y: pad + labelHeight)
            left += jsRound(Double(b.width) * zoom) + pad
            tallest = max(tallest, jsRound(Double(b.height) * zoom))
            return at
        }
        return (placed, left, pad * 2 + labelHeight + tallest)
    }

    /// `Math.round`: halves go up, also below zero.
    public static func jsRound(_ x: Double) -> Double { (x + 0.5).rounded(.down) }

    public static func clampZoom(_ zoom: Double) -> Double {
        min(max(jsRound(zoom * 100) / 100, minZoom), maxZoom)
    }

    /// The largest zoom at which every artboard is in view.
    public static func fitZoom(_ boards: [Artboard], area: CGSize) -> Double {
        guard !boards.isEmpty, area.width > 0, area.height > 0 else { return maxZoom }
        let widths = boards.reduce(0.0) { $0 + Double($1.width) }
        let tallest = Double(boards.map(\.height).max() ?? 1)
        let byWidth = (area.width - pad * (Double(boards.count) + 1)) / widths
        let byHeight = (area.height - pad * 2 - labelHeight) / tallest
        return clampZoom((min(byWidth, byHeight) * 100).rounded(.down) / 100)
    }

    /// The page told which artboard it is, in its hash.
    public static func artboardURL(_ url: String, id: String) -> String {
        let base = url.split(separator: "#", maxSplits: 1, omittingEmptySubsequences: false).first.map(String.init) ?? url
        return "\(base)#artboard=\(BrowserURL.encodeURIComponent(id))"
    }
}

public struct Artboard: Equatable, Sendable, Identifiable {
    public var id: String
    public var label: String
    public var width: Int
    public var height: Int
    public init(id: String, label: String, width: Int, height: Int) {
        self.id = id; self.label = label; self.width = width; self.height = height
    }
}

/// One declared input of a design.
public enum Tweak: Equatable, Sendable {
    case number(label: String?, default: Double, unit: String?, min: Double?, max: Double?, step: Double?)
    case boolean(label: String?, default: Bool)
    case color(label: String?, default: String, options: [String]?)
    case select(label: String?, default: String, options: [String])

    public var label: String? {
        switch self {
        case .number(let l, _, _, _, _, _), .boolean(let l, _), .color(let l, _, _), .select(let l, _, _): l
        }
    }

    public var defaultValue: JSONValue {
        switch self {
        case .number(_, let d, _, _, _, _): .number(d)
        case .boolean(_, let d): .bool(d)
        case .color(_, let d, _), .select(_, let d, _): .string(d)
        }
    }

    /// The value if it is of this tweak's type and still among its options.
    public func usable(_ v: JSONValue?) -> JSONValue? {
        guard let v else { return nil }
        switch self {
        case .number: return v.numberValue.map { $0.isFinite ? v : nil } ?? nil
        case .boolean: return v.boolValue == nil ? nil : v
        case .color(_, _, let options):
            guard let s = v.stringValue else { return nil }
            return options.map { $0.contains(s) } ?? true ? v : nil
        case .select(_, _, let options):
            guard let s = v.stringValue else { return nil }
            return options.contains(s) ? v : nil
        }
    }
}

/// What a page says it is, from its `<script id="artifact">` block.
public struct Manifest: Equatable, Sendable {
    public var kind: String
    public var title: String?
    public var tweaks: [(String, Tweak)]
    public var artboards: [Artboard]

    public static func == (a: Manifest, b: Manifest) -> Bool {
        a.kind == b.kind && a.title == b.title && a.artboards == b.artboards
            && a.tweaks.map(\.0) == b.tweaks.map(\.0) && a.tweaks.map(\.1) == b.tweaks.map(\.1)
    }

    static let budget = 16_000
    static let maxTweaks = 24
    static let maxArtboards = 8

    /// The page's declaration, validated, or nil for an ordinary page.
    public static func parse(_ text: String) -> Manifest? {
        guard !text.isEmpty, text.utf16.count <= budget, let root = OrderedJSON.parse(text),
              case .object = root, let kind = root["kind"]?.string, ["page", "doc", "design"].contains(kind)
        else { return nil }
        let title = root["title"]?.string.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .flatMap { $0.isEmpty ? nil : cut($0, 120) }
        var tweaks: [(String, Tweak)] = []
        if case .object(let declared)? = root["tweaks"] {
            for (key, value) in declared {
                if tweaks.count >= maxTweaks { break }
                guard isName(key, max: 40, dash: false), let t = parseTweak(value) else { continue }
                tweaks.append((key, t))
            }
        }
        var artboards: [Artboard] = []
        if kind == "design", case .array(let raw)? = root["artboards"] {
            for b in raw {
                if artboards.count >= maxArtboards { break }
                if let b = parseArtboard(b), !artboards.contains(where: { $0.id == b.id }) { artboards.append(b) }
            }
        }
        return Manifest(kind: kind, title: title, tweaks: tweaks, artboards: artboards)
    }

    static func cut(_ s: String, _ n: Int) -> String { String(s.prefix(n)) }

    static func isName(_ s: String, max: Int, dash: Bool) -> Bool {
        let b = Array(s.unicodeScalars)
        guard let first = b.first, WebURL.isAlpha(first) || first == "_", s.utf8.count <= max else { return false }
        return b.allSatisfy { WebURL.isAlnum($0) || $0 == "_" || (dash && $0 == "-") }
    }

    static func parseTweak(_ raw: OrderedJSON) -> Tweak? {
        guard case .object = raw, let type = raw["type"]?.string, let def = raw["default"] else { return nil }
        let label = raw["label"]?.string.map { cut($0, 40) }.flatMap { $0.isEmpty ? nil : $0 }
        func finite(_ k: String) -> Double? { raw[k]?.number.flatMap { $0.isFinite ? $0 : nil } }
        func strings() -> [String]? {
            guard case .array(let all)? = raw["options"] else { return nil }
            let s = all.compactMap(\.string)
            guard s.count == all.count else { return nil }
            return s.prefix(12).map { cut($0, 40) }
        }
        switch (type, def) {
        case ("number", .number):
            guard let d = finite("default") else { return nil }
            return .number(label: label, default: d, unit: raw["unit"]?.string.map { cut($0, 8) },
                           min: finite("min"), max: finite("max"), step: finite("step"))
        case ("boolean", .bool(let b)): return .boolean(label: label, default: b)
        case ("color", .string(let d)): return .color(label: label, default: cut(d, 40), options: strings())
        case ("select", .string(let d)):
            guard let options = strings(), !options.isEmpty else { return nil }
            let fallback = cut(d, 40)
            return .select(label: label, default: options.contains(fallback) ? fallback : options[0], options: options)
        default: return nil
        }
    }

    static func parseArtboard(_ raw: OrderedJSON) -> Artboard? {
        guard case .object = raw, let id = raw["id"]?.string, isName(id, max: 40, dash: true) else { return nil }
        func size(_ k: String) -> Int? {
            guard let v = raw[k]?.number, v.isFinite, (120...4096).contains(v) else { return nil }
            return Int(DesignCanvas.jsRound(v))
        }
        let label = raw["label"]?.string.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .flatMap { $0.isEmpty ? nil : cut($0, 40) } ?? id
        guard let w = size("width"), let h = size("height") else { return nil }
        return Artboard(id: id, label: label, width: w, height: h)
    }

    /// Per declared tweak: what was set here, else what the page holds, else the default.
    public static func mergeTweaks(_ declared: [(String, Tweak)], stored: [String: JSONValue], live: [String: JSONValue]?) -> [String: JSONValue] {
        var out: [String: JSONValue] = [:]
        for (key, decl) in declared {
            out[key] = decl.usable(stored[key]) ?? decl.usable(live?[key]) ?? decl.defaultValue
        }
        return out
    }

    /// A colour tweak's `#rgb` or `#rrggbb` as `0xrrggbb`.
    public static func hexColor(_ css: String) -> UInt32? {
        let t = css.trimmingCharacters(in: .whitespacesAndNewlines)
        guard t.hasPrefix("#") else { return nil }
        let hex = t.dropFirst()
        guard hex.allSatisfy({ $0.isASCII && $0.isHexDigit }) else { return nil }
        let full: String
        switch hex.count {
        case 3: full = hex.map { "\($0)\($0)" }.joined()
        case 6: full = String(hex)
        default: return nil
        }
        return UInt32(full, radix: 16)
    }
}
