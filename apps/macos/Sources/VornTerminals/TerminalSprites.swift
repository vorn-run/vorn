import CoreGraphics

/// Box drawing, block elements and powerline separators drawn as shapes that
/// fill their cell exactly, as today's terminal does, instead of font glyphs
/// that leave gaps between rows.
enum TerminalSprites {
    /// Arm weights per box-drawing character U+2500...U+257F, as up, right,
    /// down, left digits: 0 none, 1 light, 2 heavy, 3 double. "x" marks the
    /// dashed, arc and diagonal characters drawn separately.
    private static let arms: [[UInt8]] = """
    0101 0202 1010 2020 x x x x x x x x 0110 0210 0120 0220 \
    0011 0012 0021 0022 1100 1200 2100 2200 1001 1002 2001 2002 1110 1210 2110 1120 \
    2120 2210 1220 2220 1011 1012 2011 1021 2021 2012 1022 2022 0111 0112 0211 0212 \
    0121 0122 0221 0222 1101 1102 1201 1202 2101 2102 2201 2202 1111 1112 1211 1212 \
    2111 1121 2121 2112 2211 1122 1221 2212 1222 2122 2221 2222 x x x x \
    0303 3030 0310 0130 0330 0013 0031 0033 1300 3100 3300 1003 3001 3003 1310 3130 \
    3330 1013 3031 3033 0313 0131 0333 1303 3101 3303 1313 3131 3333 x x x \
    x x x x 0001 1000 0100 0010 0002 2000 0200 0020 0201 1020 0102 2010
    """.split(separator: " ").map { $0 == "x" ? [] : $0.map { UInt8(String($0))! } }

    static func covers(_ v: UInt32) -> Bool {
        (0x2500...0x259F).contains(v) || (0xE0B0...0xE0B3).contains(v)
    }

    /// Draws `v` into the cell at `r` (top-down) in `color`.
    static func draw(_ v: UInt32, in r: CGRect, color: CGColor, scale: CGFloat, ctx: CGContext) {
        let s = Snap(scale: scale)
        ctx.setFillColor(color)
        ctx.setStrokeColor(color)
        switch v {
        case 0x2500...0x257F:
            let a = arms[Int(v - 0x2500)]
            if a.count == 4 { lines(a, r, s, ctx) } else { special(v, r, s, ctx) }
        case 0x2580...0x259F: block(v, r, s, ctx)
        default: powerline(v, r, ctx)
        }
    }

    private struct Snap {
        let scale: CGFloat
        func callAsFunction(_ x: CGFloat) -> CGFloat { (x * scale).rounded() / scale }
        var light: CGFloat { max(1, scale.rounded()) / scale }
        var heavy: CGFloat { 2 * light }
        /// Distance of each line of a double stroke from the centre.
        var gap: CGFloat { 1.5 * light }
        func thickness(_ w: UInt8) -> CGFloat { w == 2 ? heavy : light }
        /// How far a perpendicular arm reaches from the centre line.
        func half(_ w: UInt8) -> CGFloat {
            switch w {
            case 0: 0
            case 3: gap + light / 2
            default: thickness(w) / 2
            }
        }
    }

    // MARK: Lines

    /// Lines from the cell's edges to its centre, joined like the font's box drawing.
    private static func lines(_ a: [UInt8], _ r: CGRect, _ s: Snap, _ ctx: CGContext) {
        let cx = s(r.midX), cy = s(r.midY)
        // Directions: 0 up, 1 right, 2 down, 3 left. Sides of a vertical arm are left/right.
        for d in 0..<4 where a[d] != 0 {
            let horizontal = d % 2 == 1
            let sign: CGFloat = d == 1 || d == 2 ? 1 : -1
            let edge = horizontal ? (sign > 0 ? r.maxX : r.minX) : (sign > 0 ? r.maxY : r.minY)
            let centre = horizontal ? cx : cy
            let sideArms = horizontal ? (a[0], a[2]) : (a[3], a[1])
            let through = a[(d + 2) % 4]
            func segment(offset: CGFloat, thickness t: CGFloat, end: CGFloat) {
                let lo = min(edge, end), hi = max(edge, end)
                let c = (horizontal ? cy : cx) + offset
                let rect = horizontal
                    ? CGRect(x: lo, y: s(c - t / 2), width: hi - lo, height: t)
                    : CGRect(x: s(c - t / 2), y: lo, width: t, height: hi - lo)
                ctx.fill(rect)
            }
            if a[d] == 3 {
                for (side, near, far) in [(-1.0, sideArms.0, sideArms.1), (1.0, sideArms.1, sideArms.0)] {
                    let end: CGFloat
                    if near != 0 {
                        end = centre + sign * (near == 3 ? s.gap - s.light / 2 : s.thickness(near) / 2)
                    } else if through != 0 {
                        end = centre - sign * (through == 3 ? 0 : s.light / 2)
                    } else {
                        end = centre - sign * (far == 3 ? s.gap + s.light / 2 : s.half(far))
                    }
                    segment(offset: CGFloat(side) * s.gap, thickness: s.light, end: end)
                }
            } else {
                let reach = max(s.half(sideArms.0), s.half(sideArms.1), s.thickness(a[d]) / 2)
                segment(offset: 0, thickness: s.thickness(a[d]), end: centre - sign * reach)
            }
        }
    }

