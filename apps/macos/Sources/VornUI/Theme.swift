import SwiftUI

// Values from src/renderer/theme.css and the Tailwind v4 defaults the renderer
// uses (grays are its oklch values converted to sRGB).
public enum Theme {
    public static let surfaceBase = Color(hex: 0x0D0D0F)
    public static let surfaceSunken = Color(hex: 0x141416)
    public static let surfacePanel = Color(hex: 0x101012)
    public static let surfaceOverlay = Color(hex: 0x1C1C20)
    public static let surfaceRaised = surfaceSunken
    public static let surfaceNode = surfaceSunken

    public static let bronzo = Color(hex: 0xC9972A)
    public static let bronzoDark = Color(hex: 0xB8862A)
    public static let ink = Color(hex: 0xFAF9F7)
    public static let inkSecondary = Color.white.opacity(0.55)
    public static let inkFaint = Color.white.opacity(0.35)
    public static let inkGhost = Color.white.opacity(0.18)

    public static let danger = Color(hex: 0xD4623F)
    public static let statusSage = Color(hex: 0x7D9471)
    public static let statusSlate = Color(hex: 0x7D8590)
    public static let statusBlue = Color(hex: 0x6F8FAF)

    public static let gray200 = Color(hex: 0xE5E7EB)
    public static let gray300 = Color(hex: 0xD1D5DC)
    public static let gray400 = Color(hex: 0x99A1AF)
    public static let gray500 = Color(hex: 0x6A7282)
    public static let gray600 = Color(hex: 0x4A5565)

    public static func white(_ opacity: Double) -> Color { Color.white.opacity(opacity) }

    // Tailwind radii: rounded = 4, md = 6, lg = 8, xl = 12.
    public static let radius: CGFloat = 4
    public static let radiusMd: CGFloat = 6
    public static let radiusLg: CGFloat = 8
    public static let radiusXl: CGFloat = 12

    public static let toolbarHeight: CGFloat = 40
    public static let trafficLightPad: CGFloat = 80
}

extension Color {
    public init(hex: UInt32) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255
        )
    }

    /// Parses "#rgb" or "#rrggbb"; nil for anything else.
    public init?(cssHex: String) {
        var s = cssHex.trimmingCharacters(in: .whitespaces)
        guard s.hasPrefix("#") else { return nil }
        s.removeFirst()
        if s.count == 3 { s = s.map { "\($0)\($0)" }.joined() }
        guard s.count == 6, let v = UInt32(s, radix: 16) else { return nil }
        self.init(hex: v)
    }
}

/// `w-px h-4 bg-white/[0.06] mx-0.5`, the divider between top-bar groups.
public struct ToolbarDivider: View {
    public init() {}

    public var body: some View {
        Rectangle()
            .fill(Theme.white(0.06))
            .frame(width: 1, height: 16)
            .padding(.horizontal, 2)
    }
}

/// Close/minimise/zoom where the window puts them, for renders without a window.
public struct FauxTrafficLights: View {
    public init() {}

    public var body: some View {
        HStack(spacing: 8) {
            light(0xFF5F57, 0xE14640)
            light(0xFEBC2E, 0xDFA023)
            light(0x28C840, 0x1AAB29)
        }
        .padding(.leading, 16)
        .padding(.top, 14)
    }

    private func light(_ fill: UInt32, _ edge: UInt32) -> some View {
        Circle()
            .fill(Color(hex: fill))
            .overlay(Circle().strokeBorder(Color(hex: edge), lineWidth: 0.5))
            .frame(width: 12, height: 12)
    }
}
