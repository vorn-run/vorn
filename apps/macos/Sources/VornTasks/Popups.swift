import SwiftUI
import VornCore
import VornUI

/// A menu or panel floating over the board, hung from the control that opened it.
struct Popup {
    enum Align { case leading, trailing }

    let id: String
    /// The opener's frame in global coordinates.
    let anchor: CGRect
    var align: Align = .leading
    let content: AnyView
}

extension TasksStore {
    /// Opens `content` under `anchor`, or closes it if that same popup is already open.
    func toggle<V: View>(_ id: String, anchor: CGRect, align: Popup.Align = .leading, @ViewBuilder _ content: () -> V) {
        if popup?.id == id {
            popup = nil
        } else {
            popup = Popup(id: id, anchor: anchor, align: align, content: AnyView(content()))
        }
    }

    func closePopup() { popup = nil }
}

/// Draws the open popup over the board; a click anywhere else closes it.
struct PopupLayer: View {
    @Bindable var store: TasksStore

    var body: some View {
        GeometryReader { geo in
            if let popup = store.popup {
                let root = geo.frame(in: .global)
                ZStack(alignment: .topLeading) {
                    Color.black.opacity(0.0001)
                        .contentShape(Rectangle())
                        .onTapGesture { store.closePopup() }
                    popup.content
                        .fixedSize()
                        .alignmentGuide(.leading) { d in
                            let x = popup.align == .leading
                                ? popup.anchor.minX - root.minX
                                : popup.anchor.maxX - root.minX - d.width
                            return -min(max(8, x), max(8, root.width - d.width - 8))
                        }
                        .alignmentGuide(.top) { d in
                            let below = popup.anchor.maxY - root.minY + 4
                            let fits = below + d.height <= root.height - 8
                            return -(fits ? below : max(8, popup.anchor.minY - root.minY - 4 - d.height))
                        }
                        .transition(.opacity.combined(with: .scale(scale: 0.96, anchor: .top)))
                        .id(popup.id)
                }
                .frame(width: geo.size.width, height: geo.size.height, alignment: .topLeading)
                .background {
                    Button("") { store.closePopup() }
                        .keyboardShortcut(.cancelAction)
                        .opacity(0)
                }
            }
        }
    }
}

/// The surface every dropdown here sits on: overlay, hairline, rounded-lg, py-1.
struct MenuSurface<Content: View>: View {
    var minWidth: CGFloat = 180
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) { content }
            .padding(.vertical, 4)
            .frame(minWidth: minWidth, alignment: .leading)
            .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusLg))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.1)))
            .floatingShadow()
    }
}

/// `w-full flex items-center gap-2.5 px-3 py-1.5 text-xs hover:bg-white/[0.06]`.
struct MenuRow<Leading: View>: View {
    let label: String
    var textColor: Color = Theme.inkSecondary
    var italic = false
    var checked = false
    var trailingNote: String?
    var disabled = false
    let leading: Leading
    let action: () -> Void
    @State private var hovered = false

    init(_ label: String, textColor: Color = Theme.inkSecondary, italic: Bool = false, checked: Bool = false,
         trailingNote: String? = nil, disabled: Bool = false,
         @ViewBuilder leading: () -> Leading, action: @escaping () -> Void) {
        self.label = label
        self.textColor = textColor
        self.italic = italic
        self.checked = checked
        self.trailingNote = trailingNote
        self.disabled = disabled
        self.leading = leading()
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                leading
                Text(label).italic(italic).frame(maxWidth: .infinity, alignment: .leading)
                if let trailingNote {
                    Text(trailingNote).font(.system(size: 10)).foregroundStyle(Theme.gray600)
                }
                if checked {
                    LucideIcon(.check, size: 13).foregroundStyle(Theme.gray400)
                }
            }
            .font(.system(size: 12))
            .foregroundStyle(textColor)
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(hovered && !disabled ? Theme.white(0.06) : .clear)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(disabled)
        .onHover { hovered = $0 }
    }
}

struct MenuSeparator: View {
    var body: some View {
        Rectangle().fill(Theme.white(0.06)).frame(height: 1).padding(.vertical, 4)
    }
}

/// A bare glyph button: `p-1 text-ink-faint hover:text-… rounded`.
struct GlyphButton: View {
    let glyph: LucideGlyph
    var size: CGFloat = 14
    var strokeWidth: CGFloat = 2
    var padding: CGFloat = 4
    var color: Color = Theme.inkFaint
    var hoverColor: Color = Theme.inkSecondary
    var help: String?
    let action: () -> Void
    @State private var hovered = false

    var body: some View {
        Button(action: action) {
            LucideIcon(glyph, size: size, strokeWidth: strokeWidth)
                .foregroundStyle(hovered ? hoverColor : color)
                .padding(padding)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovered = $0 }
        .help(help ?? "")
    }
}

/// The two-button delete confirmation of ConfirmPopover.
struct ConfirmDelete: View {
    let message: String
    let onCancel: () -> Void
    let onConfirm: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(message).font(.system(size: 12)).foregroundStyle(Theme.gray300)
            HStack(spacing: 8) {
                Spacer(minLength: 0)
                Button(action: onCancel) {
                    Text("Cancel").font(.system(size: 12)).foregroundStyle(Theme.gray400)
                        .padding(.horizontal, 12).padding(.vertical, 8)
                        .background(Theme.white(0.04), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                }
                .buttonStyle(.plain)
                Button(action: onConfirm) {
                    Text("Delete").font(.system(size: 12, weight: .medium)).foregroundStyle(Theme.red400)
                        .padding(.horizontal, 12).padding(.vertical, 8)
                        .background(Theme.red500.opacity(0.1), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.red500.opacity(0.2)))
                }
                .buttonStyle(.plain)
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(12)
        .frame(minWidth: 180, maxWidth: 280)
        .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusLg))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.1)))
        .floatingShadow()
    }
}

/// `ToggleSwitch`: 40×24, blue when on, a 16pt knob.
struct ToggleSwitch: View {
    @Binding var isOn: Bool

    var body: some View {
        Button { isOn.toggle() } label: {
            ZStack(alignment: .leading) {
                Capsule().fill(isOn ? Theme.blue500 : Theme.white(0.1))
                Circle().fill(.white).frame(width: 16, height: 16).offset(x: isOn ? 20 : 4)
            }
            .frame(width: 40, height: 24)
            .animation(.easeOut(duration: 0.15), value: isOn)
        }
        .buttonStyle(.plain)
    }
}

extension View {
    /// Keeps `frame` at this view's global frame, for popups to hang from.
    func globalFrame(_ frame: Binding<CGRect>) -> some View {
        onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { frame.wrappedValue = $0 }
    }
}
