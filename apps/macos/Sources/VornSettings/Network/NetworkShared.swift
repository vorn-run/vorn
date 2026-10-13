import AppKit
import SwiftUI
import VornCore
import VornUI

/// The renderer's text buttons on this page: `px-3 py-1.5 text-xs rounded-md`, a hover fill, an optional border.
struct NetButton: View {
    let label: String
    var color = Theme.gray200
    var hoverColor: Color?
    var border: Color?
    var hoverFill = Theme.white(0.06)
    var weight: Font.Weight = .medium
    var padding: (h: CGFloat, v: CGFloat) = (12, 6)
    let action: () -> Void

    /// `text-gray-200 border border-white/[0.08]`: the page's primary actions.
    static func outline(_ label: String, color: Color = Theme.gray200, action: @escaping () -> Void) -> NetButton {
        NetButton(label: label, color: color, border: Theme.white(0.08), action: action)
    }

    /// `text-gray-500 hover:text-white`: Cancel and its kin.
    static func ghost(_ label: String, weight: Font.Weight = .medium, action: @escaping () -> Void) -> NetButton {
        NetButton(label: label, color: Theme.gray500, hoverColor: .white, weight: weight, action: action)
    }

    /// `px-2 py-1 text-xs` links that sit in a line of text.
    static func small(_ label: String, color: Color = Theme.gray400, action: @escaping () -> Void) -> NetButton {
        NetButton(label: label, color: color, hoverColor: .white, weight: .regular, padding: (8, 4), action: action)
    }

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                Text(label).font(.ui(12, weight: weight)).lineHeight(16, size: 12)
                    .foregroundStyle(hover ? (hoverColor ?? color) : color)
                    .padding(.horizontal, padding.h)
                    .padding(.vertical, padding.v)
                    .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(hover ? hoverFill : .clear))
                    .overlay { if let border { RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(border, lineWidth: 1) } }
                    .contentShape(Rectangle())
                    .animation(.easeInOut(duration: 0.15), value: hover)
            }
            .buttonStyle(.plain)
        }
    }
}

/// `text-[10px] text-gray-600 uppercase tracking-wider font-medium`.
struct NetCaption: View {
    let text: String
    var weight: Font.Weight = .medium
    init(_ text: String, weight: Font.Weight = .medium) {
        self.text = text
        self.weight = weight
    }
    var body: some View {
        Text(text.uppercased()).font(.ui(10, weight: weight)).tracking(0.5).foregroundStyle(Theme.gray600).lineHeight(15, size: 10)
    }
}

extension View {
    /// `rounded-lg border border-white/[0.06] bg-white/[0.02] p-4`.
    func netCard() -> some View {
        padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.white(0.02)))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.hairline, lineWidth: 1))
    }

    /// `rounded-lg border bg-surface-raised p-3`: the minted token and the pairing request.
    func netCallout(border: Color) -> some View {
        padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.surfaceRaised))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(border, lineWidth: 1))
    }
}

/// `bg-white/[0.04] border-white/[0.08] rounded-md px-3 py-2 text-sm` inputs, ringed `white/20` on focus.
struct NetInput: View {
    @Binding var text: String
    var placeholder = ""
    var mono = false
    var secure = false
    var autofocus = false
    var onSubmit: () -> Void = {}
    @FocusState private var focused: Bool

    var body: some View {
        ZStack(alignment: .leading) {
            if text.isEmpty {
                Text(placeholder).foregroundStyle(Theme.gray600).lineLimit(1).allowsHitTesting(false)
            }
            if secure {
                SecureField("", text: $text)
            } else {
                TextField("", text: $text)
            }
        }
        .textFieldStyle(.plain)
        .autocorrectionDisabled()
        .font(mono ? .system(size: 14, design: .monospaced) : .ui(14))
        .frame(height: 20)
        .foregroundStyle(Theme.gray200)
        .focused($focused)
        .onSubmit(onSubmit)
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.white(0.04)))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd)
            .strokeBorder(focused ? Theme.white(0.2) : Theme.white(0.08), lineWidth: 1))
        .onAppear { if autofocus { focused = true } }
    }
}

/// shared.tsx `CopyButton`: copies, then reads "Copied" for two seconds.
struct CopyButton: View {
    let text: String
    var label = "Copy"
    @State private var copied = false
    @State private var reset: Task<Void, Never>?

    var body: some View {
        QuietButton(label: copied ? "Copied" : label, bordered: false, fill: Theme.white(0.06), hoverFill: Theme.white(0.1),
                    color: Theme.gray400, hoverColor: .white, padding: (8, 4)) {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)
            copied = true
            reset?.cancel()
            reset = Task {
                try? await Task.sleep(for: .seconds(2))
                if !Task.isCancelled { copied = false }
            }
        }
        .fixedSize()
        .tooltip("Copy to clipboard")
        .padding(.leading, 8)
    }
}

/// shared.tsx `ScannableQRCode`: 212 px, white, `rounded-lg p-2`, a two-module quiet zone.
struct ScannableQRCode: View {
    let url: String

    /// The last code drawn: the countdown redraws the page every second.
    @MainActor private static var last: (url: String, matrix: QRMatrix?)?

    private var matrix: QRMatrix? {
        if let last = Self.last, last.url == url { return last.matrix }
        let made = QRMatrix.make(url, level: "M")
        Self.last = (url, made)
        return made
    }

    var body: some View {
        if let matrix {
            Canvas { ctx, size in
                let cells = CGFloat(matrix.size + 4)
                let cell = size.width / cells
                var path = Path()
                for y in 0..<matrix.size {
                    for x in 0..<matrix.size where matrix[x, y] {
                        path.addRect(CGRect(x: CGFloat(x + 2) * cell, y: CGFloat(y + 2) * cell, width: cell, height: cell))
                    }
                }
                ctx.fill(path, with: .color(.black))
            }
            .frame(width: 196, height: 196)
            .padding(8)
            .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(.white))
            .accessibilityLabel("Pairing code")
            .tooltip("Scan with the Vorn app")
        }
    }
}
