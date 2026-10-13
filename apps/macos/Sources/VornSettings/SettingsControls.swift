import SwiftUI
import VornCore
import VornUI

/// SettingsPageHeader.tsx: title, description, trailing actions.
struct SettingsPageHeader<Actions: View>: View {
    let title: String
    let description: String
    @ViewBuilder var actions: () -> Actions

    var body: some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(.ui(20, weight: .semibold)).foregroundStyle(.white).lineHeight(28, size: 20)
                Text(description).font(.ui(14)).foregroundStyle(Theme.gray500).lineHeight(20, size: 14)
            }
            Spacer(minLength: 0)
            actions()
        }
        .padding(.bottom, 24)
    }
}

extension SettingsPageHeader where Actions == EmptyView {
    init(title: String, description: String) {
        self.init(title: title, description: description) { EmptyView() }
    }
}

/// SettingRow.tsx: label and description on the left, the control on the right, a hairline under.
struct SettingRow<Control: View>: View {
    let label: String
    let description: String
    var note: String?
    var disabled = false
    @ViewBuilder var control: () -> Control

    var body: some View {
        HStack(alignment: .center, spacing: 0) {
            VStack(alignment: .leading, spacing: 0) {
                Text(label).font(.ui(14, weight: .medium)).foregroundStyle(Theme.gray200).lineHeight(20, size: 14)
                Text(description).font(.ui(12)).foregroundStyle(Theme.gray500).lineHeight(16, size: 12).padding(.top, 2)
                    .fixedSize(horizontal: false, vertical: true)
                if let note {
                    Text(note).font(.ui(12)).foregroundStyle(Theme.gray400).lineHeight(16, size: 12)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.leading, 8)
                        .overlay(alignment: .leading) { Rectangle().fill(Theme.white(0.12)).frame(width: 1) }
                        .padding(.top, 6)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            control().fixedSize()
        }
        .padding(.vertical, 16)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }
        .opacity(disabled ? 0.4 : 1)
    }
}

/// ToggleSwitch.tsx: a 40×24 pill, blue when on.
struct ToggleSwitch: View {
    let checked: Bool
    var disabled = false
    var label: String?
    let onChange: (Bool) -> Void

    var body: some View {
        Button {
            if !disabled { onChange(!checked) }
        } label: {
            ZStack(alignment: .topLeading) {
                Capsule().fill(checked ? Theme.blue500 : Theme.white(0.1))
                Circle().fill(.white).frame(width: 16, height: 16).offset(x: checked ? 20 : 4, y: 4)
            }
            .frame(width: 40, height: 24)
            .animation(.easeOut(duration: 0.15), value: checked)
        }
        .buttonStyle(.plain)
        .disabled(disabled)
        .accessibilityLabel(label ?? "")
        .accessibilityValue(checked ? "on" : "off")
        .accessibilityAddTraits(.isToggle)
    }
}

struct SegmentOption: Identifiable {
    let value: String
    let label: String
    var icon: LucideGlyph?
    var disabled = false
    var title: String?
    var id: String { value }
}

/// SegmentedControl.tsx.
struct SegmentedControl: View {
    let options: [SegmentOption]
    let value: String
    var padding: (h: CGFloat, v: CGFloat) = (12, 4)
    let onChange: (String) -> Void

    var body: some View {
        HStack(spacing: 2) {
            ForEach(options) { opt in
                Hovering { hover in
                    Button {
                        if !opt.disabled { onChange(opt.value) }
                    } label: {
                        HStack(spacing: 6) {
                            if let icon = opt.icon { LucideIcon(icon, size: 14) }
                            Text(opt.label).font(.ui(12))
                        }
                        .foregroundStyle(color(opt, hover))
                        .padding(.horizontal, padding.h)
                        .padding(.vertical, padding.v)
                        .background(RoundedRectangle(cornerRadius: Theme.radiusMd)
                            .fill(value == opt.value && !opt.disabled ? Theme.white(0.1) : .clear))
                        .contentShape(Rectangle())
                        .opacity(opt.disabled ? 0.3 : 1)
                    }
                    .buttonStyle(.plain)
                    .help(opt.title ?? "")
                }
            }
        }
        .padding(2)
        .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.white(0.04)))
    }

    private func color(_ opt: SegmentOption, _ hover: Bool) -> Color {
        if opt.disabled { return Theme.gray600 }
        if value == opt.value { return .white }
        return hover ? Theme.gray300 : Theme.gray500
    }
}

/// The `<select>` the general page uses: a bordered box that opens a menu.
struct SettingsSelect: View {
    let options: [(value: Double, label: String)]
    let value: Double
    var width: CGFloat = 128
    let onChange: (Double) -> Void

    var body: some View {
        Menu {
            ForEach(options, id: \.value) { opt in
                Button(opt.label) { onChange(opt.value) }
            }
        } label: {
            HStack {
                Text(options.first { $0.value == value }?.label ?? "").font(.ui(14)).foregroundStyle(Theme.gray200)
                Spacer(minLength: 4)
                LucideIcon(.chevronDown, size: 12).foregroundStyle(Theme.gray400)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .frame(width: width)
            .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.white(0.04)))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.08), lineWidth: 1))
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
    }
}

