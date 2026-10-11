import AppKit
import SwiftUI

/// A floating panel: surface-overlay, a white hairline, rounded corners, a deep shadow.
public struct MenuPanel<Content: View>: View {
    var width: CGFloat?
    var minWidth: CGFloat?
    var radius: CGFloat
    var border: Double
    var verticalPadding: CGFloat
    @ViewBuilder let content: Content

    public init(width: CGFloat? = nil, minWidth: CGFloat? = nil, radius: CGFloat = Theme.radiusLg,
                border: Double = 0.1, verticalPadding: CGFloat = 4, @ViewBuilder content: () -> Content) {
        self.width = width
        self.minWidth = minWidth
        self.radius = radius
        self.border = border
        self.verticalPadding = verticalPadding
        self.content = content()
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) { content }
            .padding(.vertical, verticalPadding)
            .frame(width: width)
            .frame(minWidth: minWidth, alignment: .leading)
            .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: radius))
            .overlay(RoundedRectangle(cornerRadius: radius).strokeBorder(Theme.white(border), lineWidth: 1))
            .clipShape(RoundedRectangle(cornerRadius: radius))
            .shadow(color: .black.opacity(0.5), radius: 20, y: 10)
    }
}

/// An uppercase section label: 10px gray-500, tracking-wider.
public struct MenuSectionLabel: View {
    let text: String
    var weight: Font.Weight

    public init(_ text: String, weight: Font.Weight = .regular) {
        self.text = text
        self.weight = weight
    }

    public var body: some View {
        Text(text.uppercased())
            .font(.ui(10, weight: weight))
            .tracking(0.5)
            .foregroundStyle(Theme.gray500)
            .padding(.horizontal, 12)
            .padding(.vertical, 4)
    }
}

/// OptionRow.tsx: a check (or its space), an optional dot, the label.
public struct MenuOptionRow: View {
    let label: String
    let selected: Bool
    let dot: Color?
    let help: String?
    let action: () -> Void

    public init(_ label: String, selected: Bool, dot: Color? = nil, help: String? = nil, action: @escaping () -> Void) {
        self.label = label
        self.selected = selected
        self.dot = dot
        self.help = help
        self.action = action
    }

    public var body: some View {
        Hovering { hovered in
            Button(action: action) {
                HStack(spacing: 8) {
                    Group {
                        if selected { LucideIcon(.check, size: 11, strokeWidth: 3) } else { Color.clear }
                    }
                    .frame(width: 11, height: 11)
                    if let dot { Circle().fill(dot).frame(width: 6, height: 6) }
                    Text(label)
                    Spacer(minLength: 0)
                }
                .font(.ui(12))
                .foregroundStyle(selected || hovered ? Color.white : Theme.gray300)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .background(selected ? Theme.white(0.06) : hovered ? Theme.white(0.04) : .clear)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(help ?? "")
        }
    }
}

/// A context-menu row (CardContextMenu.tsx): px-3 py-1.5, 12px gray-300, an icon, a quiet detail or a chevron.
public struct MenuEntry<Icon: View>: View {
    let label: String
    let detail: String?
    let shortcut: String?
    let submenu: Bool
    let highlighted: Bool
    let iconWidth: CGFloat
    let icon: Icon
    let action: () -> Void
    var onHover: (Bool) -> Void

    public init(_ label: String, detail: String? = nil, shortcut: String? = nil, submenu: Bool = false,
                highlighted: Bool = false, iconWidth: CGFloat = 14, onHover: @escaping (Bool) -> Void = { _ in },
                action: @escaping () -> Void, @ViewBuilder icon: () -> Icon) {
        self.label = label
        self.detail = detail
        self.shortcut = shortcut
        self.submenu = submenu
        self.highlighted = highlighted
        self.iconWidth = iconWidth
        self.icon = icon()
        self.action = action
        self.onHover = onHover
    }

