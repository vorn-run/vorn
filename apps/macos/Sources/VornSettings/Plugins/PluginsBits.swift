import SwiftUI
import VornCore
import VornUI

/// status-tone.ts toneClass and dotClass: how loudly each state reads.
extension StatusTone {
    var text: Color {
        switch self {
        case .live: Theme.ink
        case .blocked: Theme.bronzo
        case .broken: Theme.danger
        case .settled, .idle: Theme.inkFaint
        }
    }

    var dot: Color {
        switch self {
        case .live: Theme.ink
        case .blocked: Theme.bronzo
        case .broken: Theme.danger
        case .settled: Theme.inkFaint
        case .idle: Theme.inkGhost
        }
    }
}

/// A 6pt status dot; a live one breathes.
struct ToneDot: View {
    let tone: StatusTone
    @State private var dim = false

    var body: some View {
        Circle().fill(tone.dot).frame(width: 6, height: 6)
            .opacity(tone == .live && dim ? 0.35 : 1)
            .animation(tone == .live ? .easeInOut(duration: 0.9).repeatForever() : nil, value: dim)
            .onAppear { dim = true }
    }
}

/// StatusLine: a dot and a sentence in the state's tone.
struct ToneLine: View {
    let tone: StatusTone
    let text: String
    var size: CGFloat = 11

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            ToneDot(tone: tone).alignmentGuide(.firstTextBaseline) { $0[.bottom] - 1 }
            Text(text).font(.ui(size)).foregroundStyle(tone.text).fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// ActivityLine.tsx: what a row is doing now, else why it last failed.
struct ActivityLine: View {
    let state: RowState

    var body: some View {
        if let phrase = state.phrase {
            ToneLine(tone: .live, text: phrase)
        } else if let error = state.error {
            ToneLine(tone: .broken, text: error)
        }
    }
}

/// BusyIcon.tsx: a spinner while busy, else the icon.
struct BusyIcon: View {
    let busy: Bool
    var icon: LucideGlyph?
    var size: CGFloat = 12

    var body: some View {
        if busy {
            Spinner(size: size)
        } else if let icon {
            LucideIcon(icon, size: size)
        }
    }
}

/// The small outlined pill the directory puts beside a name.
struct PluginBadge: View {
    let text: String
    var color = Theme.gray500

    var body: some View {
        Text(text).font(.ui(10)).foregroundStyle(color)
            .padding(.horizontal, 5).padding(.vertical, 1)
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusSm).strokeBorder(Theme.white(0.1), lineWidth: 1))
    }
}

/// `px-2.5 py-1 text-[11px] border border-white/[0.1] rounded-sm`: the plugins page's row buttons.
struct PluginButton: View {
    let label: String
    var icon: LucideGlyph?
    var busy = false
    var danger = false
    var strong = false
    var disabled = false
    let action: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                HStack(spacing: 5) {
                    if busy || icon != nil { BusyIcon(busy: busy, icon: icon, size: 11) }
                    Text(label).font(.ui(11)).lineLimit(1)
                }
                .foregroundStyle(color(hover))
                .padding(.horizontal, 10).padding(.vertical, 4)
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusSm)
                    .strokeBorder(Theme.white(strong ? 0.15 : 0.1), lineWidth: 1))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(disabled || busy)
            .opacity(disabled || busy ? 0.5 : 1)
        }
        .fixedSize()
    }

    private func color(_ hover: Bool) -> Color {
        if danger { return hover ? Theme.red400 : Theme.danger }
        if strong { return hover ? .white : Theme.gray200 }
        return hover ? Theme.gray200 : Theme.gray400
    }
}

/// A grey text link (`text-[11px] text-gray-500 hover:text-gray-300`).
struct PluginLink: View {
    let label: String
    var size: CGFloat = 11
    var color = Theme.gray500
    var hoverColor = Theme.gray300
    var disabled = false
    let action: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                Text(label).font(.ui(size)).foregroundStyle(hover && !disabled ? hoverColor : color)
            }
            .buttonStyle(.plain)
            .disabled(disabled)
        }
        .fixedSize()
    }
}

