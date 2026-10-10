import SwiftUI
import VornCore

/// A lucide icon stroked as today's renderer draws it: `size` points square,
/// `strokeWidth` in the icon's 24-unit grid, round caps and joins.
public struct LucideIcon: View {
    let icon: Lucide
    let size: CGFloat
    let strokeWidth: CGFloat

    public init(_ icon: Lucide, size: CGFloat = 16, strokeWidth: CGFloat = 2) {
        self.icon = icon
        self.size = size
        self.strokeWidth = strokeWidth
    }

    public var body: some View {
        LucideShape(icon: icon)
            .stroke(style: StrokeStyle(lineWidth: strokeWidth * size / 24, lineCap: .round, lineJoin: .round))
            .frame(width: size, height: size)
    }
}

struct LucideShape: Shape {
    let icon: Lucide

    func path(in rect: CGRect) -> Path {
        var p = Path()
        for d in icon.paths { p.addPath(SVGPathParser.parse(d)) }
        let s = min(rect.width, rect.height) / 24
        return p.applying(CGAffineTransform(scaleX: s, y: s)).offsetBy(dx: rect.minX, dy: rect.minY)
    }
}

/// Several SVG paths filled as one shape.
struct FilledPaths: Shape {
    let paths: [String]
    var viewBox: CGFloat = 24

    func path(in rect: CGRect) -> Path {
        var p = Path()
        for d in paths { p.addPath(SVGPathParser.parse(d)) }
        let s = min(rect.width, rect.height) / viewBox
        return p.applying(CGAffineTransform(scaleX: s, y: s)).offsetBy(dx: rect.minX, dy: rect.minY)
    }
}

/// An agent's brand mark (AgentIcon.tsx); shells and unknown agents get the terminal glyph.
public struct AgentIcon: View {
    let agent: AgentType
    let size: CGFloat

    public init(_ agent: AgentType, size: CGFloat = 16) {
        self.agent = agent
        self.size = size
    }

    public var body: some View {
        Group {
            switch agent {
            case .claude:
                SVGPath(BrandPaths.claude).fill(Color(hex: 0xD97757))
            case .copilot:
                SVGPath(BrandPaths.copilot).fill(Color.white, style: FillStyle(eoFill: true))
            case .codex:
                ZStack {
                    SVGPath(BrandPaths.codexBackground).fill(Color.white)
                    SVGPath(BrandPaths.codex).fill(gradient(
                        [(0xB1A7FF, 1, 0), (0x7A9DFF, 1, 0.5), (0x3941FF, 1, 1)],
                        from: (12, 3), to: (12, 21)))
                }
            case .opencode:
                SVGPath(BrandPaths.opencode).fill(Color.white, style: FillStyle(eoFill: true))
            case .gemini:
                ZStack {
                    SVGPath(BrandPaths.gemini).fill(Color(hex: 0x3186FF))
                    SVGPath(BrandPaths.gemini).fill(gradient([(0x08B962, 1, 0), (0x08B962, 0, 1)], from: (7, 15.5), to: (11, 12)))
                    SVGPath(BrandPaths.gemini).fill(gradient([(0xF94543, 1, 0), (0xF94543, 0, 1)], from: (8, 5.5), to: (11.5, 11)))
                    SVGPath(BrandPaths.gemini).fill(gradient([(0xFABC12, 1, 0), (0xFABC12, 0, 0.46)], from: (3.5, 13.5), to: (17.5, 12)))
                }
            case .shell, .other:
                LucideIcon(.terminal, size: size, strokeWidth: 1.5).foregroundStyle(Theme.gray400)
            }
        }
        .frame(width: size, height: size)
    }

    /// A user-space SVG linear gradient on the 24-unit grid, scaled to this icon.
    private func gradient(_ stops: [(UInt32, Double, CGFloat)], from: (CGFloat, CGFloat), to: (CGFloat, CGFloat)) -> LinearGradient {
        LinearGradient(
            stops: stops.map { Gradient.Stop(color: Color(hex: $0.0, opacity: $0.1), location: $0.2) },
            startPoint: UnitPoint(x: from.0 / 24, y: from.1 / 24),
            endPoint: UnitPoint(x: to.0 / 24, y: to.1 / 24))
    }
}

/// Today's running indicator: a 4x4 grid of rounded cells whose inner and outer
/// rings pulse out of phase (RunningGlyph.tsx).
public struct RunningGlyph: View {
    let size: CGFloat
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(size: CGFloat = 16) { self.size = size }

    private enum Tier { case inner, outer, hidden }

    private static let tiers: [Tier] = [
        .hidden, .outer, .outer, .hidden,
        .outer, .inner, .inner, .outer,
        .outer, .inner, .inner, .outer,
        .hidden, .outer, .outer, .hidden,
    ]

    private static func seeded(_ seed: Double) -> Double {
        let s = sin(seed * 12.9898 + 78.233) * 43758.5453
        return s - floor(s)
    }

    private static let timing: [(delay: Double, duration: Double)] = (0..<16).map { i in
        ((seeded(Double(i + 1)) * 1.3 * 1000).rounded() / 1000,
         ((0.95 + seeded(Double(i + 101)) * 0.8) * 1000).rounded() / 1000)
    }

