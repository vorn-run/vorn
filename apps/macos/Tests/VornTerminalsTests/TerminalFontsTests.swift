import CoreGraphics
import CoreText
import Testing
@testable import VornTerminals

struct TerminalFontsTests {
    @Test func cellsSnapToDevicePixelsLikeTodaysRenderer() {
        let f = TerminalFonts.of(size: 13, scale: 2)
        #expect(f.family == "Menlo" || f.family == "JetBrains Mono")
        guard f.family == "Menlo" else { return }
        #expect(f.cell == CGSize(width: 7.5, height: 15))
        #expect(TerminalFonts.of(size: 13, scale: 1).cell == CGSize(width: 7, height: 15))
        #expect(f.ascent == 12)
    }

    @Test func symbolsStayMonochromeAndOnlyEmojiDrawInColour() {
        let f = TerminalFonts.of(size: 13, scale: 2)
        for symbol in ["\u{23FA}", "\u{23BF}", "\u{2714}", "\u{276F}", "\u{25B6}"] {
            #expect(!TerminalFonts.isColor(f.fallback(for: symbol, in: f.regular)), "\(symbol)")
        }
        for emoji in ["\u{1F680}", "\u{231A}", "\u{23FA}\u{FE0F}"] {
            #expect(TerminalFonts.isColor(f.fallback(for: emoji, in: f.regular)), "\(emoji)")
        }
        #expect(f.fallback(for: "a", in: f.bold) === f.bold)
    }

    @Test func presentationFollowsVariationSelectors() {
        #expect(!TerminalFonts.prefersEmoji("\u{23FA}"))
        #expect(TerminalFonts.prefersEmoji("\u{23FA}\u{FE0F}"))
        #expect(!TerminalFonts.prefersEmoji("\u{231A}\u{FE0E}"))
        #expect(TerminalFonts.prefersEmoji("\u{1F680}"))
    }

    @Test func spritesCoverBoxBlocksAndPowerline() {
        #expect(TerminalSprites.covers(0x2500) && TerminalSprites.covers(0x259F) && TerminalSprites.covers(0xE0B0))
        #expect(!TerminalSprites.covers(0x25A0) && !TerminalSprites.covers(0x23FA))
    }

    @Test func boxDrawingFillsItsCellEdgeToEdge() throws {
        // A light horizontal line spans the whole cell, so adjacent cells join.
        let ctx = try #require(CGContext(data: nil, width: 15, height: 30, bitsPerComponent: 8, bytesPerRow: 0,
                                         space: CGColorSpaceCreateDeviceGray(),
                                         bitmapInfo: CGImageAlphaInfo.none.rawValue))
        ctx.scaleBy(x: 2, y: 2)
        TerminalSprites.draw(0x2500, in: CGRect(x: 0, y: 0, width: 7.5, height: 15),
                             color: CGColor(gray: 1, alpha: 1), scale: 2, ctx: ctx)
        let data = try #require(ctx.data).assumingMemoryBound(to: UInt8.self)
        let row = (0..<30).first { y in data[y * ctx.bytesPerRow] > 0 }
        let y = try #require(row)
        #expect((0..<15).allSatisfy { data[y * ctx.bytesPerRow + $0] == 255 })
    }
}
