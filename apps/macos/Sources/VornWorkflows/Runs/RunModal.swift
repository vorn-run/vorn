import SwiftUI
import VornCore
import VornUI

/// A centred card over a dimmed backdrop; a backdrop click calls `onDismiss`.
struct RunModal<Content: View>: View {
    var backdrop = 0.4
    var onDismiss: (() -> Void)?
    @ViewBuilder let content: Content

    var body: some View {
        ZStack {
            Color.black.opacity(backdrop)
                .contentShape(Rectangle())
                .onTapGesture { onDismiss?() }
            content.shadow(color: .black.opacity(0.5), radius: 25, y: 12)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .ignoresSafeArea()
    }
}

extension View {
    /// The modal card's fill, hairline and corners (`rounded-xl border`), unclipped so dropdowns can spill.
    func modalCard(_ fill: Color, border: Double = 0.08, radius: CGFloat = Theme.radiusXl) -> some View {
        background(fill, in: RoundedRectangle(cornerRadius: radius))
            .overlay(RoundedRectangle(cornerRadius: radius).strokeBorder(Theme.white(border), lineWidth: 1))
    }

    /// INPUT_CLASS: `px-3 py-2 text-[13px] bg-white/[0.06] border-white/[0.1] rounded-md`.
    func formInput() -> some View {
        font(.system(size: 13))
            .foregroundStyle(Color.white)
            .frame(minHeight: 19.5)
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.white(0.06), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.1), lineWidth: 1))
    }
}

/// `<input type="checkbox" class="accent-white/80">` with its label.
struct FormCheckbox: View {
    let checked: Bool
    let label: String
    var labelColor: Color = Theme.gray300
    var labelSize: CGFloat = 13
    let toggle: () -> Void

    var body: some View {
        Button(action: toggle) {
            HStack(spacing: 8) {
                CheckSquare(checked: checked)
                Text(label).font(.system(size: labelSize)).foregroundStyle(labelColor)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
        .accessibilityValue(checked ? "checked" : "unchecked")
    }
}

struct CheckSquare: View {
    let checked: Bool
    var tint: Color = Theme.white(0.8)

    var body: some View {
        // An unstyled checkbox draws light even in a dark page.
        RoundedRectangle(cornerRadius: 2.5)
            .fill(checked ? tint : Color.white)
            .overlay(RoundedRectangle(cornerRadius: 2.5).strokeBorder(checked ? .clear : Color(hex: 0x767676), lineWidth: 1))
            .overlay {
                if checked { LucideIcon(.check, size: 10, strokeWidth: 3).foregroundStyle(tint == Theme.blue500 ? .white : .black) }
            }
            .frame(width: 13, height: 13)
    }
}

/// One choice of a form dropdown.
struct FormOption: Identifiable {
    var value: String
    var label: String
    var project: Project?
    var italic = false

    var id: String { value }
}

/// SelectPicker / ProjectPicker in their `form` variant: a field-styled trigger and a list under it.
struct FormDropdown: View {
    let options: [FormOption]
    let value: String
    let placeholder: String
    var showsProjectIcon = false
    let onSelect: (String) -> Void
    @Binding var open: Bool

    var body: some View {
        let current = options.first { $0.value == value && !$0.value.isEmpty }
        Hovering { hovered in
            Button { open.toggle() } label: {
                HStack(spacing: 8) {
                    if showsProjectIcon {
                        ProjectPickerIcon(icon: current?.project?.icon, color: current?.project?.iconColor)
                    }
                    Text(current?.label ?? placeholder)
                        .foregroundStyle(current == nil ? Theme.gray600 : Color.white)
                        .lineLimit(1)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    LucideIcon(.chevronDown, size: showsProjectIcon ? 11 : 12)
                        .foregroundStyle(Theme.gray500)
                        .rotationEffect(.degrees(open && !showsProjectIcon ? 180 : 0))
                }
                .font(.system(size: 13))
                .frame(minHeight: 19.5)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(Theme.white(0.06), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                .overlay(
                    RoundedRectangle(cornerRadius: Theme.radiusMd)
                        .strokeBorder(Theme.white(hovered ? 0.2 : 0.1), lineWidth: 1))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(current?.label ?? placeholder)
        }
        .overlay(alignment: .topLeading) {
            if open { list.offset(y: 41.5) }
        }
        .zIndex(open ? 10 : 0)
    }

    private var list: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(options) { opt in
                Hovering { hovered in
                    Button {
                        open = false
                        onSelect(opt.value)
                    } label: {
                        row(opt, hovered: hovered)
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .padding(.vertical, 4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .modalCard(Theme.surfaceOverlay, radius: Theme.radiusLg)
        .shadow(color: .black.opacity(0.4), radius: 12, y: 6)
    }

    private func row(_ opt: FormOption, hovered: Bool) -> some View {
        let selected = opt.value == value
        return HStack(spacing: 8) {
            if showsProjectIcon {
                if opt.italic {
                    LucideIcon(.folder, size: 13, strokeWidth: 1.5).foregroundStyle(Theme.gray600)
                } else {
                    ProjectPickerIcon(icon: opt.project?.icon, color: opt.project?.iconColor)
                }
            } else if selected {
                LucideIcon(.check, size: 11, strokeWidth: 3)
            } else {
                Color.clear.frame(width: 11, height: 11)
            }
            Text(opt.label)
                .italic(opt.italic)
                .lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
            if showsProjectIcon && selected { LucideIcon(.check, size: 13).foregroundStyle(Theme.gray400) }
        }
        .font(.system(size: 12))
        .foregroundStyle(
            opt.italic ? Theme.gray500 : (selected && !showsProjectIcon) || hovered ? Color.white : Theme.gray300)
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(
            (selected && !showsProjectIcon) ? Theme.white(0.06) : hovered ? Theme.white(0.04) : .clear)
        .contentShape(Rectangle())
    }
}

extension FormOption {
    /// ProjectPicker's rows: an italic None first when the field may stay empty.
    static func projects(_ projects: [Project], allowNone: Bool) -> [FormOption] {
        (allowNone ? [FormOption(value: "", label: "None", italic: true)] : [])
            + projects.map { FormOption(value: $0.name, label: $0.name, project: $0) }
    }
}
