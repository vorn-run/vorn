import Foundation

/// The parts of a URL the browser pane reads, parsed and serialized as the web's URL standard does for
/// `http`, `https` and `file` (the address bar loads nothing else); any other scheme is kept opaque.
public struct WebURL: Equatable, Sendable {
    public var scheme: String
    public var username = ""
    public var password = ""
    /// The serialized host: lowercase, IPv6 in brackets; nil for an opaque URL.
    public var host: String?
    /// The port, or nil when absent or the scheme's default.
    public var port: Int?
    public var path: String
    public var query: String?
    public var fragment: String?

    static let defaultPorts: [String: Int] = ["http": 80, "https": 443, "ws": 80, "wss": 443, "ftp": 21]
    public var isSpecial: Bool { Self.defaultPorts[scheme] != nil || scheme == "file" }

    public var serialized: String {
        var out = scheme + ":"
        if let host {
            out += "//"
            if !username.isEmpty || !password.isEmpty {
                out += username + (password.isEmpty ? "" : ":" + password) + "@"
            }
            out += host
            if let port { out += ":\(port)" }
        }
        out += path
        if let query { out += "?" + query }
        if let fragment { out += "#" + fragment }
        return out
    }

    // MARK: Parsing

    public init?(_ input: String) {
        var scalars = Array(input.unicodeScalars)
        while let f = scalars.first, f.value <= 0x20 { scalars.removeFirst() }
        while let l = scalars.last, l.value <= 0x20 { scalars.removeLast() }
        scalars.removeAll { $0 == "\t" || $0 == "\n" || $0 == "\r" }
        let s = String(String.UnicodeScalarView(scalars))
        guard let colon = s.firstIndex(of: ":") else { return nil }
        let name = s[..<colon]
        guard let first = name.unicodeScalars.first, Self.isAlpha(first),
              name.unicodeScalars.allSatisfy({ Self.isAlnum($0) || $0 == "+" || $0 == "-" || $0 == "." }) else { return nil }
        scheme = name.lowercased()
        let rest = Substring(s[s.index(after: colon)...])
        path = ""
        if scheme == "file" {
            guard Self.parseFile(rest, into: &self) else { return nil }
        } else if isSpecial {
            guard Self.parseSpecial(rest, into: &self) else { return nil }
        } else {
            path = String(rest)
        }
    }

    private static func splitTail(_ rest: Substring, into url: inout WebURL) -> Substring {
        var body = rest
        if let hash = body.firstIndex(of: "#") {
            url.fragment = encode(body[body.index(after: hash)...], fragmentSet)
            body = body[..<hash]
        }
        if let q = body.firstIndex(of: "?") {
            url.query = encode(body[body.index(after: q)...], querySet)
            body = body[..<q]
        }
        return body
    }

    private static func parseSpecial(_ input: Substring, into url: inout WebURL) -> Bool {
        var rest = input
        while let f = rest.first, f == "/" || f == "\\" { rest.removeFirst() }
        let end = rest.firstIndex { $0 == "/" || $0 == "\\" || $0 == "?" || $0 == "#" } ?? rest.endIndex
        var authority = rest[..<end]
        let remainder = rest[end...]
        if let at = authority.lastIndex(of: "@") {
            let info = authority[..<at]
            authority = authority[authority.index(after: at)...]
            if let c = info.firstIndex(of: ":") {
                url.username = encode(info[..<c], userinfoSet)
                url.password = encode(info[info.index(after: c)...], userinfoSet)
            } else {
                url.username = encode(info, userinfoSet)
            }
        }
        var hostPart = authority
        var portPart: Substring?
        var inBrackets = false
        for i in authority.indices {
            let ch = authority[i]
            if ch == "[" { inBrackets = true }
            if ch == "]" { inBrackets = false }
            if ch == ":" && !inBrackets {
                hostPart = authority[..<i]
                portPart = authority[authority.index(after: i)...]
                break
            }
        }
        guard !hostPart.isEmpty, let host = parseHost(hostPart) else { return false }
        url.host = host
        if let portPart, !portPart.isEmpty {
            guard portPart.allSatisfy({ $0.isASCII && $0.isNumber }), let n = Int(portPart), n <= 65535 else { return false }
            url.port = n == defaultPorts[url.scheme] ? nil : n
        }
        let body = splitTail(remainder, into: &url)
        url.path = normalizePath(body)
        return true
    }

