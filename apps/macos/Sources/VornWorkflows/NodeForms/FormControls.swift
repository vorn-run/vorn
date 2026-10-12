import AppKit
import SwiftUI
import VornCore
import VornUI

// MARK: Popups

/// A dropdown a form field opened, drawn by the panel above everything so a scroll view cannot clip it.
struct FormPopup {
    var anchor: Anchor<CGRect>
    var width: CGFloat?
    var minWidth: CGFloat
    var trailing: Bool
    var content: AnyView
    var dismiss: () -> Void
}

struct FormPopupKey: PreferenceKey {
    static var defaultValue: FormPopup? { nil }

    static func reduce(value: inout FormPopup?, nextValue: () -> FormPopup?) {
        if let next = nextValue() { value = next }
    }
}

extension View {
    /// Shows `content` under this view while `isPresented`, in the nearest `formPopupHost`.
    func formPopup<Content: View>(
        isPresented: Binding<Bool>, width: CGFloat? = nil, minWidth: CGFloat = 180, trailing: Bool = false,
        @ViewBuilder content: @escaping () -> Content
    ) -> some View {
        anchorPreference(key: FormPopupKey.self, value: .bounds) { anchor in
            guard isPresented.wrappedValue else { return nil }
            return FormPopup(
                anchor: anchor, width: width, minWidth: minWidth, trailing: trailing, content: AnyView(content()),
                dismiss: { isPresented.wrappedValue = false })
        }
    }

    /// Draws the open form popup over this view; a click outside it closes it.
    func formPopupHost() -> some View {
        overlayPreferenceValue(FormPopupKey.self) { popup in
            GeometryReader { g in
                if let popup {
                    let r = g[popup.anchor]
                    let w = popup.width ?? max(r.width, popup.minWidth)
                    let x = min(max(0, popup.trailing ? r.maxX - w : r.minX), max(0, g.size.width - w))
                    let below = g.size.height - r.maxY - 4
                    let flip = below < 160 && r.minY > below
                    ZStack(alignment: .topLeading) {
                        Color.black.opacity(0.001)
                            .contentShape(Rectangle())
                            .onTapGesture { popup.dismiss() }
                        Button("", action: popup.dismiss).keyboardShortcut(.cancelAction).opacity(0).frame(width: 0, height: 0)
                        popup.content
                            .frame(width: w, alignment: .leading)
                            .frame(height: flip ? r.minY - 4 : below, alignment: flip ? .bottom : .top)
                            .offset(x: x, y: flip ? 0 : r.maxY + 4)
                    }
                }
            }
        }
    }
}

// MARK: Line height

extension View {
    /// CSS `line-height`: the leading a browser splits above and below each line.
    func cssLineHeight(_ size: CGFloat, _ ratio: CGFloat = 1.5, mono: Bool = false) -> some View {
        let font = mono ? NSFont.monospacedSystemFont(ofSize: size, weight: .regular) : NSFont.systemFont(ofSize: size)
        let extra = max(0, size * ratio - (font.ascender - font.descender + font.leading))
        return lineSpacing(extra).padding(.vertical, extra / 2)
    }
}

// MARK: Labels and fields

/// `text-[13px] text-gray-400 font-medium block mb-2`.
struct FormLabel: View {
    let text: String
    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text)
            .font(.system(size: 13, weight: .medium))
            .foregroundStyle(Theme.gray400)
            .cssLineHeight(13)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.bottom, 8)
    }
}

/// `text-[11px] text-gray-500`, the hint under a field.
struct FormHint: View {
    let text: String
    var top: CGFloat = 4
    var color: Color = Theme.gray500
    init(_ text: String, top: CGFloat = 4, color: Color = Theme.gray500) {
        self.text = text
        self.top = top
        self.color = color
    }

    var body: some View {
        Text(text)
            .font(.system(size: 11))
            .foregroundStyle(color)
            .cssLineHeight(11)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.top, top)
    }
}

/// A labelled block: label, the control, an optional hint.
struct FormSection<Content: View>: View {
    let label: String?
    var hint: String?
    @ViewBuilder let content: Content

