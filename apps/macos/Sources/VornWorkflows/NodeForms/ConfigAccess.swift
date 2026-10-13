import Foundation
import VornCore

/// Reads and writes on a step's `config` object; unknown keys survive every edit.
extension JSONValue {
    /// The string at `key`, nil when absent or not a string.
    public func str(_ key: String) -> String? { self[key]?.stringValue }

    /// The number at `key`, nil when absent or not a number.
    public func num(_ key: String) -> Double? { self[key]?.numberValue }

    /// The bool at `key`, nil when absent or not a bool.
    public func bool(_ key: String) -> Bool? { self[key]?.boolValue }

    /// Sets `key`; nil removes it, like a JS `undefined` that JSON.stringify drops.
    public mutating func set(_ key: String, _ value: JSONValue?) {
        var o = objectValue ?? [:]
        o[key] = value
        self = .object(o)
    }

    public mutating func set(_ key: String, string value: String?) { set(key, value.map(JSONValue.string)) }

    public mutating func set(_ key: String, number value: Double?) { set(key, value.map(JSONValue.number)) }

    public mutating func set(_ key: String, bool value: Bool?) { set(key, value.map(JSONValue.bool)) }

    /// Returns a copy with `updates` merged over it, like `{ ...config, ...updates }`.
    public func merged(_ updates: [String: JSONValue?]) -> JSONValue {
        var copy = self
        for (k, v) in updates { copy.set(k, v) }
        return copy
    }

    /// Compact JSON text, keys sorted, like JSON.stringify.
    public var jsonText: String {
        let enc = JSONEncoder()
        enc.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        guard let data = try? enc.encode(self), let s = String(data: data, encoding: .utf8) else { return "" }
        return s
    }

    /// Indented JSON text, like `JSON.stringify(v, null, 2)`.
    public var prettyJSONText: String {
        let enc = JSONEncoder()
        enc.outputFormatting = [.sortedKeys, .withoutEscapingSlashes, .prettyPrinted]
        guard let data = try? enc.encode(self), let s = String(data: data, encoding: .utf8) else { return "" }
        // Halve only the leading indentation and write `"key": v` like JSON.stringify, not `"key" : v`.
        return s.split(separator: "\n", omittingEmptySubsequences: false).map { line in
            let indent = line.prefix { $0 == " " }.count
            var rest = String(line.dropFirst(indent))
            if rest.hasPrefix("\"") {
                var escaped = false
                var i = rest.index(after: rest.startIndex)
                while i < rest.endIndex {
                    let ch = rest[i]
                    if escaped { escaped = false } else if ch == "\\" { escaped = true } else if ch == "\"" { break }
                    i = rest.index(after: i)
                }
                if i < rest.endIndex {
                    let after = rest.index(after: i)
                    if rest[after...].hasPrefix(" : ") {
                        rest.replaceSubrange(after..<rest.index(after, offsetBy: 3), with: ": ")
                    }
                }
            }
            return String(repeating: " ", count: indent / 2) + rest
        }.joined(separator: "\n")
    }

    /// Parses JSON text; nil when it is not valid JSON.
    public static func parse(_ text: String) -> JSONValue? {
        guard let data = text.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(JSONValue.self, from: data)
    }

    /// `{}` for a missing or non-object value.
    var objectOrEmpty: [String: JSONValue] { objectValue ?? [:] }
}

/// A whitespace-split argument list, as the Arguments fields write it.
func splitArgs(_ text: String) -> [String]? {
    let parts = text.split(whereSeparator: { $0.isWhitespace }).map(String.init)
    return parts.isEmpty ? nil : parts
}
