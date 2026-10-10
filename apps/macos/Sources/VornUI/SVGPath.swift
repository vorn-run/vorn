import SwiftUI

/// Draws SVG path data (`d`), so today's inline brand marks render natively.
public struct SVGPath: Shape {
    public let data: String
    public let viewBox: CGFloat

    public init(_ data: String, viewBox: CGFloat = 24) {
        self.data = data
        self.viewBox = viewBox
    }

    public func path(in rect: CGRect) -> Path {
        let scale = min(rect.width, rect.height) / viewBox
        return SVGPathParser.parse(data)
            .applying(CGAffineTransform(scaleX: scale, y: scale))
            .offsetBy(dx: rect.minX, dy: rect.minY)
    }
}

/// A parser for SVG path data: every command, absolute and relative, with arcs
/// turned into cubic curves.
public enum SVGPathParser {
    public static func parse(_ d: String) -> Path {
        var scanner = Tokens(Array(d.utf8))
        var path = Path()
        var cur = CGPoint.zero, start = CGPoint.zero
        var lastControl: CGPoint?
        var lastQuad: CGPoint?
        var cmd: UInt8 = 0

        while let next = scanner.command(current: cmd) {
            cmd = next
            let rel = cmd >= 97
            let base = rel ? cur : .zero
            func pt(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: base.x + x, y: base.y + y) }
            var control: CGPoint?
            var quad: CGPoint?
            switch cmd | 0x20 {
            case UInt8(ascii: "m"):
                guard let x = scanner.number(), let y = scanner.number() else { return path }
                cur = pt(x, y)
                start = cur
                path.move(to: cur)
                // Further pairs after a moveto are linetos.
                cmd = rel ? UInt8(ascii: "l") : UInt8(ascii: "L")
            case UInt8(ascii: "l"):
                guard let x = scanner.number(), let y = scanner.number() else { return path }
                cur = pt(x, y)
                path.addLine(to: cur)
            case UInt8(ascii: "h"):
                guard let x = scanner.number() else { return path }
                cur = CGPoint(x: rel ? cur.x + x : x, y: cur.y)
                path.addLine(to: cur)
            case UInt8(ascii: "v"):
                guard let y = scanner.number() else { return path }
                cur = CGPoint(x: cur.x, y: rel ? cur.y + y : y)
                path.addLine(to: cur)
            case UInt8(ascii: "c"):
                guard let x1 = scanner.number(), let y1 = scanner.number(),
                      let x2 = scanner.number(), let y2 = scanner.number(),
                      let x = scanner.number(), let y = scanner.number() else { return path }
                let c2 = pt(x2, y2)
                let end = pt(x, y)
                path.addCurve(to: end, control1: pt(x1, y1), control2: c2)
                control = c2
                cur = end
            case UInt8(ascii: "s"):
                guard let x2 = scanner.number(), let y2 = scanner.number(),
                      let x = scanner.number(), let y = scanner.number() else { return path }
                let c1 = lastControl.map { CGPoint(x: 2 * cur.x - $0.x, y: 2 * cur.y - $0.y) } ?? cur
                let c2 = pt(x2, y2)
                let end = pt(x, y)
                path.addCurve(to: end, control1: c1, control2: c2)
                control = c2
                cur = end
            case UInt8(ascii: "q"):
                guard let x1 = scanner.number(), let y1 = scanner.number(),
                      let x = scanner.number(), let y = scanner.number() else { return path }
                let c = pt(x1, y1)
                let end = pt(x, y)
                path.addQuadCurve(to: end, control: c)
                quad = c
                cur = end
            case UInt8(ascii: "t"):
                guard let x = scanner.number(), let y = scanner.number() else { return path }
                let c = lastQuad.map { CGPoint(x: 2 * cur.x - $0.x, y: 2 * cur.y - $0.y) } ?? cur
                let end = pt(x, y)
                path.addQuadCurve(to: end, control: c)
                quad = c
                cur = end
            case UInt8(ascii: "a"):
                guard let rx = scanner.number(), let ry = scanner.number(), let rot = scanner.number(),
                      let large = scanner.flag(), let sweep = scanner.flag(),
                      let x = scanner.number(), let y = scanner.number() else { return path }
                let end = pt(x, y)
                addArc(&path, from: cur, to: end, rx: rx, ry: ry, rotation: rot, large: large, sweep: sweep)
                cur = end
            case UInt8(ascii: "z"):
                path.closeSubpath()
                cur = start
            default:
                return path
            }
            lastControl = control
            lastQuad = quad
        }
        return path
    }

    /// Endpoint arc parameterisation to centre form (SVG 1.1 F.6.5), drawn as cubics.
    static func addArc(_ path: inout Path, from p0: CGPoint, to p1: CGPoint, rx: CGFloat, ry: CGFloat,
                       rotation: CGFloat, large: Bool, sweep: Bool) {
        var rx = abs(rx), ry = abs(ry)
        if rx == 0 || ry == 0 || p0 == p1 {
            path.addLine(to: p1)
            return
        }
        let phi = rotation * .pi / 180
        let cosP = cos(phi), sinP = sin(phi)
        let dx = (p0.x - p1.x) / 2, dy = (p0.y - p1.y) / 2
        let x1 = cosP * dx + sinP * dy
        let y1 = -sinP * dx + cosP * dy
        let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry)
        if lambda > 1 {
            rx *= sqrt(lambda)
            ry *= sqrt(lambda)
        }
        let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1
        let den = rx * rx * y1 * y1 + ry * ry * x1 * x1
        var coef = sqrt(max(0, num / den))
        if large == sweep { coef = -coef }
        let cx1 = coef * rx * y1 / ry
        let cy1 = -coef * ry * x1 / rx
        let cx = cosP * cx1 - sinP * cy1 + (p0.x + p1.x) / 2
        let cy = sinP * cx1 + cosP * cy1 + (p0.y + p1.y) / 2
        func angle(_ ux: CGFloat, _ uy: CGFloat, _ vx: CGFloat, _ vy: CGFloat) -> CGFloat {
            let a = atan2(ux * vy - uy * vx, ux * vx + uy * vy)
            return a
        }
        let theta1 = angle(1, 0, (x1 - cx1) / rx, (y1 - cy1) / ry)
        var delta = angle((x1 - cx1) / rx, (y1 - cy1) / ry, (-x1 - cx1) / rx, (-y1 - cy1) / ry)
        if !sweep && delta > 0 { delta -= 2 * .pi }
        if sweep && delta < 0 { delta += 2 * .pi }
        let segments = max(1, Int(ceil(abs(delta) / (.pi / 2))))
        let step = delta / CGFloat(segments)
        let k = 4 / 3 * tan(step / 4)
        func point(_ t: CGFloat) -> CGPoint {
            CGPoint(x: cx + rx * cos(t) * cosP - ry * sin(t) * sinP,
                    y: cy + rx * cos(t) * sinP + ry * sin(t) * cosP)
        }
        func derivative(_ t: CGFloat) -> CGPoint {
            CGPoint(x: -rx * sin(t) * cosP - ry * cos(t) * sinP,
                    y: -rx * sin(t) * sinP + ry * cos(t) * cosP)
        }
        var t = theta1
        for i in 0..<segments {
            let t2 = t + step
            let a = point(t), b = i == segments - 1 ? p1 : point(t2)
            let da = derivative(t), db = derivative(t2)
            path.addCurve(to: b,
                          control1: CGPoint(x: a.x + k * da.x, y: a.y + k * da.y),
                          control2: CGPoint(x: b.x - k * db.x, y: b.y - k * db.y))
            t = t2
        }
    }

    struct Tokens {
        let s: [UInt8]
        var i = 0

        init(_ s: [UInt8]) { self.s = s }

        mutating func skip() {
            while i < s.count, s[i] == 32 || s[i] == 44 || s[i] == 9 || s[i] == 10 || s[i] == 13 { i += 1 }
        }

        /// The next command letter, or `current` repeated if a number follows.
        mutating func command(current: UInt8) -> UInt8? {
            skip()
            guard i < s.count else { return nil }
            let c = s[i]
            if (c >= 65 && c <= 90) || (c >= 97 && c <= 122) {
                i += 1
                return c
            }
            return current == 0 ? nil : current
        }

        mutating func flag() -> Bool? {
            skip()
            guard i < s.count, s[i] == 48 || s[i] == 49 else { return nil }
            defer { i += 1 }
            return s[i] == 49
        }

        mutating func number() -> CGFloat? {
            skip()
            let begin = i
            if i < s.count, s[i] == 45 || s[i] == 43 { i += 1 }
            var dot = false
            while i < s.count {
                let c = s[i]
                if c >= 48 && c <= 57 {
                    i += 1
                } else if c == 46 && !dot {
                    dot = true
                    i += 1
                } else if (c == 101 || c == 69) && i > begin {
                    i += 1
                    if i < s.count, s[i] == 45 || s[i] == 43 { i += 1 }
                } else {
                    break
                }
            }
            guard i > begin, let v = Double(String(decoding: s[begin..<i], as: UTF8.self)) else { return nil }
            return CGFloat(v)
        }
    }
}