    init(_ label: String?, hint: String? = nil, @ViewBuilder content: () -> Content) {
        self.label = label
        self.hint = hint
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let label { FormLabel(label) }
            content
            if let hint { FormHint(hint) }
        }
    }
}

/// The form field box: `px-3 py-2 bg-white/[0.06] border border-white/[0.1] rounded-md`.
struct FieldChrome: ViewModifier {
    var focused = false
    var invalid = false
    var horizontal: CGFloat = 12
    var vertical: CGFloat = 8

    func body(content: Content) -> some View {
        content
            .padding(.horizontal, horizontal)
            .padding(.vertical, vertical)
            .background(Theme.white(0.06), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
            .overlay(
                RoundedRectangle(cornerRadius: Theme.radiusMd)
                    .strokeBorder(
                        invalid ? Theme.danger.opacity(0.5) : Theme.white(focused ? 0.2 : 0.1), lineWidth: 1))
    }
}

extension View {
    func fieldChrome(focused: Bool = false, invalid: Bool = false, horizontal: CGFloat = 12, vertical: CGFloat = 8)
        -> some View
    {
        modifier(FieldChrome(focused: focused, invalid: invalid, horizontal: horizontal, vertical: vertical))
    }
}

/// A bare single-line text input; offscreen renders draw its text instead.
struct BareTextInput: View {
    let placeholder: String
    @Binding var text: String
    var size: CGFloat = 13
    var mono = false
    var color: Color = .white
    @Environment(\.workflowsStaticLayout) private var isStatic

    var body: some View {
        let font = Font.system(size: size, design: mono ? .monospaced : .default)
        if isStatic {
            Text(text.isEmpty ? placeholder : text)
                .font(font)
                .foregroundStyle(text.isEmpty ? Theme.gray600 : color)
                .lineLimit(1)
                .frame(maxWidth: .infinity, minHeight: size * 1.5, maxHeight: size * 1.5, alignment: .leading)
        } else {
            TextField("", text: $text, prompt: Text(placeholder).foregroundStyle(Theme.gray600))
                .textFieldStyle(.plain)
                .font(font)
                .foregroundStyle(color)
                .frame(height: size * 1.5)
        }
    }
}

/// A form text input in the field box.
struct FormTextField: View {
    let placeholder: String
    @Binding var text: String
    var mono = false
    var size: CGFloat = 13
    var invalid = false
    @FocusState private var focused: Bool

    var body: some View {
        BareTextInput(placeholder: placeholder, text: $text, size: size, mono: mono)
            .focused($focused)
            .fieldChrome(focused: focused, invalid: invalid)
    }
}

/// A multi-line text input in the field box, `rows` lines tall at least.
struct FormTextArea: View {
    let placeholder: String
    @Binding var text: String
    var rows = 3
    var mono = false
    var size: CGFloat = 13
    var invalid = false
    @FocusState private var focused: Bool
    @Environment(\.workflowsStaticLayout) private var isStatic

    private var font: Font { .system(size: size, design: mono ? .monospaced : .default) }
    private var minHeight: CGFloat { CGFloat(rows) * (size * 1.5) }

    var body: some View {
        Group {
            if isStatic {
                Text(text.isEmpty ? placeholder : text)
                    .font(font)
                    .foregroundStyle(text.isEmpty ? Theme.gray600 : .white)
                    .lineSpacing(size * 0.3)
                    .frame(maxWidth: .infinity, minHeight: minHeight, alignment: .topLeading)
            } else {
                TextEditor(text: $text)
                    .font(font)
                    .foregroundStyle(.white)
                    .scrollContentBackground(.hidden)
                    .scrollIndicators(.never)
                    .padding(.horizontal, -5)
                    .focused($focused)
                    .frame(minHeight: minHeight)
                    .fixedSize(horizontal: false, vertical: true)
                    .overlay(alignment: .topLeading) {
                        if text.isEmpty {
                            Text(placeholder).font(font).foregroundStyle(Theme.gray600).allowsHitTesting(false)
                        }
                    }
            }
        }
        .fieldChrome(focused: focused, invalid: invalid)
    }
}

// MARK: Select

/// SelectPicker.tsx's option.
struct SelectOption: Identifiable {
    var value: String
    var label: String
    var icon: AnyView?
    var hint: String?