    public var body: some View {
        Hovering { hovered in
            Button(action: action) {
                HStack(spacing: 10) {
                    icon.frame(width: iconWidth, height: iconWidth)
                    Text(label).lineLimit(1).truncationMode(.tail)
                    Spacer(minLength: 8)
                    if let detail, !detail.isEmpty {
                        Text(detail).font(.ui(10)).foregroundStyle(Theme.gray600)
                    }
                    if let shortcut {
                        Text(shortcut).font(.ui(10)).foregroundStyle(Theme.gray600)
                    }
                    if submenu { LucideIcon(.chevronRight, size: 11).foregroundStyle(Theme.gray600) }
                }
                .font(.ui(12))
                .foregroundStyle(Theme.gray300)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .frame(height: 30)
                .background(hovered || highlighted ? Theme.white(0.06) : .clear)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .onHover(perform: onHover)
        }
    }
}

/// A submenu's group heading: the project a list of worktrees belongs to.
public struct MenuGroupHeader<Icon: View>: View {
    let label: String
    let icon: Icon

    public init(_ label: String, @ViewBuilder icon: () -> Icon) {
        self.label = label
        self.icon = icon()
    }

    public var body: some View {
        HStack(spacing: 8) {
            icon.frame(width: 12, height: 12)
            Text(label.uppercased()).tracking(0.5).lineLimit(1)
        }
        .font(.ui(10, weight: .medium))
        .foregroundStyle(Theme.gray500)
        .padding(.horizontal, 12)
        .padding(.top, 8)
        .padding(.bottom, 4)
    }
}

/// border-t white/0.06, my-1.
public struct MenuRule: View {
    public init() {}
    public var body: some View { Rectangle().fill(Theme.hairline).frame(height: 1).padding(.vertical, 4) }
}

extension View {
    /// Reports this view's frame in `space` to `frame`, for anchoring a floating panel.
    public func reportFrame(_ frame: Binding<CGRect>, in space: CoordinateSpace = .global) -> some View {
        background(
            GeometryReader { g in
                Color.clear
                    .onAppear { frame.wrappedValue = g.frame(in: space) }
                    .onChange(of: g.frame(in: space)) { _, f in frame.wrappedValue = f }
            })
    }

    /// Calls `action` with the click's point in window space (top-left origin) on a right click,
    /// leaving every other event to the views underneath.
    public func onRightClick(_ action: @escaping (CGPoint) -> Void) -> some View {
        modifier(RightClickModifier(action: action))
    }
}

/// Whether views draw as static snapshots (offscreen renders, where an NSView cannot draw).
public struct TerminalSnapshotKey: EnvironmentKey {
    public static let defaultValue = false
}

public extension EnvironmentValues {
    var terminalSnapshot: Bool {
        get { self[TerminalSnapshotKey.self] }
        set { self[TerminalSnapshotKey.self] = newValue }
    }
}

private struct RightClickModifier: ViewModifier {
    let action: (CGPoint) -> Void
    @Environment(\.terminalSnapshot) private var snapshot

    func body(content: Content) -> some View {
        content.overlay { if !snapshot { RightClickCatcher(action: action) } }
    }
}

private struct RightClickCatcher: NSViewRepresentable {
    let action: (CGPoint) -> Void

    final class CatcherView: NSView {
        var action: (CGPoint) -> Void = { _ in }

        override var isFlipped: Bool { true }

        override func hitTest(_ point: NSPoint) -> NSView? {
            guard let event = NSApp.currentEvent, event.type == .rightMouseDown
                || (event.type == .leftMouseDown && event.modifierFlags.contains(.control))
            else { return nil }
            return super.hitTest(point)
        }

        override func rightMouseDown(with event: NSEvent) { report(event) }

        override func mouseDown(with event: NSEvent) { report(event) }

        private func report(_ event: NSEvent) {
            let p = event.locationInWindow
            action(CGPoint(x: p.x, y: (window?.contentView?.bounds.height ?? 0) - p.y))
        }
    }

    func makeNSView(context: Context) -> CatcherView {
        let v = CatcherView()
        v.action = action
        return v
    }

    func updateNSView(_ v: CatcherView, context: Context) { v.action = action }
}
