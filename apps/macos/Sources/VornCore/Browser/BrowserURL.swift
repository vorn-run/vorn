import Foundation

/// The address bar's rules (`browser-url.ts`): what loads for what was typed, and what a tab is called.
public enum BrowserURL {
    public static let blank = "about:blank"
    static let schemeNames: Set<String> = ["javascript", "file", "data", "blob", "vbscript", "about", "chrome"]

    /// What the address bar loads for `input`, or nil for something it will not load.
    /// `file:` only with `allowFile`, with no host, and without its query and hash.
    public static func normalize(_ input: String, allowFile: Bool = false) -> String? {
        let raw = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !raw.isEmpty else { return nil }
        if isHostAndPort(raw) {
            let name = raw[..<raw.firstIndex(of: ":")!].lowercased()
            return schemeNames.contains(name) ? nil : build(raw)
        }
        if hasScheme(raw) {
            guard var url = WebURL(raw) else { return nil }
            switch url.scheme {
            case "about": return raw == blank ? blank : nil
            case "file":
                guard allowFile, (url.host ?? "").isEmpty else { return nil }
                url.query = nil
                url.fragment = nil
                return url.serialized
            case "http", "https": return url.serialized
            default: return nil
            }
        }
        return build(raw.hasPrefix("//") ? String(raw.dropFirst(2)) : raw)
    }

    /// `/^[a-z0-9.-]+:\d+(\/.*)?$/i`: `localhost:3000`, which a URL parser would read as a scheme.
    static func isHostAndPort(_ raw: String) -> Bool {
        guard let colon = raw.firstIndex(of: ":") else { return false }
        let name = raw[..<colon]
        let rest = raw[raw.index(after: colon)...]
        let digits = rest.firstIndex(of: "/").map { rest[..<$0] } ?? rest
        return !name.isEmpty && name.unicodeScalars.allSatisfy { WebURL.isAlnum($0) || $0 == "." || $0 == "-" }
            && !digits.isEmpty && digits.allSatisfy { $0.isASCII && $0.isNumber }
            && !rest[digits.endIndex...].contains("\n")
    }

    static func hasScheme(_ raw: String) -> Bool {
        guard let colon = raw.firstIndex(of: ":") else { return false }
        let name = raw[..<colon].unicodeScalars
        guard let first = name.first, WebURL.isAlpha(first) else { return false }
        return name.allSatisfy { WebURL.isAlnum($0) || $0 == "+" || $0 == "." || $0 == "-" }
    }

    static func build(_ hostAndPath: String) -> String? {
        let scheme = isLocalHost(hostAndPath) ? "http://" : "https://"
        guard let url = WebURL(scheme + hostAndPath), let host = url.host, !host.isEmpty else { return nil }
        return url.serialized
    }

    static func isLocalHost(_ value: String) -> Bool {
        let authority = value.split(separator: "/", omittingEmptySubsequences: false).first.map(String.init) ?? ""
        let host: String
        if authority.hasPrefix("[") {
            host = authority.firstIndex(of: "]").map { String(authority[...$0]).lowercased() } ?? ""
        } else {
            host = (authority.split(separator: ":", omittingEmptySubsequences: false).first.map(String.init) ?? "").lowercased()
        }
        return ["localhost", "127.0.0.1", "0.0.0.0", "[::1]"].contains(host)
            || host.hasSuffix(".local") || host.hasSuffix(".localhost")
    }

    /// What a tab is called before its page has a title.
    public static func displayHost(_ url: String) -> String {
        if url == blank { return "New tab" }
        guard let parsed = WebURL(url) else { return url }
        if parsed.scheme == "file" {
            guard let name = parsed.path.split(separator: "/").last(where: { !$0.isEmpty }) else { return url }
            return WebURL.percentDecode(name) ?? String(name)
        }
        guard let host = parsed.host, !host.isEmpty else { return url }
        return parsed.port.map { "\(host):\($0)" } ?? host
    }

    /// Page text on one line, cut at `max` characters.
    public static func flattenPageText(_ raw: String, max: Int) -> String {
        let spaced = String(String.UnicodeScalarView(raw.unicodeScalars.map {
            $0.properties.generalCategory == .control ? " " : $0
        }))
        let oneLine = spaced.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
        guard oneLine.utf16.count > max else { return oneLine }
        return String(String.UnicodeScalarView(oneLine.unicodeScalars.prefix(max))) + "\u{2026}"
    }

    /// What the agent may load in a session whose directory is `root`: a `file:` address must
    /// resolve, links followed, inside the root.
    public static func loadable(_ input: String, root: String?) -> String? {
        let realRoot = root.flatMap(canonical)
        guard let normalized = normalize(input, allowFile: realRoot != nil) else { return nil }
        guard normalized.hasPrefix("file:") else { return normalized }
        guard let path = filePath(normalized), let real = canonical(path), let realRoot else { return nil }
        return real == realRoot || real.hasPrefix(realRoot.hasSuffix("/") ? realRoot : realRoot + "/") ? normalized : nil
    }

    static func canonical(_ path: String) -> String? {
        guard FileManager.default.fileExists(atPath: path) else { return nil }
        return URL(fileURLWithPath: path).resolvingSymlinksInPath().standardizedFileURL.path
    }

    /// The file a `file:` address shows.
    public static func filePath(_ url: String) -> String? {
        guard let parsed = WebURL(url), parsed.scheme == "file", (parsed.host ?? "").isEmpty else { return nil }
        return WebURL.percentDecode(Substring(parsed.path))
    }

    /// Where a paged read resumes; a cursor from another page generation starts over.
    public static func parseCursor(_ cursor: String?, generation: Int) -> Int {
        guard let cursor, let colon = cursor.firstIndex(of: ":"),
              Int(cursor[..<colon]) == generation else { return 0 }
        return Int(cursor[cursor.index(after: colon)...]) ?? 0
    }

    /// `encodeURIComponent`.
    public static func encodeURIComponent(_ s: String) -> String {
        var out = ""
        for b in s.utf8 {
            let c = Unicode.Scalar(b)
            if WebURL.isAlnum(c) || "-_.!~*'()".unicodeScalars.contains(c) {
                out.unicodeScalars.append(c)
            } else {
                out += String(format: "%%%02X", b)
            }
        }
        return out
    }
}
