import Foundation

/// JSON that keeps object keys in the order they came, as JSON.parse and JSON.stringify do.
indirect enum OJSON: Equatable, Sendable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([OJSON])
    case object(OObject)

    var objectValue: OObject? {
        if case .object(let o) = self { return o }
        return nil
    }

    var arrayValue: [OJSON]? {
        if case .array(let a) = self { return a }
        return nil
    }
}

/// An object's members in insertion order; setting a key that exists keeps its place.
struct OObject: Equatable, Sendable {
    private(set) var keys: [String] = []
    private var values: [String: OJSON] = [:]

    init() {}

    init(_ pairs: [(String, OJSON)]) {
        for (k, v) in pairs { self[k] = v }
    }

    subscript(key: String) -> OJSON? {
        get { values[key] }
        set {
            if let newValue {
                if values[key] == nil { keys.append(key) }
                values[key] = newValue
            } else if values.removeValue(forKey: key) != nil {
                keys.removeAll { $0 == key }
            }
        }
    }

    func has(_ key: String) -> Bool { values[key] != nil }

    var entries: [(key: String, value: OJSON)] { keys.map { ($0, values[$0]!) } }
}

/// A JSON.parse failure, at a line and column counted from 1.
struct OJSONError: Error, Equatable {
    var message: String
    var line: Int
    var column: Int
}

extension OJSON {
    /// Parses strict JSON, as JSON.parse does.
    static func parse(_ text: String) throws(OJSONError) -> OJSON {
        var p = Parser(scalars: Array(text.unicodeScalars))
        p.skipSpace()
        let value = try p.value()
        p.skipSpace()
        if p.i < p.scalars.count { throw p.fail("Unexpected non-whitespace character after JSON") }
        return value
    }

    /// JSON.stringify(value), or JSON.stringify(value, null, indent) when indent > 0.
    func stringify(indent: Int = 0) -> String {
        var out = ""
        write(&out, indent: indent, depth: 0)
        return out
    }

    private func write(_ out: inout String, indent: Int, depth: Int) {
        let pad = { (d: Int) in indent > 0 ? "\n" + String(repeating: " ", count: indent * d) : "" }
        switch self {
        case .null: out += "null"
        case .bool(let b): out += b ? "true" : "false"
        case .number(let n): out += Self.format(n)
        case .string(let s): out += Self.quote(s)
        case .array(let a):
            guard !a.isEmpty else { out += "[]"; return }
            out += "["
            for (i, v) in a.enumerated() {
                if i > 0 { out += "," }
                out += pad(depth + 1)
                v.write(&out, indent: indent, depth: depth + 1)
            }
            out += pad(depth) + "]"
        case .object(let o):
            guard !o.keys.isEmpty else { out += "{}"; return }
            out += "{"
            for (i, e) in o.entries.enumerated() {
                if i > 0 { out += "," }
                out += pad(depth + 1) + Self.quote(e.key) + (indent > 0 ? ": " : ":")
                e.value.write(&out, indent: indent, depth: depth + 1)
            }
            out += pad(depth) + "}"
        }
    }

    /// A number as JavaScript prints it.
    static func format(_ n: Double) -> String {
        guard n.isFinite else { return "null" }
        if n == n.rounded(), abs(n) < 1e21 { return String(Int64(n)) }
        var s = "\(n)"
        if let e = s.firstIndex(where: { $0 == "e" || $0 == "E" }) {
            var exp = String(s[s.index(after: e)...])
            let sign = exp.hasPrefix("-") ? "-" : "+"
            exp = String(exp.drop { $0 == "-" || $0 == "+" || $0 == "0" })
            s = String(s[..<e]) + "e" + sign + (exp.isEmpty ? "0" : exp)
        }
        return s
    }

    static func quote(_ s: String) -> String {
        var out = "\""
        for u in s.unicodeScalars {
            switch u {
            case "\"": out += "\\\""
            case "\\": out += "\\\\"
            case "\n": out += "\\n"
            case "\r": out += "\\r"
            case "\t": out += "\\t"
            case "\u{08}": out += "\\b"
            case "\u{0C}": out += "\\f"
            default:
                if u.value < 0x20 {
                    out += String(format: "\\u%04x", u.value)
                } else {
                    out.unicodeScalars.append(u)
                }
            }
        }
        return out + "\""
    }

    private struct Parser {
        let scalars: [Unicode.Scalar]
        var i = 0

        func fail(_ message: String) -> OJSONError {
            var line = 1
            var column = 1
            for s in scalars[..<min(i, scalars.count)] {
                if s == "\n" {
                    line += 1
                    column = 1
                } else {
                    column += 1
                }
            }
            return OJSONError(message: message, line: line, column: column)
        }

        func unexpected() -> OJSONError {
            guard i < scalars.count else { return fail("Unexpected end of JSON input") }
            return fail("Unexpected token '\(Character(scalars[i]))'")
        }

        mutating func skipSpace() {
            while i < scalars.count, [" ", "\n", "\r", "\t"].contains(scalars[i]) { i += 1 }
        }