    var id: String { value }

    init(_ value: String, _ label: String, icon: AnyView? = nil, hint: String? = nil) {
        self.value = value
        self.label = label
        self.icon = icon
        self.hint = hint
    }
}

/// The field trigger every picker shares: icon, label or placeholder, chevron.
struct PickerTrigger<Leading: View>: View {
    let text: String?
    let placeholder: String
    var open = false
    var disabled = false
    var chevron = true
    var tone = PickerTone.form
    @ViewBuilder var leading: Leading
    let action: () -> Void

    var body: some View {
        Hovering { hovered in
            Button(action: action) {
                HStack(spacing: 8) {
                    leading
                    Text(text ?? placeholder)
                        .font(.system(size: 13))
                        .foregroundStyle(text == nil || disabled ? Theme.gray600 : tone.text)
                        .lineLimit(1)
                        .frame(maxWidth: .infinity, minHeight: 19.5, alignment: .leading)
                    if chevron {
                        Image(systemName: "chevron.down")
                            .font(.system(size: 9, weight: .semibold))
                            .foregroundStyle(Theme.gray500)
                            .rotationEffect(.degrees(open ? 180 : 0))
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(Theme.white(tone.fill), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                .overlay(
                    RoundedRectangle(cornerRadius: Theme.radiusMd)
                        .strokeBorder(Theme.white(hovered && !disabled ? 0.2 : tone.border), lineWidth: 1))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(disabled)
        }
    }
}

/// The field wash: the form one, or the quieter one the error-policy select uses.
struct PickerTone {
    var fill: Double
    var border: Double
    var text: Color

    static let form = PickerTone(fill: 0.06, border: 0.1, text: .white)
    static let quiet = PickerTone(fill: 0.03, border: 0.08, text: Theme.gray200)
}

/// A dropdown list: surface-overlay, hairline, max 280pt tall.
struct PopupList<Content: View>: View {
    var maxHeight: CGFloat = 280
    @ViewBuilder let content: Content

    var body: some View {
        DropdownPanel(verticalPadding: 4) {
            ScrollView { VStack(alignment: .leading, spacing: 0) { content } }
                .scrollIndicators(.never)
                .frame(maxHeight: maxHeight)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// SelectPicker.tsx's row: a check or its space, the icon, the label, a hint.
struct SelectRow: View {
    let option: SelectOption
    let selected: Bool
    let action: () -> Void

    var body: some View {
        Hovering { hovered in
            Button(action: action) {
                HStack(spacing: 8) {
                    Group {
                        if selected { Icon(.check, size: 11, weight: .bold) } else { Color.clear }
                    }
                    .frame(width: 11, height: 11)
                    if let icon = option.icon { icon }
                    Text(option.label).lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
                    if let hint = option.hint {
                        Text(hint).font(.system(size: 10)).foregroundStyle(Theme.gray600)
                    }
                }
                .font(.system(size: 12))
                .foregroundStyle(selected || hovered ? Color.white : Theme.gray300)
                .frame(minHeight: 18)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .background(selected ? Theme.white(0.06) : hovered ? Theme.white(0.04) : .clear)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }
}

/// SelectPicker.tsx, the form variant.
struct FormSelect: View {
    let value: String
    let options: [SelectOption]
    var placeholder = "Select..."
    var tone = PickerTone.form
    let onChange: (String) -> Void
    @State private var open = false

    init(
        value: String, options: [SelectOption], placeholder: String = "Select...", tone: PickerTone = .form,
        onChange: @escaping (String) -> Void
    ) {
        self.value = value
        self.options = options
        self.placeholder = placeholder
        self.tone = tone
        self.onChange = onChange
    }

    var body: some View {
        let current = options.first { $0.value == value }
        PickerTrigger(text: current?.label.isEmpty == false ? current?.label : nil, placeholder: placeholder, open: open, tone: tone) {
            if let icon = current?.icon { icon }
        } action: {
            open.toggle()
        }
        .formPopup(isPresented: $open) {
            PopupList {
                ForEach(options) { opt in
                    SelectRow(option: opt, selected: opt.value == value) {
                        open = false
                        if opt.value != value { onChange(opt.value) }
                    }
                }
            }
        }
    }
}

/// A Project / Agent picker row: icon, label, an optional hint, a trailing check.
struct CheckTrailingRow<Leading: View>: View {
    let label: String
    var selected = false
    var italic = false
    var muted = false
    var disabled = false
    var hint: String?
    @ViewBuilder var leading: Leading
    let action: () -> Void

    var body: some View {
        Hovering { hovered in
            Button(action: action) {
                HStack(spacing: 10) {
                    leading
                    Text(label)
                        .italic(italic)
                        .lineLimit(1)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    if let hint { Text(hint).font(.system(size: 10)).foregroundStyle(Theme.gray600) }
                    if selected { LucideIcon(.check, size: 13).foregroundStyle(Theme.gray400) }
                }
                .font(.system(size: 12))
                .foregroundStyle(disabled || muted ? (disabled ? Theme.gray600 : Theme.gray500) : Theme.gray300)
                .frame(minHeight: 16)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .background(hovered && !disabled ? Theme.white(0.06) : .clear)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(disabled)
        }
    }
}

// MARK: Toggles

/// The switch row TriggerConfigForm and LaunchAgentConfigForm draw: a small pill, an icon, a title, a line.
struct SwitchCard: View {
    let isOn: Bool
    let symbol: LucideGlyph?
    var sfSymbol: String?
    let title: String
    let detail: String
    var disabled = false
    let action: () -> Void

    var body: some View {
        Hovering { hovered in
            Button(action: action) {
                HStack(alignment: .center, spacing: 12) {
                    ZStack(alignment: isOn ? .trailing : .leading) {
                        Capsule().fill(isOn ? Theme.gray400 : Theme.white(0.1))
                        Circle().fill(Color.white).frame(width: 12, height: 12).padding(.horizontal, 2)
                    }
                    .frame(width: 28, height: 16)
                    VStack(alignment: .leading, spacing: 2) {
                        HStack(spacing: 6) {
                            Group {
                                if let symbol { LucideIcon(symbol, size: 12) } else if let sfSymbol {
                                    Image(systemName: sfSymbol).font(.system(size: 10))
                                }
                            }
                            .foregroundStyle(isOn ? Theme.gray300 : Theme.gray500)
                            Text(title).font(.system(size: 12)).foregroundStyle(isOn ? Theme.gray200 : Theme.gray400)
                                .cssLineHeight(12)
                        }
                        Text(detail)
                            .font(.system(size: 11))
                            .foregroundStyle(Theme.gray500)
                            .cssLineHeight(11)
                            .multilineTextAlignment(.leading)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .background(
                    disabled ? Theme.white(0.01) : isOn ? Theme.white(0.04) : Theme.white(0.02),
                    in: RoundedRectangle(cornerRadius: Theme.radiusLg))
                .overlay(
                    RoundedRectangle(cornerRadius: Theme.radiusLg)
                        .strokeBorder(Theme.white(isOn || (hovered && !disabled) ? 0.1 : 0.04), lineWidth: 1))
                .opacity(disabled ? 0.5 : 1)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(disabled)
        }
    }
}

/// ToggleSwitch.tsx: a 40×24 pill, blue when on.
struct ToggleSwitch: View {
    let isOn: Bool
    var disabled = false
    let onChange: (Bool) -> Void

    var body: some View {
        Button {
            if !disabled { onChange(!isOn) }
        } label: {
            ZStack(alignment: isOn ? .trailing : .leading) {
                Capsule().fill(isOn ? Theme.blue500 : Theme.white(0.1))
                Circle().fill(Color.white).frame(width: 16, height: 16).padding(.horizontal, 4)
            }
            .frame(width: 40, height: 24)
        }
        .buttonStyle(.plain)
        .disabled(disabled)
    }
}

/// TriggerConfigForm's Segmented and LoopConfigForm's mode switch.
struct FormSegmented: View {
    let value: String
    let options: [(value: String, label: String)]
    let onChange: (String) -> Void

    var body: some View {
        HStack(spacing: 0) {
            ForEach(options, id: \.value) { o in
                Hovering { hovered in
                    Button {
                        onChange(o.value)
                    } label: {
                        Text(o.label)
                            .font(.system(size: 12))
                            .foregroundStyle(
                                o.value == value ? Theme.gray200 : hovered ? Theme.gray300 : Theme.gray500)
                            .frame(maxWidth: .infinity, minHeight: 18)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 6)
                            .background(o.value == value ? Theme.white(0.1) : hovered ? Theme.white(0.04) : .clear)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: Theme.radiusLg))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.06), lineWidth: 1))
    }
}

/// The collapsible "Advanced" header with a settings glyph.
struct AdvancedSection<Content: View>: View {
    @Binding var open: Bool
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Hovering { hovered in
                Button {
                    withAnimation(.easeInOut(duration: 0.2)) { open.toggle() }
                } label: {
                    HStack(spacing: 6) {
                        Image(systemName: "chevron.right")
                            .font(.system(size: 9, weight: .semibold))
                            .rotationEffect(.degrees(open ? 90 : 0))
                            .frame(width: 12)
                        LucideIcon(.slidersHorizontal, size: 11)
                        Text("ADVANCED").font(.system(size: 11, weight: .medium)).tracking(0.6)
                        Spacer(minLength: 0)
                    }
                    .foregroundStyle(hovered ? Theme.gray400 : Theme.gray500)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            if open {
                VStack(alignment: .leading, spacing: 16) { content }.padding(.top, 12)
            }
        }
    }
}

/// A rounded group box: `border border-white/[0.06] rounded-lg p-3 space-y-3`.
struct FormBox<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 12) { content }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.06), lineWidth: 1))
    }
}

/// A small text button: `text-[12px] text-gray-400 hover:text-white` with a leading glyph.
struct FormLinkButton: View {
    let symbol: LucideGlyph
    let title: String
    let action: () -> Void

    var body: some View {
        Hovering { hovered in
            Button(action: action) {
                HStack(spacing: 6) {
                    LucideIcon(symbol, size: 12)
                    Text(title).font(.system(size: 12))
                }
                .foregroundStyle(hovered ? Color.white : Theme.gray400)
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(hovered ? Theme.white(0.06) : .clear, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }
}

/// A connector's mark (ConnectorIcon.tsx): GitHub, Linear and MCP drawn, HTTP a globe, else an external link.
struct ConnectorMark: View {
    let connectorId: String
    var size: CGFloat = 14

    var body: some View {
        switch connectorId {
        case "github", "linear", "mcp": ProjectIcon(icon: connectorId, color: "#99A1AF", size: size)
        case "http": LucideIcon(.globe, size: size).foregroundStyle(Theme.gray400)
        default:
            Image(systemName: "arrow.up.right.square")
                .font(.system(size: size * 0.8))
                .frame(width: size, height: size)
                .foregroundStyle(Theme.gray400)
        }
    }
}

/// A lucide glyph where VornUI draws one, an SF Symbol where it does not.
enum FormGlyph {
    case lucide(LucideGlyph)
    case sf(String)
}

struct FormIcon: View {
    let glyph: FormGlyph
    var size: CGFloat = 13

    init(_ glyph: FormGlyph, size: CGFloat = 13) {
        self.glyph = glyph
        self.size = size
    }

    var body: some View {
        switch glyph {
        case .lucide(let g): LucideIcon(g, size: size)
        case .sf(let name):
            Image(systemName: name).font(.system(size: size * 0.82)).frame(width: size, height: size)
        }
    }
}
