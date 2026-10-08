// The look test's screen in SwiftUI, shared by macOS, iPadOS and iOS: the
// app's sidebar, one session card with a live terminal (the same pane view
// the bench uses, drawn by option A or B) and a small diff card, with the
// desktop app's colours, fonts, sizes and spacing. With `polish`, the
// sidebar sits on the system's sidebar material and the icons are SF
// Symbols. Wide windows get the desktop layout; an iPad in portrait keeps
// the sidebar and stacks the cards; a phone shows the cards only.

import SwiftUI
#if os(macOS)
import AppKit
#else
import UIKit
#endif

extension Color {
    init(hex: UInt32, _ a: Double = 1) {
        self.init(.sRGB, red: Double((hex >> 16) & 0xff) / 255,
                  green: Double((hex >> 8) & 0xff) / 255, blue: Double(hex & 0xff) / 255, opacity: a)
    }
}

/// The app's dark theme tokens.
enum Tok {
    static let base = Color(hex: 0x0d0d0f)
    static let panel = Color(hex: 0x101012)
    static let sunken = Color(hex: 0x141416)
    static let overlay = Color(hex: 0x1c1c20)
    static let ink = Color(hex: 0xfaf9f7)
    static let inkSecondary = Color.white.opacity(0.55)
    static let line = Color.white.opacity(0.06)
    static let hairline = Color.white.opacity(0.04)
    static let gray300 = Color(hex: 0xd1d5db)
    static let gray400 = Color(hex: 0x9ca3af)
    static let gray500 = Color(hex: 0x6b7280)
    static let gray600 = Color(hex: 0x4b5563)
    static let running = Color(hex: 0xfaf9f7)
    static let waiting = Color(hex: 0xc9972a)
    static let idle = Color.white.opacity(0.18)
}

// MARK: - Icons (the app's stroke icons, 24-unit grid, 1.5 stroke)

struct LineIcon: Shape {
    let name: String

    func path(in r: CGRect) -> Path {
        var p = Path()
        func rr(_ x: CGFloat, _ y: CGFloat, _ w: CGFloat, _ h: CGFloat, _ c: CGFloat) {
            p.addRoundedRect(in: CGRect(x: x, y: y, width: w, height: h), cornerSize: CGSize(width: c, height: c))
        }
        func lines(_ pts: [(CGFloat, CGFloat)]) {
            p.move(to: CGPoint(x: pts[0].0, y: pts[0].1))
            for q in pts.dropFirst() { p.addLine(to: CGPoint(x: q.0, y: q.1)) }
        }
        func q(_ cx: CGFloat, _ cy: CGFloat, _ x: CGFloat, _ y: CGFloat) {
            p.addQuadCurve(to: CGPoint(x: x, y: y), control: CGPoint(x: cx, y: cy))
        }
        func l(_ x: CGFloat, _ y: CGFloat) { p.addLine(to: CGPoint(x: x, y: y)) }
        switch name {
        case "terminal":
            lines([(4, 17), (10, 11), (4, 5)]); lines([(12, 19), (20, 19)])
        case "folder":
            p.move(to: CGPoint(x: 3, y: 7)); l(3, 17); q(3, 19, 5, 19); l(19, 19); q(21, 19, 21, 17)
            l(21, 9); q(21, 7, 19, 7); l(13, 7); l(11, 5); l(5, 5); q(3, 5, 3, 7); p.closeSubpath()
        case "folder-open":
            p.move(to: CGPoint(x: 6, y: 14)); l(7.5, 11.1); q(8, 10, 9.24, 10); l(20, 10); q(22.4, 10, 21.94, 12.5)
            l(20.4, 18.5); q(20, 20, 18.45, 20); l(4, 20); q(2, 20, 2, 18); l(2, 5); q(2, 3, 4, 3); l(7.9, 3)
            q(9, 3, 9.59, 3.9); l(10.4, 5.1); q(11, 6, 12.07, 6); l(18, 6); q(20, 6, 20, 8); l(20, 10)
        case "square-terminal":
            lines([(7, 11), (9, 9), (7, 7)]); lines([(11, 13), (15, 13)]); rr(3, 3, 18, 18, 2)
        case "globe":
            p.addEllipse(in: CGRect(x: 2, y: 2, width: 20, height: 20))
            p.move(to: CGPoint(x: 12, y: 2)); q(4, 12, 12, 22); q(20, 12, 12, 2)
            lines([(2, 12), (22, 12)])
        case "git-branch":
            lines([(6, 3), (6, 15)])
            p.addEllipse(in: CGRect(x: 15, y: 3, width: 6, height: 6))
            p.addEllipse(in: CGRect(x: 3, y: 15, width: 6, height: 6))
            p.move(to: CGPoint(x: 18, y: 9))
            p.addArc(center: CGPoint(x: 9, y: 9), radius: 9, startAngle: .degrees(0), endAngle: .degrees(90), clockwise: false)
        case "panel-left":
            rr(3, 3, 18, 18, 2); lines([(9, 3), (9, 21)])
        case "file-diff":
            p.move(to: CGPoint(x: 15, y: 2)); l(6, 2); q(4, 2, 4, 4); l(4, 20); q(4, 22, 6, 22); l(18, 22)
            q(20, 22, 20, 20); l(20, 7); p.closeSubpath()
            lines([(9, 10), (15, 10)]); lines([(12, 13), (12, 7)]); lines([(9, 17), (15, 17)])
        default: break
        }
        let s = min(r.width, r.height) / 24
        return p.applying(CGAffineTransform(scaleX: s, y: s).translatedBy(x: r.minX / s, y: r.minY / s))
    }
}