    public var body: some View {
        TimelineView(.animation(paused: reduceMotion)) { ctx in
            Canvas { gc, sz in
                let unit = sz.width / 15
                let t = ctx.date.timeIntervalSinceReferenceDate
                for (i, tier) in Self.tiers.enumerated() where tier != .hidden {
                    let rect = CGRect(x: CGFloat(i % 4) * 4 * unit, y: CGFloat(i / 4) * 4 * unit, width: 3 * unit, height: 3 * unit)
                    let (lo, hi) = tier == .inner ? (0.35, 0.95) : (0.12, 0.38)
                    let opacity: Double
                    if reduceMotion {
                        opacity = 0.5
                    } else {
                        let (delay, duration) = Self.timing[i]
                        let phase = ((t - delay) / duration).truncatingRemainder(dividingBy: 1)
                        let p = phase < 0 ? phase + 1 : phase
                        // ease-in-out between the 0%/100% and 50% keyframes
                        let half = p < 0.5 ? p * 2 : (1 - p) * 2
                        opacity = lo + (hi - lo) * Self.easeInOut(half)
                    }
                    gc.fill(Path(roundedRect: rect, cornerRadius: unit), with: .color(.white.opacity(0.85 * opacity)))
                }
            }
        }
        .frame(width: size, height: size)
        .accessibilityLabel("Running")
    }

    /// CSS `ease-in-out`, cubic-bezier(0.42, 0, 0.58, 1).
    static func easeInOut(_ x: Double) -> Double {
        var t = x
        for _ in 0..<6 {
            let bx = 3 * (1 - t) * (1 - t) * t * 0.42 + 3 * (1 - t) * t * t * 0.58 + t * t * t
            let d = 3 * (1 - t) * (1 - t) * 0.42 + 6 * (1 - t) * t * (0.58 - 0.42) + 3 * t * t * (1 - 0.58)
            if d == 0 { break }
            t -= (bx - x) / d
            t = min(max(t, 0), 1)
        }
        return 3 * (1 - t) * t * t + t * t * t
    }
}

/// The agent's mark, or the running glyph while an agent is working (AgentStatusIcon.tsx).
public struct AgentStatusIcon: View {
    let agent: AgentType
    let status: AgentStatus
    let size: CGFloat

    public init(_ agent: AgentType, status: AgentStatus, size: CGFloat = 14) {
        self.agent = agent
        self.status = status
        self.size = size
    }

    public var body: some View {
        if agent == .shell || status != .running {
            AgentIcon(agent, size: size)
        } else {
            RunningGlyph(size: size)
        }
    }
}

/// A project's icon from config (ProjectIcon.tsx); the folder outline when none is set.
public struct ProjectIcon: View {
    let icon: String?
    let color: String?
    let size: CGFloat

    public init(icon: String?, color: String?, size: CGFloat = 14) {
        self.icon = icon
        self.color = color
        self.size = size
    }

    private var tint: Color { color.flatMap(Color.init(css:)) ?? Color(hex: 0x6B7280) }

    public var body: some View {
        switch icon {
        case "github":
            FilledPaths(paths: [BrandPaths.github], viewBox: 16).fill(tint).frame(width: size, height: size)
        case "linear":
            FilledPaths(paths: BrandPaths.linear).fill(tint).frame(width: size, height: size)
        case "mcp":
            FilledPaths(paths: BrandPaths.mcp).fill(tint, style: FillStyle(eoFill: true)).frame(width: size, height: size)
        case let name? where Lucide(componentName: name) != nil:
            LucideIcon(Lucide(componentName: name)!, size: size, strokeWidth: 1.5).foregroundStyle(tint)
        default:
            SVGPath(BrandPaths.folder)
                .stroke(style: StrokeStyle(lineWidth: 1.5 * size / 24))
                .frame(width: size, height: size)
                .foregroundStyle(color.flatMap(Color.init(css:)) ?? Color.primary)
        }
    }
}

/// A workspace's icon (WorkspaceSwitcher.tsx): a lucide icon, the user glyph by default.
public struct WorkspaceIcon: View {
    let icon: String?
    let color: String?
    let size: CGFloat

    public init(icon: String?, color: String?, size: CGFloat = 14) {
        self.icon = icon
        self.color = color
        self.size = size
    }

    public var body: some View {
        LucideIcon(icon.flatMap(Lucide.init(componentName:)) ?? .user, size: size, strokeWidth: 1.5)
            .foregroundStyle(color.flatMap(Color.init(css:)) ?? Color(hex: 0x6B7280))
    }
}

/// The Vorn mark from the renderer's assets.
public struct VornLogo: View {
    let height: CGFloat

    public init(height: CGFloat = 32) { self.height = height }

    // Loaded from the file, so offscreen renders draw it too.
    private static let image: Image = {
        guard let url = Bundle.module.url(forResource: "vorn-logo", withExtension: "png") else { return Image(systemName: "questionmark") }
        #if canImport(AppKit)
        return NSImage(contentsOf: url).map(Image.init(nsImage:)) ?? Image(systemName: "questionmark")
        #else
        return UIImage(contentsOfFile: url.path).map(Image.init(uiImage:)) ?? Image(systemName: "questionmark")
        #endif
    }()

    public var body: some View {
        Self.image
            .resizable().interpolation(.high).aspectRatio(contentMode: .fit).frame(height: height)
    }
}
