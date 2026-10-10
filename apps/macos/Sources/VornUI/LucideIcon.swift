import SwiftUI

/// One Lucide icon as its SVG elements in a 24×24 box.
public struct LucideGlyph: Sendable {
    public enum Element: Sendable {
        case path(String)
        case circle(cx: CGFloat, cy: CGFloat, r: CGFloat)
        case ellipse(cx: CGFloat, cy: CGFloat, rx: CGFloat, ry: CGFloat)
        case rect(x: CGFloat, y: CGFloat, width: CGFloat, height: CGFloat, rx: CGFloat)
        case line(x1: CGFloat, y1: CGFloat, x2: CGFloat, y2: CGFloat)
        case poly([(CGFloat, CGFloat)], closed: Bool)
    }

    public let elements: [Element]
    let path: Path

    public init(_ elements: [Element]) {
        self.elements = elements
        var p = Path()
        for e in elements {
            switch e {
            case .path(let d):
                p.addPath(SVGPath.parse(d))
            case .circle(let cx, let cy, let r):
                p.addEllipse(in: CGRect(x: cx - r, y: cy - r, width: 2 * r, height: 2 * r))
            case .ellipse(let cx, let cy, let rx, let ry):
                p.addEllipse(in: CGRect(x: cx - rx, y: cy - ry, width: 2 * rx, height: 2 * ry))
            case .rect(let x, let y, let w, let h, let rx):
                p.addRoundedRect(in: CGRect(x: x, y: y, width: w, height: h), cornerSize: CGSize(width: rx, height: rx))
            case .line(let x1, let y1, let x2, let y2):
                p.move(to: CGPoint(x: x1, y: y1))
                p.addLine(to: CGPoint(x: x2, y: y2))
            case .poly(let points, let closed):
                guard let first = points.first else { continue }
                p.move(to: CGPoint(x: first.0, y: first.1))
                for pt in points.dropFirst() { p.addLine(to: CGPoint(x: pt.0, y: pt.1)) }
                if closed { p.closeSubpath() }
            }
        }
        path = p
    }
}

/// A Lucide glyph drawn the way lucide-react draws it: stroked, round caps and
/// joins, the stroke scaled with the icon.
public struct LucideIcon: View {
    let glyph: LucideGlyph
    let size: CGFloat
    let strokeWidth: CGFloat

    public init(_ glyph: LucideGlyph, size: CGFloat = 24, strokeWidth: CGFloat = 2) {
        self.glyph = glyph
        self.size = size
        self.strokeWidth = strokeWidth
    }

    public var body: some View {
        let scale = size / 24
        glyph.path
            .applying(CGAffineTransform(scaleX: scale, y: scale))
            .stroke(style: StrokeStyle(lineWidth: strokeWidth * scale, lineCap: .round, lineJoin: .round))
            .frame(width: size, height: size)
    }
}