    private static func parseFile(_ input: Substring, into url: inout WebURL) -> Bool {
        var body = splitTail(input, into: &url)
        url.host = ""
        if body.hasPrefix("//") || body.hasPrefix("\\\\") || body.hasPrefix("/\\") || body.hasPrefix("\\/") {
            body = body.dropFirst(2)
            let end = body.firstIndex { $0 == "/" || $0 == "\\" } ?? body.endIndex
            let raw = body[..<end]
            body = body[end...]
            if !raw.isEmpty {
                guard let host = parseHost(raw) else { return false }
                url.host = host == "localhost" ? "" : host
            }
        }
        url.path = normalizePath(body)
        return true
    }

    private static func normalizePath(_ raw: Substring) -> String {
        var segments: [String] = []
        let parts = raw.split(omittingEmptySubsequences: false) { $0 == "/" || $0 == "\\" }
        let list = raw.first == "/" || raw.first == "\\" ? Array(parts.dropFirst()) : Array(parts)
        for (i, part) in list.enumerated() {
            let last = i == list.count - 1
            let lower = part.lowercased()
            if [".", "%2e"].contains(lower) {
                if last { segments.append("") }
            } else if ["..", ".%2e", "%2e.", "%2e%2e"].contains(lower) {
                if !segments.isEmpty { segments.removeLast() }
                if last { segments.append("") }
            } else {
                segments.append(encode(part, pathSet))
            }
        }
        return "/" + segments.joined(separator: "/")
    }

    // MARK: Hosts

    private static func parseHost(_ raw: Substring) -> String? {
        if raw.first == "[" {
            guard raw.last == "]", let pieces = parseIPv6(raw.dropFirst().dropLast()) else { return nil }
            return "[" + serializeIPv6(pieces) + "]"
        }
        guard let decoded = percentDecode(raw) else { return nil }
        let domain = decoded.lowercased()
        let forbidden: Set<Character> = ["\0", "\t", "\n", "\r", " ", "#", "/", ":", "<", ">", "?", "@", "[", "\\", "]", "^", "|", "%"]
        guard !domain.isEmpty, !domain.contains(where: { forbidden.contains($0) }),
              !domain.unicodeScalars.contains(where: { $0.value < 0x20 || $0.value == 0x7F }) else { return nil }
        var labels = domain.split(separator: ".", omittingEmptySubsequences: false)
        if labels.last == "" && labels.count > 1 { labels.removeLast() }
        if let last = labels.last, !last.isEmpty,
           last.allSatisfy({ $0.isASCII && $0.isNumber }) || (last.hasPrefix("0x") && last.dropFirst(2).allSatisfy(\.isHexDigit)) {
            return parseIPv4(domain)
        }
        return domain
    }

    private static func parseIPv4(_ s: String) -> String? {
        var parts = s.split(separator: ".", omittingEmptySubsequences: false)
        if parts.last == "" { parts.removeLast() }
        guard !parts.isEmpty, parts.count <= 4 else { return nil }
        var numbers: [UInt64] = []
        for p in parts {
            guard !p.isEmpty else { return nil }
            var text = Substring(p)
            var radix = 10
            if text.count >= 2, text.lowercased().hasPrefix("0x") {
                text = text.dropFirst(2)
                radix = 16
            } else if text.count >= 2, text.hasPrefix("0") {
                text = text.dropFirst()
                radix = 8
            }
            if text.isEmpty { numbers.append(0); continue }
            guard text.count <= 12, let n = UInt64(text, radix: radix) else { return nil }
            numbers.append(n)
        }
        for n in numbers.dropLast() where n > 255 { return nil }
        guard let last = numbers.last, last < UInt64(1) << (8 * (5 - numbers.count)) else { return nil }
        var value = last
        for (i, n) in numbers.dropLast().enumerated() { value += n << (8 * (3 - i)) }
        return (0..<4).map { String((value >> (8 * (3 - $0))) & 255) }.joined(separator: ".")
    }

