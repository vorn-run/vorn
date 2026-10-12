import CoreText
import SwiftUI

/// file-icons.tsx FileTypeIcon: a file's Seti glyph in its colour, chosen by name, then extension.
public struct FileTypeIcon: View {
    let name: String
    var size: CGFloat

    public init(name: String, size: CGFloat = 16) {
        self.name = name
        self.size = size
    }

    public var body: some View {
        let icon = FileTypeIcon.resolve(name)
        Text(String(UnicodeScalar(icon.ch).map(Character.init) ?? " "))
            .font(.custom(FileTypeIcon.fontName, fixedSize: size))
            .foregroundStyle(Color(hex: icon.color))
            .frame(width: size, height: size)
    }

    static let fontName: String = {
        guard let url = Bundle.module.url(forResource: "seti", withExtension: "ttf") else { return "seti" }
        CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
        let descriptors = CTFontManagerCreateFontDescriptorsFromURL(url as CFURL) as? [CTFontDescriptor]
        return descriptors?.first.flatMap { CTFontDescriptorCopyAttribute($0, kCTFontNameAttribute) as? String } ?? "seti"
    }()

    /// resolveIcon: exact name, compound extension, last two segments, extension, then dotfiles.
    public static func resolve(_ name: String) -> (ch: UInt32, color: UInt32) {
        let fallback = FileIconTables.defs["_default"]!
        func def(_ key: String?) -> (ch: UInt32, color: UInt32)? {
            key.map { FileIconTables.defs[$0] ?? fallback }
        }
        let lower = name.lowercased()
        if let d = def(FileIconTables.names[lower]) { return d }
        if let dot = lower.firstIndex(of: ".") {
            let afterFirst = String(lower[lower.index(after: dot)...])
            if let d = def(FileIconTables.ext[afterFirst]) { return d }
            if let last = lower.lastIndex(of: "."), last != dot,
               let prev = lower[..<last].lastIndex(of: ".") {
                if let d = def(FileIconTables.ext[String(lower[lower.index(after: prev)...])]) { return d }
            }
        }
        if lower.contains("."), let ext = lower.split(separator: ".", omittingEmptySubsequences: false).last.map(String.init) {
            if let d = def(FileIconTables.ext[ext]) { return d }
            if let d = def(FileIconTables.langExt[ext]) { return d }
        }
        if lower.hasPrefix(".") { return FileIconTables.defs["_config"] ?? fallback }
        return fallback
    }
}
