import SwiftUI

// Values copied from src/renderer/theme.css and the Tailwind v4 defaults the
// renderer uses (grays are its oklch values converted to sRGB).
enum Theme {
    static let surfaceBase = Color(hex: 0x0D0D0F)
    static let surfaceSunken = Color(hex: 0x141416)
    static let surfacePanel = Color(hex: 0x101012)
    static let surfaceOverlay = Color(hex: 0x1C1C20)
    static let surfaceRaised = surfaceSunken

    static let bronzo = Color(hex: 0xC9972A)
    static let ink = Color(hex: 0xFAF9F7)
    static let inkSecondary = Color.white.opacity(0.55)
    static let inkFaint = Color.white.opacity(0.35)
    static let inkGhost = Color.white.opacity(0.18)

    static let gray200 = Color(hex: 0xE5E7EB)
    static let gray300 = Color(hex: 0xD1D5DC)
    static let gray400 = Color(hex: 0x99A1AF)
    static let gray500 = Color(hex: 0x6A7282)
    static let gray600 = Color(hex: 0x4A5565)

    static func white(_ opacity: Double) -> Color { Color.white.opacity(opacity) }

    // Tailwind radii: rounded = 4, md = 6, lg = 8, xl = 12.
    static let radius: CGFloat = 4
    static let radiusMd: CGFloat = 6
    static let radiusLg: CGFloat = 8
    static let radiusXl: CGFloat = 12

    static let toolbarHeight: CGFloat = 40
    static let trafficLightPad: CGFloat = 80
}

extension Color {
    init(hex: UInt32) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255
        )
    }
}