        mutating func value() throws(OJSONError) -> OJSON {
            guard i < scalars.count else { throw unexpected() }
            switch scalars[i] {
            case "{": return try object()
            case "[": return try array()
            case "\"": return .string(try string())
            case "t": return try literal("true", .bool(true))
            case "f": return try literal("false", .bool(false))
            case "n": return try literal("null", .null)
            case "-", "0"..."9": return try number()
            default: throw unexpected()
            }
        }

        mutating func literal(_ word: String, _ v: OJSON) throws(OJSONError) -> OJSON {
            for s in word.unicodeScalars {
                guard i < scalars.count, scalars[i] == s else { throw unexpected() }
                i += 1
            }
            return v
        }

        mutating func number() throws(OJSONError) -> OJSON {
            let start = i
            if scalars[i] == "-" { i += 1 }
            func digits(_ p: inout Parser) -> Int {
                let s = p.i
                while p.i < p.scalars.count, ("0"..."9").contains(p.scalars[p.i]) { p.i += 1 }
                return p.i - s
            }
            if i < scalars.count, scalars[i] == "0" {
                i += 1
            } else if digits(&self) == 0 {
                throw unexpected()
            }
            if i < scalars.count, scalars[i] == "." {
                i += 1
                if digits(&self) == 0 { throw unexpected() }
            }
            if i < scalars.count, scalars[i] == "e" || scalars[i] == "E" {
                i += 1
                if i < scalars.count, scalars[i] == "+" || scalars[i] == "-" { i += 1 }
                if digits(&self) == 0 { throw unexpected() }
            }
            var text = ""
            text.unicodeScalars.append(contentsOf: scalars[start..<i])
            return .number(Double(text) ?? 0)
        }

        mutating func hex4() throws(OJSONError) -> UInt32 {
            guard i + 4 <= scalars.count else { throw fail("Bad Unicode escape in JSON") }
            var text = ""
            text.unicodeScalars.append(contentsOf: scalars[i..<i + 4])
            guard let v = UInt32(text, radix: 16) else { throw fail("Bad Unicode escape in JSON") }
            i += 4
            return v
        }

        mutating func string() throws(OJSONError) -> String {
            i += 1
            var out = String.UnicodeScalarView()
            while true {
                guard i < scalars.count else { throw fail("Unterminated string in JSON") }
                let s = scalars[i]
                if s == "\"" {
                    i += 1
                    return String(out)
                }
                if s.value < 0x20 { throw fail("Bad control character in string literal in JSON") }
                if s != "\\" {
                    out.append(s)
                    i += 1
                    continue
                }
                i += 1
                guard i < scalars.count else { throw fail("Unterminated string in JSON") }
                let e = scalars[i]
                i += 1
                switch e {
                case "\"": out.append("\"")
                case "\\": out.append("\\")
                case "/": out.append("/")
                case "b": out.append("\u{08}")
                case "f": out.append("\u{0C}")
                case "n": out.append("\n")
                case "r": out.append("\r")
                case "t": out.append("\t")
                case "u":
                    var v = try hex4()
                    if (0xD800...0xDBFF).contains(v), i + 1 < scalars.count, scalars[i] == "\\", scalars[i + 1] == "u" {
                        let save = i
                        i += 2
                        let low = try hex4()
                        if (0xDC00...0xDFFF).contains(low) {
                            v = 0x10000 + ((v - 0xD800) << 10) + (low - 0xDC00)
                        } else {
                            i = save
                        }
                    }
                    out.append(Unicode.Scalar(v) ?? "\u{FFFD}")
                default:
                    i -= 1
                    throw fail("Bad escaped character in JSON")
                }
            }
        }

        mutating func array() throws(OJSONError) -> OJSON {
            i += 1
            var items: [OJSON] = []
            skipSpace()
            if i < scalars.count, scalars[i] == "]" {
                i += 1
                return .array(items)
            }
            while true {
                skipSpace()
                items.append(try value())
                skipSpace()
                guard i < scalars.count else { throw unexpected() }
                if scalars[i] == "," {
                    i += 1
                    continue
                }
                if scalars[i] == "]" {
                    i += 1
                    return .array(items)
                }
                throw fail("Expected ',' or ']' after array element in JSON")
            }
        }

        mutating func object() throws(OJSONError) -> OJSON {
            i += 1
            var o = OObject()
            skipSpace()
            if i < scalars.count, scalars[i] == "}" {
                i += 1
                return .object(o)
            }
            while true {
                skipSpace()
                guard i < scalars.count, scalars[i] == "\"" else {
                    throw i < scalars.count ? fail("Expected double-quoted property name in JSON") : unexpected()
                }
                let key = try string()
                skipSpace()
                guard i < scalars.count, scalars[i] == ":" else {
                    throw i < scalars.count ? fail("Expected ':' after property name in JSON") : unexpected()
                }
                i += 1
                skipSpace()
                o[key] = try value()
                skipSpace()
                guard i < scalars.count else { throw unexpected() }
                if scalars[i] == "," {
                    i += 1
                    continue
                }
                if scalars[i] == "}" {
                    i += 1
                    return .object(o)
                }
                throw fail("Expected ',' or '}' after property value in JSON")
            }
        }
    }
}
