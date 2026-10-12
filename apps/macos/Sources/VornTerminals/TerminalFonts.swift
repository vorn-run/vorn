import CVornGrid
import CoreText
import Foundation
import VornUI

/// The terminal font in its four variants, with the cell metrics they share,
/// measured the way today's terminal measures them so the grid matches.
final class TerminalFonts {
    let family: String
    let regular: CTFont, bold: CTFont, italic: CTFont, boldItalic: CTFont
    /// One cell in points, snapped to whole device pixels at `scale`.
    let cell: CGSize
    /// The baseline's distance from the top of a row.
    let ascent: CGFloat
    let scale: CGFloat
    /// Glyphs of printable ASCII per variant: plain runs skip typesetting.
    let ascii: [[CGGlyph]]
    private var clusters: [ClusterKey: CTLine] = [:]

    private struct Key: Hashable { let size: CGFloat, scale: CGFloat }
    private struct ClusterKey: Hashable { let text: String, variant: Int }

    nonisolated(unsafe) private static var cache: [Key: TerminalFonts] = [:]

    static func of(size: CGFloat, scale: CGFloat = 2) -> TerminalFonts {
        let key = Key(size: size, scale: max(1, scale))
        if let f = cache[key] { return f }
        let f = TerminalFonts(size: size, scale: key.scale)
        cache[key] = f
        return f
    }

    /// The first family of today's stack that is installed; CoreText substitutes
    /// another font for a missing name, so the family is checked.
    static let resolvedFamily: String = {
        for name in Theme.Terminal.fontFamilies {
            let f = CTFontCreateWithName(name as CFString, 12, nil)
            if (CTFontCopyFamilyName(f) as String) == name { return name }
        }
        return "Menlo"
    }()

    private init(size: CGFloat, scale: CGFloat) {
        self.scale = scale
        family = Self.resolvedFamily
        let base = CTFontCreateWithName(family as CFString, size, nil)
        func variant(_ t: CTFontSymbolicTraits) -> CTFont {
            CTFontCreateCopyWithSymbolicTraits(base, size, nil, t, t) ?? base
        }
        regular = base
        bold = variant(.traitBold)
        italic = variant(.traitItalic)
        boldItalic = variant([.traitBold, .traitItalic])
        var glyph = CGGlyph(0)
        var ch = UniChar(87) // "W", as the renderer measures it
        CTFontGetGlyphsForCharacters(base, &ch, &glyph, 1)
        var adv = CGSize.zero
        CTFontGetAdvancesForGlyphs(base, .horizontal, &glyph, &adv, 1)
        // The browser rounds ascent and descent; the GPU renderer floors the width
        // and ceils the height to device pixels (Menlo 13 at 2x: 7.5 x 15).
        let up = round(CTFontGetAscent(base)), down = round(CTFontGetDescent(base))
        let height = ceil((up + down) * scale) / scale
        cell = CGSize(width: max(1, floor(adv.width * scale)) / scale, height: height)
        ascent = height - down
        ascii = [regular, bold, italic, boldItalic].map { f in
            var chars = (0..<128).map { UniChar($0) }
            var glyphs = [CGGlyph](repeating: 0, count: 128)
            CTFontGetGlyphsForCharacters(f, &chars, &glyphs, 128)
            return glyphs
        }
    }

    func variant(_ flags: UInt16) -> Int {
        (flags & UInt16(VG_BOLD) != 0 ? 1 : 0) + (flags & UInt16(VG_ITALIC) != 0 ? 2 : 0)
    }

    func font(_ flags: UInt16) -> CTFont {
        [regular, bold, italic, boldItalic][variant(flags)]
    }

    /// The grid that fits `size` points inside the padding.
    func grid(for size: CGSize) -> (cols: UInt16, rows: UInt16) {
        (UInt16(clamping: max(2, Int((size.width - 2 * terminalPadX) / cell.width))),
         UInt16(clamping: max(1, Int((size.height - 2 * terminalPadY) / cell.height))))
    }

