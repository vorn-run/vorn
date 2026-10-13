import SwiftUI

public extension EnvironmentValues {
    /// Off where glass cannot draw (offscreen renders), so the navigation layer shows its opaque fallback.
    @Entry var navigationGlass = true
}

public extension View {
    /// The navigation layer's material: Liquid Glass on macOS 26, an opaque fill and hairline before it.
    func navigationGlass<S: InsettableShape>(_ shape: S, fallback: Color = Theme.surfaceOverlay) -> some View {
        modifier(NavigationGlass(shape: shape, fallback: fallback))
    }
}

public extension View {
    /// A panel hanging from the navigation layer: glass on macOS 26, an opaque fill, hairline and shadow before it.
    func navigationPanel(radius: CGFloat, fallback: Color = Theme.surfaceOverlay, border: Double = 0.08) -> some View {
        modifier(NavigationPanel(shape: RoundedRectangle(cornerRadius: radius), fallback: fallback, border: border))
    }
}

private struct NavigationPanel: ViewModifier {
    let shape: RoundedRectangle
    let fallback: Color
    let border: Double
    @Environment(\.navigationGlass) private var enabled

    func body(content: Content) -> some View {
        if #available(macOS 26, *), enabled {
            content.clipShape(shape).glassEffect(.regular, in: shape)
        } else {
            content
                .background(fallback, in: shape)
                .overlay(shape.strokeBorder(Theme.white(border), lineWidth: 1))
                .clipShape(shape)
                .shadow(color: .black.opacity(0.5), radius: 20, y: 10)
        }
    }
}

private struct NavigationGlass<S: InsettableShape>: ViewModifier {
    let shape: S
    let fallback: Color
    @Environment(\.navigationGlass) private var enabled

    func body(content: Content) -> some View {
        if #available(macOS 26, *), enabled {
            content.glassEffect(.regular, in: shape)
        } else {
            content
                .background(fallback, in: shape)
                .overlay(shape.strokeBorder(Theme.white(0.08), lineWidth: 1))
        }
    }
}

/// Groups glass shapes so nearby ones blend and share one sampling region, as the system toolbar does.
public struct NavigationGlassGroup<Content: View>: View {
    let spacing: CGFloat
    @ViewBuilder let content: Content
    @Environment(\.navigationGlass) private var enabled

    public init(spacing: CGFloat = 8, @ViewBuilder content: () -> Content) {
        self.spacing = spacing
        self.content = content()
    }

    public var body: some View {
        if #available(macOS 26, *), enabled {
            GlassEffectContainer(spacing: spacing) { content }
        } else {
            content
        }
    }
}

/// A 28pt borderless toolbar button around a monochrome SF Symbol, dimmed until hovered or on.
public struct ToolbarSymbolButton: View {
    let symbol: String
    let help: String
    let shortcut: String?
    let isOn: Bool
    let action: () -> Void

    public init(_ symbol: String, help: String, shortcut: String? = nil, isOn: Bool = false,
                action: @escaping () -> Void) {
        self.symbol = symbol
        self.help = help
        self.shortcut = shortcut
        self.isOn = isOn
        self.action = action
    }

    public var body: some View {
        Hovering { hover in
            Button(action: action) {
                Image(systemName: symbol)
                    .font(.system(size: 13, weight: .regular))
                    .symbolRenderingMode(.monochrome)
                    .foregroundStyle(isOn || hover ? Theme.ink : Theme.white(0.6))
                    .frame(width: 28, height: 28)
                    .background(Capsule().fill(isOn ? Theme.white(0.1) : .clear))
                    .contentShape(Capsule())
            }
            .buttonStyle(.plain)
        }
        .tooltip(help, shortcut: shortcut)
        .accessibilityLabel(help)
    }
}

/// The 5pt status dot a toolbar control carries in its corner.
public struct StatusDot: View {
    let color: Color
    public init(_ color: Color) { self.color = color }
    public var body: some View {
        Circle().fill(color).frame(width: 5, height: 5).padding(3).accessibilityHidden(true)
    }
}