/// The form's primary and cancel buttons (`px-4 py-1.5 text-sm`).
struct FormButtons: View {
    let label: String
    var busy = false
    var disabled = false
    let onSave: () -> Void
    let onCancel: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Hovering { hover in
                Button(action: onSave) {
                    HStack(spacing: 6) {
                        if busy { Spinner(size: 12) }
                        Text(label).font(.ui(13))
                    }
                    .foregroundStyle(.white)
                    .padding(.horizontal, 16).padding(.vertical, 6)
                    .background(RoundedRectangle(cornerRadius: Theme.radiusSm)
                        .fill(Theme.white(hover && !disabled ? 0.15 : 0.1)))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(disabled || busy)
                .opacity(disabled || busy ? 0.5 : 1)
            }
            PluginLink(label: "Cancel", size: 13, action: onCancel).padding(.horizontal, 16)
        }
        .padding(.top, 4)
    }
}

/// A red alert line under a form.
struct FormError: View {
    let text: String

    var body: some View {
        HStack(alignment: .top, spacing: 4) {
            LucideIcon(.circleAlert, size: 12).padding(.top, 1)
            Text(text).font(.ui(11)).fixedSize(horizontal: false, vertical: true)
        }
        .foregroundStyle(Theme.red400)
    }
}

/// A labelled `<select>` of strings: a bordered box that opens a menu.
struct PluginSelect: View {
    let options: [(value: String, label: String)]
    let value: String
    var placeholder = "—"
    var fill = false
    let onChange: (String) -> Void

    var body: some View {
        Menu {
            ForEach(options, id: \.value) { opt in
                Button(opt.label) { onChange(opt.value) }
            }
        } label: {
            HStack {
                Text(options.first { $0.value == value }?.label ?? placeholder).font(.ui(12)).foregroundStyle(Theme.gray200)
                    .lineLimit(1)
                if fill { Spacer(minLength: 4) }
                LucideIcon(.chevronDown, size: 11).foregroundStyle(Theme.gray500)
            }
            .padding(.horizontal, 10).padding(.vertical, 6)
            .frame(maxWidth: fill ? .infinity : nil, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: Theme.radiusSm).fill(Theme.white(0.05)))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusSm).strokeBorder(Theme.white(0.1), lineWidth: 1))
            .contentShape(Rectangle())
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize(horizontal: !fill, vertical: true)
    }
}

/// A form field's label, with the required star and the "encrypted" note (DynamicField.tsx).
struct PluginFieldLabel: View {
    let text: String
    var mono = false
    var required = false
    var secret = false

    var body: some View {
        HStack(spacing: 6) {
            Text(text).font(mono ? .system(size: 11, design: .monospaced) : .ui(12)).foregroundStyle(Theme.gray500)
            if required { Text("*").font(.ui(12)).foregroundStyle(Theme.red400) }
            if secret { Text("· ENCRYPTED").font(.ui(9)).tracking(0.6).foregroundStyle(Theme.gray600) }
        }
        .padding(.bottom, 4)
    }
}

/// A form box (`bg-white/[0.03] border-white/[0.08] rounded-sm`).
extension View {
    func pluginBox(padding: CGFloat = 12) -> some View {
        self.padding(padding)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: Theme.radiusSm).fill(Theme.white(0.03)))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusSm).strokeBorder(Theme.white(0.08), lineWidth: 1))
    }
}

/// The connector glyph for a listing, a group or a preview.
struct ListingIcon: View {
    let id: String
    var icon: ConnectorIconData?
    var packaged = false
    var size: CGFloat = 14
    var color = Theme.gray400

    var body: some View {
        ConnectorIcon(connectorId: id, viewBox: icon?.viewBox, paths: icon?.paths, packaged: packaged, size: size, color: color)
    }
}

extension ConnectorListing {
    var packaged: Bool { source == .catalog || source == .installed || pack != nil }
}
