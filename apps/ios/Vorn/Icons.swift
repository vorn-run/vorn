import SwiftUI
import UIKit

/// An icon node drawn as the desktop draws it: vornui's own glyphs from the asset catalog build.sh makes, an agent's or extension's SVG through the core's renderer.
struct VornIcon: View {
    let name: String
    var tone: String?
    var svg: String?
    /// The colour an untoned glyph takes; nil keeps the surrounding one.
    var ink: Color?
    @ScaledMetric(relativeTo: .body) private var size: CGFloat = 18
    @Environment(\.displayScale) private var scale

    init(_ node: Node, ink: Color? = nil) {
        name = node.value ?? ""
        tone = node.style
        svg = node.label
        self.ink = ink
    }

    init(name: String, ink: Color? = nil) {
        self.name = name
        self.ink = ink
    }

    var body: some View {
        Group {
            if let svg, !svg.isEmpty, let img = Self.raster(svg, px: Int(size * scale)) {
                Image(uiImage: img).resizable()
            } else if let asset = Self.asset(name) {
                tinted(Image(asset).renderingMode(.template).resizable())
            } else {
                Color.clear
            }
        }
        .frame(width: size, height: size)
        .overlay(alignment: .topTrailing) {
            if tone == "lit" {
                Circle().fill(Palette.ink.opacity(0.7)).frame(width: 6, height: 6).offset(x: 2, y: -2)
            }
        }
        .accessibilityHidden(true)
    }

    @ViewBuilder private func tinted(_ image: some View) -> some View {
        switch tone {
        case "good", "warn", "busy", "bad", "muted": image.foregroundStyle(Palette.tone(tone))
        case "lit": image.foregroundStyle(Palette.ink)
        default:
            if let ink { image.foregroundStyle(ink) } else { image }
        }
    }

    /// The catalog's name for an icon, if the app ships it.
    nonisolated static func asset(_ name: String) -> String? {
        !name.isEmpty && UIImage(named: name) != nil ? name : nil
    }

    private static let cache = NSCache<NSString, UIImage>()

    /// `svg` drawn by the core's renderer, `px` pixels square.
    static func raster(_ svg: String, px: Int) -> UIImage? {
        let key = "\(px):\(svg)" as NSString
        if let hit = cache.object(forKey: key) { return hit }
        guard px > 0, let p = vp_svg_rgba(svg, UInt32(px)) else { return nil }
        defer { vp_free_rgba(p, UInt32(px)) }
        let data = Data(bytes: p, count: px * px * 4) as CFData
        guard let provider = CGDataProvider(data: data),
              let cg = CGImage(width: px, height: px, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: px * 4,
                               space: CGColorSpaceCreateDeviceRGB(),
                               bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue),
                               provider: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent)
        else { return nil }
        let img = UIImage(cgImage: cg, scale: UITraitCollection.current.displayScale, orientation: .up)
        cache.setObject(img, forKey: key)
        return img
    }
}

extension Label where Title == Text, Icon == Image {
    /// A menu entry with one of vornui's glyphs; menus take only images.
    init(_ title: String, vorn name: String) {
        self.init { Text(title) } icon: { Image(VornIcon.asset(name) ?? "circle").renderingMode(.template) }
    }
}
