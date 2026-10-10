import SwiftUI

/// Values copied from src/renderer/theme.css and the utility class defaults the
/// renderer uses (grays are their oklch values converted to sRGB).
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

    public static let gray200 = Color(hex: 0xE5E7EB)
    public static let gray300 = Color(hex: 0xD1D5DC)
    public static let gray400 = Color(hex: 0x99A1AF)
    public static let gray500 = Color(hex: 0x6A7282)
    public static let gray600 = Color(hex: 0x4A5565)
    public static let blue400 = Color(hex: 0x51A2FF)
    public static let blue500 = Color(hex: 0x2B7FFF)
    public static let red400 = Color(hex: 0xFF6467)
    public static let red500 = Color(hex: 0xFB2C36)

    public static func white(_ opacity: Double) -> Color { Color.white.opacity(opacity) }

    // Radii of the renderer classes: rounded-sm = 2, rounded = 4, md = 6, lg = 8, xl = 12.
    public static let radiusSm: CGFloat = 2
    public static let radius: CGFloat = 4
    public static let radiusMd: CGFloat = 6
    public static let radiusLg: CGFloat = 8
    public static let radiusXl: CGFloat = 12

    public static let toolbarHeight: CGFloat = 40
    public static let trafficLightPad: CGFloat = 80
}

public extension Color {
    init(hex: UInt32) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255
        )
    }

    /// `#rgb` or `#rrggbb`, as a project's icon colour is stored.
    init?(cssHex: String) {
        var s = cssHex.trimmingCharacters(in: .whitespaces)
        if s.hasPrefix("#") { s.removeFirst() }
        if s.count == 3 { s = s.map { "\($0)\($0)" }.joined() }
        guard s.count == 6, let v = UInt32(s, radix: 16) else { return nil }
        self.init(hex: v)
    }
}

/// The renderer's `shadow-xl`/`shadow-2xl` for floating menus and dialogs.
public extension View {
    func floatingShadow() -> some View {
        shadow(color: .black.opacity(0.25), radius: 25, x: 0, y: 25)
    }
}