    private static func special(_ v: UInt32, _ r: CGRect, _ s: Snap, _ ctx: CGContext) {
        switch v {
        case 0x2504...0x250B, 0x254C...0x254F:
            let dashes: Int, heavy: Bool, horizontal: Bool
            if v >= 0x254C {
                dashes = 2; heavy = v % 2 == 1; horizontal = v < 0x254E
            } else {
                let i = Int(v - 0x2504)
                dashes = i < 4 ? 3 : 4; heavy = i % 2 == 1; horizontal = i % 4 < 2
            }
            let t = heavy ? s.heavy : s.light
            let length = horizontal ? r.width : r.height
            let step = length / CGFloat(dashes), gap = max(s.light, step / 3)
            for i in 0..<dashes {
                let a = s(CGFloat(i) * step + gap / 2), b = s(CGFloat(i + 1) * step - gap / 2)
                ctx.fill(horizontal
                    ? CGRect(x: r.minX + a, y: s(r.midY - t / 2), width: b - a, height: t)
                    : CGRect(x: s(r.midX - t / 2), y: r.minY + a, width: t, height: b - a))
            }
        case 0x256D...0x2570:
            let cx = s(r.midX), cy = s(r.midY)
            let vertical = v == 0x256D || v == 0x256E ? r.maxY : r.minY
            let horizontal = v == 0x256D || v == 0x2570 ? r.maxX : r.minX
            let p = CGMutablePath()
            p.move(to: CGPoint(x: cx, y: vertical))
            p.addArc(tangent1End: CGPoint(x: cx, y: cy), tangent2End: CGPoint(x: horizontal, y: cy),
                     radius: min(r.width, r.height) / 2)
            p.addLine(to: CGPoint(x: horizontal, y: cy))
            stroke(p, s.light, ctx)
        default:
            let p = CGMutablePath()
            if v != 0x2572 { p.move(to: CGPoint(x: r.minX, y: r.maxY)); p.addLine(to: CGPoint(x: r.maxX, y: r.minY)) }
            if v != 0x2571 { p.move(to: CGPoint(x: r.minX, y: r.minY)); p.addLine(to: CGPoint(x: r.maxX, y: r.maxY)) }
            stroke(p, s.light, ctx)
        }
    }

    private static func stroke(_ p: CGPath, _ width: CGFloat, _ ctx: CGContext) {
        ctx.saveGState()
        ctx.setLineWidth(width)
        ctx.addPath(p)
        ctx.strokePath()
        ctx.restoreGState()
    }

    // MARK: Blocks

    private static func block(_ v: UInt32, _ r: CGRect, _ s: Snap, _ ctx: CGContext) {
        let w = r.width, h = r.height
        func fill(_ x: CGFloat, _ y: CGFloat, _ fw: CGFloat, _ fh: CGFloat) {
            let x0 = s(r.minX + x * w), y0 = s(r.minY + y * h)
            ctx.fill(CGRect(x: x0, y: y0, width: s(r.minX + (x + fw) * w) - x0, height: s(r.minY + (y + fh) * h) - y0))
        }
        switch v {
        case 0x2580: fill(0, 0, 1, 0.5)
        case 0x2581...0x2588:
            let f = CGFloat(v - 0x2580) / 8
            fill(0, 1 - f, 1, f)
        case 0x2589...0x258F: fill(0, 0, CGFloat(0x2590 - v) / 8, 1)
        case 0x2590: fill(0.5, 0, 0.5, 1)
        case 0x2591...0x2593:
            ctx.saveGState()
            ctx.setAlpha(CGFloat(v - 0x2590) / 4)
            fill(0, 0, 1, 1)
            ctx.restoreGState()
        case 0x2594: fill(0, 0, 1, 1.0 / 8)
        case 0x2595: fill(7.0 / 8, 0, 1.0 / 8, 1)
        default:
            // Quadrants as bits: upper left 1, upper right 2, lower left 4, lower right 8.
            let bits: [UInt8] = [4, 8, 1, 13, 9, 7, 11, 2, 6, 14]
            let q = bits[Int(v - 0x2596)]
            if q & 1 != 0 { fill(0, 0, 0.5, 0.5) }
            if q & 2 != 0 { fill(0.5, 0, 0.5, 0.5) }
            if q & 4 != 0 { fill(0, 0.5, 0.5, 0.5) }
            if q & 8 != 0 { fill(0.5, 0.5, 0.5, 0.5) }
        }
    }

    // MARK: Powerline

    private static func powerline(_ v: UInt32, _ r: CGRect, _ ctx: CGContext) {
        let right = v <= 0xE0B1
        let tip = CGPoint(x: right ? r.maxX : r.minX, y: r.midY)
        let base = right ? r.minX : r.maxX
        let p = CGMutablePath()
        p.move(to: CGPoint(x: base, y: r.minY))
        p.addLine(to: tip)
        p.addLine(to: CGPoint(x: base, y: r.maxY))
        if v == 0xE0B0 || v == 0xE0B2 {
            p.closeSubpath()
            ctx.addPath(p)
            ctx.fillPath()
        } else {
            stroke(p, 1, ctx)
        }
    }
}
