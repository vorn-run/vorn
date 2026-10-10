import SwiftUI

/// Whether the pointer is over the view, for today's `hover:` styles.
public struct Hovering<Content: View>: View {
    @State private var hovering = false
    let content: (Bool) -> Content

    public init(@ViewBuilder _ content: @escaping (Bool) -> Content) { self.content = content }

    public var body: some View {
        content(hovering).onHover { hovering = $0 }
    }
}

/// A borderless icon button: `padding` around the glyph, hover tint and fill as the
/// renderer's `p-1 rounded-md text-gray-400 hover:text-white` family.
public struct IconButton: View {
    let icon: Lucide
    let size: CGFloat
    let strokeWidth: CGFloat
    let padding: CGFloat
    let radius: CGFloat
    let color: Color
    let hoverColor: Color
    let hoverFill: Color
    let help: String?
    let action: () -> Void

    public init(_ icon: Lucide, size: CGFloat = 16, strokeWidth: CGFloat = 2, padding: CGFloat = 4,
                radius: CGFloat = Theme.radiusMd, color: Color = Theme.gray400, hoverColor: Color = .white,
                hoverFill: Color = .clear, help: String? = nil, action: @escaping () -> Void) {
        self.icon = icon
        self.size = size
        self.strokeWidth = strokeWidth
        self.padding = padding
        self.radius = radius
        self.color = color
        self.hoverColor = hoverColor
        self.hoverFill = hoverFill
        self.help = help
        self.action = action
    }

    public var body: some View {
        Hovering { hover in
            Button(action: action) {
                LucideIcon(icon, size: size, strokeWidth: strokeWidth)
                    .foregroundStyle(hover ? hoverColor : color)
                    .padding(padding)
                    .background(RoundedRectangle(cornerRadius: radius).fill(hover ? hoverFill : .clear))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .help(help ?? "")
        .accessibilityLabel(help ?? "")
    }
}

/// `w-px h-4 bg-white/[0.06]`: the hairline between toolbar groups.
public struct VDivider: View {
    let height: CGFloat
    public init(height: CGFloat = 16) { self.height = height }
    public var body: some View {
        Rectangle().fill(Theme.hairline).frame(width: 1, height: height)
    }
}

/// A keycap, as in the composer's tip line.
public struct Kbd: View {
    let text: String
    public init(_ text: String) { self.text = text }
    public var body: some View {
        Text(text)
            .font(.system(size: 10, design: .monospaced))
            .foregroundStyle(Theme.gray500)
            .padding(.horizontal, 4)
            .padding(.vertical, 2)
            .background(RoundedRectangle(cornerRadius: Theme.radius).fill(Theme.white(0.06)))
    }
}

/// Tooltip text with an optional shortcut, as today's `<Tooltip label shortcut>`.
public extension View {
    func tooltip(_ label: String, shortcut: String? = nil) -> some View {
        help(shortcut.map { "\(label)  \($0)" } ?? label)
    }
}

public extension Font {
    /// The UI font at a pixel size from the renderer (`text-[13px]`, `text-xs`...).
    static func ui(_ size: CGFloat, weight: Font.Weight = .regular) -> Font {
        .system(size: size, weight: weight)
    }
}
