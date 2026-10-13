import Foundation
import JavaScriptCore
import SwiftUI

/// Syntax colour from the same Shiki theme and grammars today's code views use, run in JavaScriptCore off the main thread.
public actor SyntaxHighlighter {
    public struct Token: Hashable, Sendable {
        public let content: String
        public let color: UInt32?
        public let alpha: Double
    }

    public typealias Line = [Token]

    public static let shared = SyntaxHighlighter()

    private var context: JSContext?
    private var highlight: JSValue?
    private var failed = false

    /// shiki.ts getLang: by file name first, then extension.
    public static func language(for name: String) -> String? {
        let lower = name.lowercased()
        let names = ["dockerfile": "dockerfile", "makefile": "makefile", ".gitignore": "gitignore", ".env": "dotenv"]
        if let l = names[lower] { return l }
        guard lower.contains("."), let ext = lower.split(separator: ".").last.map(String.init) else { return nil }
        return extensions[ext]
    }

    static let extensions: [String: String] = [
        "ts": "typescript", "tsx": "tsx", "mts": "typescript", "cts": "typescript", "js": "javascript", "jsx": "jsx",
        "mjs": "javascript", "cjs": "javascript", "json": "json", "jsonc": "jsonc", "json5": "json5", "html": "html",
        "htm": "html", "vue": "vue", "svelte": "svelte", "css": "css", "scss": "scss", "sass": "sass", "less": "less",
        "md": "markdown", "mdx": "mdx", "py": "python", "pyi": "python", "rs": "rust", "go": "go", "java": "java",
        "kt": "kotlin", "swift": "swift", "rb": "ruby", "php": "php", "lua": "lua", "zig": "zig", "c": "c", "h": "c",
        "cpp": "cpp", "cc": "cpp", "hpp": "cpp", "cxx": "cpp", "cs": "csharp", "sh": "bash", "bash": "bash",
        "zsh": "bash", "fish": "fish", "sql": "sql", "graphql": "graphql", "gql": "graphql", "yml": "yaml",
        "yaml": "yaml", "toml": "toml", "ini": "ini", "xml": "xml", "svg": "xml", "dockerfile": "dockerfile",
        "makefile": "makefile", "r": "r", "dart": "dart", "ex": "elixir", "exs": "elixir", "prisma": "prisma",
        "tf": "hcl", "ps1": "powershell", "bat": "batch",
    ]

    /// Tokens per line, or nil when the language has no grammar or the highlighter could not load.
    public func tokens(_ code: String, language: String) -> [Line]? {
        guard let fn = load() else { return nil }
        guard let json = fn.call(withArguments: [code, language])?.toString(),
              let data = json.data(using: .utf8),
              let raw = try? JSONSerialization.jsonObject(with: data) as? [[[Any]]] else { return nil }
        let lines = raw.map { line in
            line.compactMap { pair -> Token? in
                guard let content = pair.first as? String else { return nil }
                let (rgb, a) = Self.parse(pair.count > 1 ? pair[1] as? String : nil)
                return Token(content: content, color: rgb, alpha: a)
            }
        }
        return lines.allSatisfy(\.isEmpty) && !code.isEmpty ? nil : lines
    }

    private func load() -> JSValue? {
        if let highlight { return highlight }
        guard !failed, let url = Bundle.module.url(forResource: "shiki", withExtension: "js"),
              let source = try? String(contentsOf: url, encoding: .utf8), let ctx = JSContext() else {
            failed = true
            return nil
        }
        ctx.evaluateScript(source)
        let fn = ctx.objectForKeyedSubscript("vornHighlight")
        guard let fn, !fn.isUndefined else {
            failed = true
            return nil
        }
        context = ctx
        highlight = fn
        return fn
    }

    /// `#RRGGBB` or `#RRGGBBAA`.
    static func parse(_ hex: String?) -> (UInt32?, Double) {
        guard var s = hex, s.hasPrefix("#") else { return (nil, 1) }
        s.removeFirst()
        guard let v = UInt32(s, radix: 16) else { return (nil, 1) }
        if s.count == 8 { return (v >> 8, Double(v & 0xFF) / 255) }
        return (v, 1)
    }
}

public extension SyntaxHighlighter.Token {
    var swiftColor: Color? { color.map { Color(hex: $0, opacity: alpha) } }
}