/// The small uppercase field label the agents and hosts forms use.
struct FieldLabel: View {
    let text: String
    var bottom: CGFloat = 4
    init(_ text: String, bottom: CGFloat = 4) {
        self.text = text
        self.bottom = bottom
    }
    var body: some View {
        Text(text.uppercased()).font(.ui(10)).tracking(0.5).foregroundStyle(Theme.gray600).lineHeight(15, size: 10).padding(.bottom, bottom)
    }
}

/// `bg-black/30 border-white/[0.06] rounded-md text-xs` inputs.
struct SettingsField: View {
    @Binding var text: String
    var placeholder = ""
    var mono = false
    var secure = false
    var onCommit: () -> Void = {}
    @FocusState private var focused: Bool

    var body: some View {
        ZStack(alignment: .leading) {
            if text.isEmpty {
                Text(placeholder).foregroundStyle(mono ? Theme.gray700 : Theme.gray600).lineLimit(1).allowsHitTesting(false)
            }
            if secure {
                SecureField("", text: $text)
            } else {
                TextField("", text: $text)
            }
        }
        .textFieldStyle(.plain)
        .font(mono ? .system(size: 12, design: .monospaced) : .ui(12))
        .frame(height: 16)
        .foregroundStyle(Theme.gray200)
        .focused($focused)
        .onSubmit(onCommit)
        .onChange(of: focused) { _, now in if !now { onCommit() } }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Color.black.opacity(0.3)))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd)
            .strokeBorder(focused ? Theme.white(0.15) : Theme.hairline, lineWidth: 1))
    }

}

/// A plain text button in the settings' quiet style (`text-xs px-3 py-1.5 bg-white/[0.04] border`).
struct QuietButton: View {
    let label: String
    var icon: LucideGlyph?
    var iconSize: CGFloat = 13
    var bordered = true
    var fontSize: CGFloat = 12
    var radius: CGFloat = Theme.radiusMd
    var fill = Theme.white(0.04)
    var hoverFill = Theme.white(0.08)
    var color = Theme.gray300
    var hoverColor: Color?
    var padding: (h: CGFloat, v: CGFloat) = (12, 6)
    var disabled = false
    let action: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                HStack(spacing: 6) {
                    if let icon { LucideIcon(icon, size: iconSize) }
                    Text(label).font(.ui(fontSize))
                }
                .foregroundStyle(hover && !disabled ? (hoverColor ?? color) : color)
                .padding(.horizontal, padding.h)
                .padding(.vertical, padding.v)
                .background(RoundedRectangle(cornerRadius: radius).fill(hover && !disabled ? hoverFill : fill))
                .overlay { if bordered { RoundedRectangle(cornerRadius: radius).strokeBorder(Theme.white(0.08), lineWidth: 1) } }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(disabled)
            .opacity(disabled ? 0.4 : 1)
        }
    }
}

/// A dashed full-width "Add" button (SSH & Hosts).
struct DashedAddButton: View {
    let label: String
    let icon: LucideGlyph
    var strokeWidth: CGFloat = 1.5
    var disabled = false
    let action: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                HStack(spacing: 8) {
                    LucideIcon(icon, size: 14, strokeWidth: strokeWidth)
                    Text(label).font(.ui(14))
                }
                .foregroundStyle(hover && !disabled ? .white : Theme.gray500)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 10)
                .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(hover && !disabled ? Theme.white(0.02) : .clear))
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg)
                    .strokeBorder(hover && !disabled ? Theme.white(0.15) : Theme.white(0.08), style: StrokeStyle(lineWidth: 1, dash: [4, 3])))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(disabled)
            .opacity(disabled ? 0.4 : 1)
        }
    }
}

/// A sunken card with a hairline border, as the agents and hosts lists draw them.
extension View {
    func sunkenCard(radius: CGFloat = Theme.radiusXl, border: Color = Theme.hairline) -> some View {
        background(RoundedRectangle(cornerRadius: radius).fill(Theme.surfaceSunken))
            .clipShape(RoundedRectangle(cornerRadius: radius))
            .overlay(RoundedRectangle(cornerRadius: radius).strokeBorder(border, lineWidth: 1))
    }
}

/// The uppercase group heading with an icon and a count (SSH & Hosts, Coding Agents).
struct GroupHeading: View {
    let icon: LucideGlyph
    let title: String
    var count: Int?

    var body: some View {
        HStack(spacing: 8) {
            LucideIcon(icon, size: 12).foregroundStyle(Theme.gray600)
            Text(title.uppercased()).font(.ui(12, weight: .medium)).tracking(0.6).foregroundStyle(Theme.gray500)
            if let count { Text("\(count)").font(.ui(10)).foregroundStyle(Theme.gray600) }
        }
        .padding(.bottom, 12)
    }
}

/// A spinner standing in for lucide's `Loader2 animate-spin`.
struct Spinner: View {
    var size: CGFloat = 13
    @State private var turn = false
    var body: some View {
        LucideIcon(.loaderCircle, size: size)
            .rotationEffect(.degrees(turn ? 360 : 0))
            .animation(.linear(duration: 1).repeatForever(autoreverses: false), value: turn)
            .onAppear { turn = true }
    }
}