    private static func parseIPv6(_ s: Substring) -> [UInt16]? {
        let halves = s.components(separatedBy: "::")
        guard halves.count <= 2 else { return nil }
        func groups(_ text: String) -> [UInt16]? {
            if text.isEmpty { return [] }
            var out: [UInt16] = []
            let parts = text.split(separator: ":", omittingEmptySubsequences: false)
            for (i, p) in parts.enumerated() {
                if i == parts.count - 1, p.contains(".") {
                    guard let v4 = parseIPv4(String(p)) else { return nil }
                    let b = v4.split(separator: ".").compactMap { UInt16($0) }
                    out.append(b[0] << 8 | b[1])
                    out.append(b[2] << 8 | b[3])
                } else {
                    guard (1...4).contains(p.count), let n = UInt16(p, radix: 16) else { return nil }
                    out.append(n)
                }
            }
            return out
        }
        guard let head = groups(halves[0]) else { return nil }
        if halves.count == 1 { return head.count == 8 ? head : nil }
        guard let tail = groups(halves[1]), head.count + tail.count <= 7 else { return nil }
        return head + Array(repeating: 0, count: 8 - head.count - tail.count) + tail
    }

    private static func serializeIPv6(_ p: [UInt16]) -> String {
        var best = (start: -1, len: 1)
        var i = 0
        while i < 8 {
            if p[i] == 0 {
                var j = i
                while j < 8 && p[j] == 0 { j += 1 }
                if j - i > best.len { best = (i, j - i) }
                i = j
            } else {
                i += 1
            }
        }
        var out = ""
        var k = 0
        while k < 8 {
            if k == best.start {
                out += k == 0 ? "::" : ":"
                k += best.len
                continue
            }
            out += String(p[k], radix: 16)
            if k < 7 { out += ":" }
            k += 1
        }
        return out
    }

    // MARK: Encoding

    static func isAlpha(_ c: Unicode.Scalar) -> Bool { (c >= "a" && c <= "z") || (c >= "A" && c <= "Z") }
    static func isAlnum(_ c: Unicode.Scalar) -> Bool { isAlpha(c) || (c >= "0" && c <= "9") }

    static let fragmentSet: Set<UInt8> = [0x20, 0x22, 0x3C, 0x3E, 0x60]
    static let querySet: Set<UInt8> = [0x20, 0x22, 0x23, 0x3C, 0x3E, 0x27]
    static let pathSet: Set<UInt8> = querySet.subtracting([0x27]).union([0x3F, 0x60, 0x7B, 0x7D])
    static let userinfoSet: Set<UInt8> = pathSet.union([0x2F, 0x3A, 0x3B, 0x3D, 0x40, 0x5B, 0x5C, 0x5D, 0x5E, 0x7C])

    static func encode(_ s: Substring, _ set: Set<UInt8>) -> String {
        var out = ""
        for b in s.utf8 {
            if b < 0x20 || b >= 0x7F || set.contains(b) {
                out += String(format: "%%%02X", b)
            } else {
                out.unicodeScalars.append(Unicode.Scalar(b))
            }
        }
        return out
    }

    /// `decodeURIComponent`, or nil where it would throw.
    public static func percentDecode(_ s: Substring) -> String? {
        var bytes: [UInt8] = []
        let u = Array(s.utf8)
        var i = 0
        while i < u.count {
            if u[i] == UInt8(ascii: "%") {
                guard i + 2 < u.count, let hi = hex(u[i + 1]), let lo = hex(u[i + 2]) else { return nil }
                bytes.append(hi << 4 | lo)
                i += 3
            } else {
                bytes.append(u[i])
                i += 1
            }
        }
        return String(bytes: bytes, encoding: .utf8)
    }

    private static func hex(_ b: UInt8) -> UInt8? {
        switch b {
        case 0x30...0x39: b - 0x30
        case 0x41...0x46: b - 0x37
        case 0x61...0x66: b - 0x57
        default: nil
        }
    }
}
