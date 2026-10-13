import Foundation

/// A JSON value whose objects keep the order they were written in, for a manifest's declared tweaks.
public indirect enum OrderedJSON: Equatable, Sendable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([OrderedJSON])
    case object([(String, OrderedJSON)])

    public static func == (a: OrderedJSON, b: OrderedJSON) -> Bool { a.plain == b.plain }

    public subscript(key: String) -> OrderedJSON? {
        if case .object(let o) = self { return o.last(where: { $0.0 == key })?.1 }
        return nil
    }

    public var string: String? { if case .string(let s) = self { s } else { nil } }
    public var number: Double? { if case .number(let n) = self { n } else { nil } }

    /// The same value, unordered.
    public var plain: JSONValue {
        switch self {
        case .null: .null
        case .bool(let b): .bool(b)
        case .number(let n): .number(n)
        case .string(let s): .string(s)
        case .array(let a): .array(a.map(\.plain))
        case .object(let o): .object(Dictionary(o.map { ($0.0, $0.1.plain) }, uniquingKeysWith: { $1 }))
        }
    }

    public static func parse(_ text: String) -> OrderedJSON? {
        var p = Parser(bytes: Array(text.utf8))
        guard let v = p.value() else { return nil }
        p.skip()
        return p.i == p.bytes.count ? v : nil
    }

    private struct Parser {
        let bytes: [UInt8]
        var i = 0
        var depth = 0

        mutating func skip() {
            while i < bytes.count, [0x20, 0x09, 0x0A, 0x0D].contains(bytes[i]) { i += 1 }
        }

        mutating func literal(_ word: String) -> Bool {
            let w = Array(word.utf8)
            guard i + w.count <= bytes.count, Array(bytes[i..<i + w.count]) == w else { return false }
            i += w.count
            return true
        }

        mutating func value() -> OrderedJSON? {
            skip()
            guard i < bytes.count, depth < 128 else { return nil }
            switch bytes[i] {
            case UInt8(ascii: "{"):
                i += 1
                depth += 1
                defer { depth -= 1 }
                var out: [(String, OrderedJSON)] = []
                skip()
                if i < bytes.count, bytes[i] == UInt8(ascii: "}") { i += 1; return .object(out) }
                while true {
                    skip()
                    guard let k = string() else { return nil }
                    skip()
                    guard i < bytes.count, bytes[i] == UInt8(ascii: ":") else { return nil }
                    i += 1
                    guard let v = value() else { return nil }
                    if let at = out.firstIndex(where: { $0.0 == k }) { out[at].1 = v } else { out.append((k, v)) }
                    skip()
                    guard i < bytes.count else { return nil }
                    if bytes[i] == UInt8(ascii: ",") { i += 1; continue }
                    if bytes[i] == UInt8(ascii: "}") { i += 1; return .object(out) }
                    return nil
                }
            case UInt8(ascii: "["):
                i += 1
                depth += 1
                defer { depth -= 1 }
                var out: [OrderedJSON] = []
                skip()
                if i < bytes.count, bytes[i] == UInt8(ascii: "]") { i += 1; return .array(out) }
                while true {
                    guard let v = value() else { return nil }
                    out.append(v)
                    skip()
                    guard i < bytes.count else { return nil }
                    if bytes[i] == UInt8(ascii: ",") { i += 1; continue }
                    if bytes[i] == UInt8(ascii: "]") { i += 1; return .array(out) }
                    return nil
                }
            case UInt8(ascii: "\""):
                return string().map(OrderedJSON.string)
            case UInt8(ascii: "t"): return literal("true") ? .bool(true) : nil
            case UInt8(ascii: "f"): return literal("false") ? .bool(false) : nil
            case UInt8(ascii: "n"): return literal("null") ? .null : nil
            default: return number()
            }
        }

        mutating func number() -> OrderedJSON? {
            let start = i
            func digits() -> Int {
                let s = i
                while i < bytes.count, (0x30...0x39).contains(bytes[i]) { i += 1 }
                return i - s
            }
            if i < bytes.count, bytes[i] == UInt8(ascii: "-") { i += 1 }
            let intStart = i
            guard digits() > 0 else { return nil }
            if bytes[intStart] == 0x30 && i - intStart > 1 { return nil }
            if i < bytes.count, bytes[i] == UInt8(ascii: ".") {
                i += 1
                guard digits() > 0 else { return nil }
            }
            if i < bytes.count, bytes[i] == UInt8(ascii: "e") || bytes[i] == UInt8(ascii: "E") {
                i += 1
                if i < bytes.count, bytes[i] == UInt8(ascii: "+") || bytes[i] == UInt8(ascii: "-") { i += 1 }
                guard digits() > 0 else { return nil }
            }
            return Double(String(decoding: bytes[start..<i], as: UTF8.self)).map(OrderedJSON.number)
        }

        mutating func string() -> String? {
            guard i < bytes.count, bytes[i] == UInt8(ascii: "\"") else { return nil }
            i += 1
            var out: [UInt8] = []
            var units: [UInt16] = []
            func flushUnits() {
                if !units.isEmpty { out += Array(String(decoding: units, as: UTF16.self).utf8); units = [] }
            }
            while i < bytes.count {
                let b = bytes[i]
                if b == UInt8(ascii: "\"") {
                    i += 1
                    flushUnits()
                    return String(decoding: out, as: UTF8.self)
                }
                if b < 0x20 { return nil }
                if b == UInt8(ascii: "\\") {
                    guard i + 1 < bytes.count else { return nil }
                    let e = bytes[i + 1]
                    i += 2
                    if e == UInt8(ascii: "u") {
                        guard i + 4 <= bytes.count,
                              let n = UInt16(String(decoding: bytes[i..<i + 4], as: UTF8.self), radix: 16) else { return nil }
                        units.append(n)
                        i += 4
                        continue
                    }
                    flushUnits()
                    switch e {
                    case UInt8(ascii: "\""): out.append(0x22)
                    case UInt8(ascii: "\\"): out.append(0x5C)
                    case UInt8(ascii: "/"): out.append(0x2F)
                    case UInt8(ascii: "b"): out.append(0x08)
                    case UInt8(ascii: "f"): out.append(0x0C)
                    case UInt8(ascii: "n"): out.append(0x0A)
                    case UInt8(ascii: "r"): out.append(0x0D)
                    case UInt8(ascii: "t"): out.append(0x09)
                    default: return nil
                    }
                    continue
                }
                flushUnits()
                out.append(b)
                i += 1
            }
            return nil
        }
    }
}