/// SF Symbols standing in for the stroke icons, for the polished variant.
let symbols: [String: String] = [
    "terminal": "terminal", "folder": "folder", "folder-open": "folder",
    "square-terminal": "apple.terminal", "globe": "globe", "git-branch": "arrow.triangle.branch",
    "panel-left": "sidebar.left", "file-diff": "doc.text",
]

struct Icon: View {
    let name: String
    let size: CGFloat
    let color: Color
    let polish: Bool

    var body: some View {
        if polish, let s = symbols[name] {
            Image(systemName: s)
                .font(.system(size: size * 0.8, weight: .regular))
                .foregroundStyle(color)
                .frame(width: size, height: size)
        } else {
            LineIcon(name: name)
                .stroke(color, style: StrokeStyle(lineWidth: 1.5 * size / 24, lineCap: .round, lineJoin: .round))
                .frame(width: size, height: size)
        }
    }
}

// MARK: - Sidebar

#if os(macOS)
struct VisualEffect: NSViewRepresentable {
    func makeNSView(context: Context) -> NSVisualEffectView {
        let v = NSVisualEffectView()
        v.material = .sidebar
        v.appearance = NSAppearance(named: .darkAqua)
        v.blendingMode = .behindWindow
        v.state = .active
        return v
    }
    func updateNSView(_ v: NSVisualEffectView, context: Context) {}
}
#else
struct VisualEffect: View {
    var body: some View { Rectangle().fill(.regularMaterial) }
}
#endif

/// Dim text: a fixed grey on the flat panel; on a translucent material the
/// system's secondary style, which keeps its contrast whatever is behind.
func dim(_ polish: Bool) -> AnyShapeStyle {
    polish ? AnyShapeStyle(.secondary) : AnyShapeStyle(Tok.gray600)
}

struct SessionRow: View {
    let name: String
    let branch: String
    let status: Color
    let selected: Bool
    let polish: Bool

    var body: some View {
        HStack(spacing: 8) {
            Icon(name: "terminal", size: 14, color: Tok.gray400, polish: polish)
            VStack(alignment: .leading, spacing: 0) {
                Text(name).font(.system(size: 12))
                    .foregroundStyle(selected ? AnyShapeStyle(Color.white) : polish ? AnyShapeStyle(.primary) : AnyShapeStyle(Tok.gray400))
                Text(branch).font(.system(size: 10)).foregroundStyle(dim(polish))
            }
            Spacer(minLength: 0)
            Circle().fill(status).frame(width: 6, height: 6)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .overlay(alignment: .leading) {
            if selected { Rectangle().fill(Color.white).frame(width: 1).padding(.vertical, 4) }
        }
    }
}

#if os(macOS)
let trafficLights: CGFloat = 80
#else
let trafficLights: CGFloat = 12
#endif

struct Sidebar: View {
    let polish: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // The titlebar row; the traffic lights sit on its left.
            HStack {
                Spacer()
                Icon(name: "panel-left", size: 14, color: Tok.gray500, polish: polish)
                    .padding(4)
                    .help("Hide sidebar")
            }
            .padding(.leading, trafficLights).padding(.trailing, 12)
            .frame(height: 40)
            .overlay(alignment: .bottom) { Rectangle().fill(Tok.line).frame(height: 1) }
            VStack(alignment: .leading, spacing: 2) {
                Text("SESSIONS")
                    .font(.system(size: 11, weight: .medium)).kerning(0.55)
                    .foregroundStyle(polish ? AnyShapeStyle(.secondary) : AnyShapeStyle(Tok.gray500))
                    .padding(.top, 12).padding(.bottom, 6)
                HStack(spacing: 8) {
                    Icon(name: "folder", size: 14, color: Tok.gray500, polish: polish)
                    Text("vorn").font(.system(size: 13)).foregroundStyle(Color.white)
                    Spacer()
                    Text("3").font(.system(size: 12)).foregroundStyle(dim(polish))
                }
                .padding(.horizontal, 8).padding(.vertical, 6)
                .background(RoundedRectangle(cornerRadius: 4).fill(Color.white.opacity(0.08)))
                VStack(spacing: 2) {
                    SessionRow(name: "fix flaky resume test", branch: "fix/resume-flake",
                               status: Tok.running, selected: true, polish: polish)
                    SessionRow(name: "grid protocol docs", branch: "docs/grid-mode",
                               status: Tok.waiting, selected: false, polish: polish)
                    SessionRow(name: "native ui spike", branch: "native-ui-spike",
                               status: Tok.idle, selected: false, polish: polish)
                }
                .padding(.leading, 16)
            }
            .padding(.horizontal, 12)
            Spacer()
        }
        .frame(width: 255)
        .environment(\.colorScheme, .dark)
        .background {
            if polish { VisualEffect() } else { Tok.panel }
        }
    }
}

