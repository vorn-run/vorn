import SwiftUI

/// Today's design tokens: src/renderer/theme.css and the Tailwind v4 defaults
/// the renderer uses (grays are its oklch values in sRGB). 1 Tailwind unit = 4pt.
public enum Theme {
    public static let surfaceBase = Color(hex: 0x0D0D0F)
    public static let surfaceSunken = Color(hex: 0x141416)
    public static let surfacePanel = Color(hex: 0x101012)
    public static let surfaceOverlay = Color(hex: 0x1C1C20)
    public static let surfaceRaised = surfaceSunken

    public static let bronzo = Color(hex: 0xC9972A)
    public static let danger = Color(hex: 0xD4623F)
    public static let ink = Color(hex: 0xFAF9F7)
    public static let inkSecondary = Color.white.opacity(0.55)
    public static let inkFaint = Color.white.opacity(0.35)
    public static let inkGhost = Color.white.opacity(0.18)

    public static let statusSlate = Color(hex: 0x7D8590)
    public static let statusBlue = Color(hex: 0x6F8FAF)
    public static let statusSage = Color(hex: 0x7D9471)
    public static let diffAdd = Color(hex: 0x7EA96A)
    public static let diffRemove = Color(hex: 0xC96F62)

    public static let gray200 = Color(hex: 0xE5E7EB)
    public static let gray300 = Color(hex: 0xD1D5DC)
    public static let gray400 = Color(hex: 0x99A1AF)
    public static let gray500 = Color(hex: 0x6A7282)
    public static let gray600 = Color(hex: 0x4A5565)
    public static let red400 = Color(hex: 0xFF6467)

    public static func white(_ opacity: Double) -> Color { Color.white.opacity(opacity) }

    /// The hairline every divider and border uses: `white/[0.06]`.
    public static let hairline = white(0.06)

    // Tailwind radii: rounded = 4, md = 6, lg = 8, xl = 12.
    public static let radius: CGFloat = 4
    public static let radiusMd: CGFloat = 6
    public static let radiusLg: CGFloat = 8
    public static let radiusXl: CGFloat = 12

    public static let toolbarHeight: CGFloat = 40
    /// Room for the window buttons at `trafficLightPosition {16, 13}`.
    public static let trafficLightPad: CGFloat = 80

    public static let sidebarWidth: CGFloat = 256
    public static let sidebarMinWidth: CGFloat = 180
    public static let sidebarMaxWidth: CGFloat = 400
    public static let sidebarCollapsedWidth: CGFloat = 52

    /// Today's terminal palette (TERM_OPTIONS in terminal-registry.ts).
    public enum Terminal {
        public static let foreground: UInt32 = 0xD4D4D8
        public static let background: UInt32 = 0x141416
        public static let cursor: UInt32 = 0xD4D4D8
        public static let selection: UInt32 = 0x3F3F46
        public static let ansi: [UInt32] = [
            0x27272A, 0xEF4444, 0x22C55E, 0xEAB308, 0x3B82F6, 0xA855F7, 0x06B6D4, 0xD4D4D8,
            0x52525B, 0xF87171, 0x4ADE80, 0xFACC15, 0x60A5FA, 0xC084FC, 0x22D3EE, 0xFAFAFA,
        ]
        public static let fontSize: CGFloat = 13
        /// The first of today's font stack that macOS ships.
        public static let fontName = "Menlo"
    }
}

public extension Color {
    init(hex: UInt32, opacity: Double = 1) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255,
            opacity: opacity
        )
    }

    /// `#rgb` or `#rrggbb`; nil for anything else.
    init?(css: String) {
        var s = css.trimmingCharacters(in: .whitespaces)
        guard s.hasPrefix("#") else { return nil }
        s.removeFirst()
        if s.count == 3 { s = s.map { "\($0)\($0)" }.joined() }
        guard s.count == 6, let v = UInt32(s, radix: 16) else { return nil }
        self.init(hex: v)
    }
}