    /// One grapheme typeset in the font that should draw it, coloured by the context's fill.
    func line(for text: String, flags: UInt16) -> CTLine {
        let key = ClusterKey(text: text, variant: variant(flags))
        if let l = clusters[key] { return l }
        let attrs: [CFString: Any] = [
            kCTFontAttributeName: fallback(for: text, in: font(flags)),
            kCTForegroundColorFromContextAttributeName: true,
        ]
        let l = CTLineCreateWithAttributedString(CFAttributedStringCreate(nil, text as CFString, attrs as CFDictionary))
        if clusters.count > 4096 { clusters.removeAll() }
        clusters[key] = l
        return l
    }

    // MARK: Fallback

    /// The font for a grapheme: the terminal font when it has the glyphs, else a
    /// system font whose colour matches the grapheme's presentation, so symbols
    /// such as U+23FA stay monochrome text and only emoji draw in colour.
    func fallback(for text: String, in primary: CTFont) -> CTFont {
        let emoji = Self.prefersEmoji(text)
        let units = Array(text.utf16.filter { $0 != 0xFE0F && $0 != 0xFE0E })
        if !emoji, Self.covers(primary, units) { return primary }
        if !emoji, primary !== regular, Self.covers(regular, units) { return regular }
        let size = CTFontGetSize(primary)
        let found = CTFontCreateForString(primary, text as CFString, CFRange(location: 0, length: (text as NSString).length))
        let name = CTFontCopyPostScriptName(found) as String
        if name != "LastResort", Self.isColor(found) == emoji { return found }
        if emoji {
            let e = CTFontCreateWithName("AppleColorEmoji" as CFString, size, nil)
            return Self.covers(e, units) ? e : found
        }
        return Self.textFace(for: units, size: size, traits: CTFontGetSymbolicTraits(primary)) ?? found
    }

    /// A monochrome installed face covering `units`, monospaced first.
    private static func textFace(for units: [UniChar], size: CGFloat, traits: CTFontSymbolicTraits) -> CTFont? {
        let s = String(decoding: units, as: UTF16.self)
        guard let cs = CFCharacterSetCreateWithCharactersInString(nil, s as CFString) as CFCharacterSet? else { return nil }
        let desc = CTFontDescriptorCreateWithAttributes([kCTFontCharacterSetAttribute: cs] as CFDictionary)
        let matches = (CTFontDescriptorCreateMatchingFontDescriptors(desc, nil) as? [CTFontDescriptor]) ?? []
        let fonts = matches.lazy
            .map { CTFontCreateWithFontDescriptor($0, size, nil) }
            .filter { !isColor($0) && (CTFontCopyPostScriptName($0) as String) != "LastResort" && covers($0, units) }
        let mono = fonts.first { CTFontGetSymbolicTraits($0).contains(.traitMonoSpace) }
        guard let face = mono ?? fonts.first else { return nil }
        let style = traits.intersection([.traitBold, .traitItalic])
        if style.isEmpty { return face }
        return CTFontCreateCopyWithSymbolicTraits(face, size, nil, style, style) ?? face
    }

    static func covers(_ font: CTFont, _ units: [UniChar]) -> Bool {
        guard !units.isEmpty else { return true }
        var glyphs = [CGGlyph](repeating: 0, count: units.count)
        return CTFontGetGlyphsForCharacters(font, units, &glyphs, units.count)
    }

    static func isColor(_ font: CTFont) -> Bool {
        CTFontGetSymbolicTraits(font).contains(.traitColorGlyphs)
    }

    /// Whether a grapheme draws as emoji: an explicit VS16, or a default emoji
    /// presentation without VS15 (Unicode's Emoji_Presentation).
    static func prefersEmoji(_ text: String) -> Bool {
        let scalars = text.unicodeScalars
        if scalars.contains(where: { $0.value == 0xFE0F }) { return true }
        if scalars.contains(where: { $0.value == 0xFE0E }) { return false }
        return scalars.first?.properties.isEmojiPresentation ?? false
    }
}