// MARK: - Cards

struct IconButton: View {
    let icon: String
    let tip: String
    let polish: Bool

    var body: some View {
        Icon(name: icon, size: 14, color: Tok.ink, polish: polish)
            .padding(4)
            .contentShape(Rectangle())
            .help(tip)
    }
}

struct CardHeader<Trailing: View>: View {
    let icon: String
    let title: String
    let branch: String?
    let polish: Bool
    @ViewBuilder let trailing: Trailing

    var body: some View {
        HStack(spacing: 8) {
            Icon(name: icon, size: 18, color: Tok.gray400, polish: polish)
            Text(title).font(.system(size: 13, weight: .medium)).foregroundStyle(Tok.gray300)
            if let branch {
                HStack(spacing: 4) {
                    Icon(name: "git-branch", size: 11, color: Tok.gray500, polish: polish)
                    Text(branch).font(.system(size: 11, design: .monospaced)).foregroundStyle(Tok.gray400)
                }
                .padding(.horizontal, 4).padding(.vertical, 2)
            }
            Spacer(minLength: 0)
            trailing
        }
        .padding(.horizontal, 12).padding(.vertical, 10)
        .frame(height: 41, alignment: .center)
        .overlay(alignment: .bottom) { Rectangle().fill(Tok.hairline).frame(height: 1) }
    }
}

struct Card<Content: View>: View {
    @ViewBuilder let content: Content
    var body: some View {
        VStack(spacing: 0) { content }
            .background(Tok.sunken)
            .overlay(Rectangle().strokeBorder(Tok.line, lineWidth: 1))
    }
}

struct DiffLine: Identifiable {
    let id: Int
    let kind: Character // " ", "+", "-", "@"
    let old: String
    let new: String
    let text: String
}

let diffLines: [DiffLine] = [
    DiffLine(id: 0, kind: "@", old: "", new: "", text: "@@ -14,7 +14,8 @@ export const TONE_DOT = {"),
    DiffLine(id: 1, kind: " ", old: "14", new: "14", text: "  broken: 'bg-danger',"),
    DiffLine(id: 2, kind: " ", old: "15", new: "15", text: "  blocked: 'bg-bronzo',"),
    DiffLine(id: 3, kind: "-", old: "16", new: "", text: "  settled: 'bg-ink-faint',"),
    DiffLine(id: 4, kind: "+", old: "", new: "16", text: "  settled: 'bg-ink-faint/80',"),
    DiffLine(id: 5, kind: "+", old: "", new: "17", text: "  parked: 'bg-ink-ghost',"),
    DiffLine(id: 6, kind: " ", old: "17", new: "18", text: "  live: 'bg-ink',"),
    DiffLine(id: 7, kind: " ", old: "18", new: "19", text: "  idle: 'bg-ink-ghost'"),
    DiffLine(id: 8, kind: " ", old: "19", new: "20", text: "}"),
]

struct DiffView: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                Text("src/renderer/lib/status-tone.ts").foregroundStyle(Tok.gray300)
                Spacer()
                Text("+2").foregroundStyle(Color(hex: 0x7ea96a))
                Text("−1").foregroundStyle(Color(hex: 0xc96f62))
            }
            .font(.system(size: 12, design: .monospaced))
            .padding(.horizontal, 12).padding(.vertical, 6)
            .background(Tok.overlay)
            .overlay(alignment: .bottom) { Rectangle().fill(Tok.line).frame(height: 1) }
            ForEach(diffLines) { line in
                if line.kind == "@" {
                    Text(line.text)
                        .font(.system(size: 12, design: .monospaced))
                        .foregroundStyle(Tok.inkSecondary)
                        .padding(.horizontal, 12).padding(.vertical, 2)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Color.white.opacity(0.05))
                } else {
                    let add = line.kind == "+", del = line.kind == "-"
                    HStack(spacing: 0) {
                        Text(line.old).frame(width: 35 - 8, alignment: .trailing).padding(.trailing, 8)
                            .foregroundStyle(del ? Color(hex: 0xdc2626) : Tok.gray600)
                        Text(line.new).frame(width: 35 - 8, alignment: .trailing).padding(.trailing, 8)
                            .foregroundStyle(add ? Color(hex: 0x16a34a) : Tok.gray600)
                        Text(line.text)
                            .font(.system(size: 12, design: .monospaced))
                            .foregroundStyle(add ? Color(hex: 0x86efac) : del ? Color(hex: 0xfca5a5) : Tok.gray400)
                            .padding(.horizontal, 1)
                        Spacer(minLength: 0)
                    }
                    .font(.system(size: 11, design: .monospaced))
                    .frame(height: 19.2)
                    .background(add ? Color(hex: 0x22c55e, 0.10) : del ? Color(hex: 0xef4444, 0.10) : .clear)
                }
            }
        }
    }
}

// MARK: - The screen

struct TerminalCard: View {
    let term: PaneRep
    let polish: Bool

    var body: some View {
        Card {
            CardHeader(icon: "terminal", title: "fix flaky resume test", branch: "fix/resume-flake",
                       polish: polish) {
                HStack(spacing: 2) {
                    IconButton(icon: "folder-open", tip: "Browse files", polish: polish)
                    IconButton(icon: "square-terminal", tip: "Add a terminal", polish: polish)
                    IconButton(icon: "globe", tip: "Open browser", polish: polish)
                }
            }
            term.padding(.top, 2).padding(.horizontal, 1).padding(.bottom, 1)
        }
    }
}

struct DiffCard: View {
    let polish: Bool

    var body: some View {
        Card {
            CardHeader(icon: "file-diff", title: "Changes", branch: nil, polish: polish) {
                Text("1 file").font(.system(size: 11)).foregroundStyle(Tok.gray500)
            }
            DiffView()
            Spacer(minLength: 0)
        }
    }
}

struct LookView: View {
    let term: PaneRep
    let polish: Bool

    var body: some View {
        GeometryReader { g in
            let w = g.size.width
            HStack(spacing: 0) {
                if w >= 700 {
                    Sidebar(polish: polish)
                    Rectangle().fill(Tok.line).frame(width: 1).background(Tok.base)
                }
                ZStack(alignment: .topLeading) {
                    Tok.base
                    if w >= 1300 {
                        // The desktop layout, at the desktop app's sizes.
                        HStack(alignment: .top, spacing: 0) {
                            TerminalCard(term: term, polish: polish)
                                .frame(width: 744).frame(maxHeight: .infinity)
                            DiffCard(polish: polish).frame(width: 440, height: 450)
                        }
                    } else {
                        VStack(spacing: 8) {
                            TerminalCard(term: term, polish: polish).frame(maxHeight: .infinity)
                            DiffCard(polish: polish).frame(height: 260)
                        }
                        .padding(w >= 700 ? 8 : 0)
                    }
                }
            }
        }
        #if os(macOS)
        .ignoresSafeArea()
        #endif
        .background(Tok.base.ignoresSafeArea())
    }
}

/// The bench's window: N panes in a grid.
struct GridView: View {
    let panes: [PaneRep]
    let columns: Int

    var body: some View {
        let rows = (panes.count + columns - 1) / columns
        Grid(horizontalSpacing: 2, verticalSpacing: 2) {
            ForEach(0..<rows, id: \.self) { r in
                GridRow {
                    ForEach(0..<columns, id: \.self) { c in
                        let i = r * columns + c
                        if i < panes.count { panes[i] } else { Color.clear }
                    }
                }
            }
        }
        .background(Color(white: 0.25))
    }
}
