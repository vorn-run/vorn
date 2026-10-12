#if os(macOS)
import AppKit
import SwiftUI

/// RichMarkdownEditor.tsx: a markdown description edited as formatted text, with "/" commands and a selection toolbar.
public struct RichMarkdownEditor: View {
    @Binding var value: String
    var placeholder: String
    @State private var focused = false

    public init(value: Binding<String>, placeholder: String = "Start writing, or type / for commands...") {
        _value = value
        self.placeholder = placeholder
    }

    public var body: some View {
        RichEditorRepresentable(value: $value, placeholder: placeholder, focused: $focused)
            .padding(1)
            .background(Color.white.opacity(0.03), in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Color.white.opacity(focused ? 0.15 : 0.06), lineWidth: 1))
            .clipShape(RoundedRectangle(cornerRadius: 8))
    }
}

struct RichEditorRepresentable: NSViewRepresentable {
    @Binding var value: String
    let placeholder: String
    @Binding var focused: Bool

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> RichTextView {
        let view = RichTextView.make()
        view.placeholder = placeholder
        view.load(value)
        let coordinator = context.coordinator
        coordinator.view = view
        view.onMarkdown = { md in
            coordinator.lastValue = md
            DispatchQueue.main.async { coordinator.setValue?(md) }
        }
        view.onFocus = { f in DispatchQueue.main.async { coordinator.setFocused?(f) } }
        view.onFloating = { [weak coordinator] f in coordinator?.show(f) }
        coordinator.lastValue = value
        return view
    }

    func updateNSView(_ view: RichTextView, context: Context) {
        context.coordinator.setValue = { value = $0 }
        context.coordinator.setFocused = { focused = $0 }
        view.placeholder = placeholder
        // Sync external value changes, as the editor does when the task it shows changes.
        if value != context.coordinator.lastValue {
            context.coordinator.lastValue = value
            view.load(value)
        }
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: RichTextView, context: Context) -> CGSize? {
        let width = proposal.width ?? 300
        return CGSize(width: width, height: nsView.height(forWidth: width))
    }

    static func dismantleNSView(_ view: RichTextView, coordinator: Coordinator) {
        coordinator.show(nil)
    }

    @MainActor
    final class Coordinator {
        weak var view: RichTextView?
        var lastValue = ""
        var setValue: ((String) -> Void)?
        var setFocused: ((Bool) -> Void)?
        private var host: NSHostingView<AnyView>?

        /// Floats the slash menu or toolbar over the window, so no scroll view clips it.
        func show(_ floating: RichFloating?) {
            guard let view, let floating, let content = view.window?.contentView else {
                host?.removeFromSuperview()
                host = nil
                return
            }
            let rootView: AnyView
            switch floating.kind {
            case .slash(_, let items, let selected):
                rootView = AnyView(SlashCommandMenu(items: items, selected: selected) { [weak view] item in view?.runSlash(item) }
                    .environment(\.colorScheme, .dark))
            case .toolbar:
                rootView = AnyView(FloatingToolbar(view: view).id(view.linkEditing).environment(\.colorScheme, .dark))
            }
            let hosting = host ?? NSHostingView(rootView: AnyView(EmptyView()))
            hosting.rootView = rootView
            if hosting.superview !== content {
                hosting.removeFromSuperview()
                content.addSubview(hosting, positioned: .above, relativeTo: nil)
            }
            host = hosting
            let size = hosting.fittingSize
            let anchor = view.convert(floating.anchor, to: content)
            var origin: CGPoint
            switch floating.kind {
            case .slash:
                // left: caret, top: caret bottom + 6.
                let below = content.isFlipped ? anchor.maxY + 6 : anchor.minY - 6 - size.height
                origin = CGPoint(x: anchor.minX, y: below)
            case .toolbar:
                // translate(-50%, -100%) from 8px above the selection.
                let above = content.isFlipped ? anchor.minY - 8 - size.height : anchor.maxY + 8
                origin = CGPoint(x: anchor.minX - size.width / 2, y: above)
            }
            hosting.frame = CGRect(origin: origin, size: size)
        }
    }
}

/// SlashCommandMenuPortal: the commands matching what follows "/".
struct SlashCommandMenu: View {
    let items: [RichSlashCommand]
    let selected: Int
    let run: (RichSlashCommand) -> Void

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView(.vertical) {
                VStack(spacing: 0) {
                    ForEach(Array(items.enumerated()), id: \.element.id) { i, item in
                        SlashRow(item: item, selected: i == selected) { run(item) }.id(i)
                    }
                }
                .padding(.vertical, 4)
            }
            .scrollIndicators(.never)
            .onChange(of: selected) { _, s in proxy.scrollTo(s) }
        }
        .frame(minWidth: 220)
        .frame(maxHeight: min(280, CGFloat(items.count) * 48 + 8))
        .fixedSize(horizontal: true, vertical: false)
        .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Theme.white(0.08), lineWidth: 1))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }
}

private struct SlashRow: View {
    let item: RichSlashCommand
    let selected: Bool
    let action: () -> Void
    @State private var hovered = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                LucideIcon(RichEditorGlyphs.glyph(item.glyph), size: 15, strokeWidth: 1.8)
                    .frame(width: 28, height: 28)
                    .background(Theme.white(selected ? 0.08 : 0.04), in: RoundedRectangle(cornerRadius: 6))
                VStack(alignment: .leading, spacing: 0) {
                    Text(item.title).font(.system(size: 14, weight: .medium)).lineLimit(1)
                    Text(item.description).font(.system(size: 11)).foregroundStyle(Theme.gray500).lineLimit(1)
                }
                Spacer(minLength: 0)
            }
            .foregroundStyle(selected || hovered ? Color.white : Theme.gray300)
            .padding(.horizontal, 12).padding(.vertical, 8)
            .background(selected ? Theme.white(0.08) : hovered ? Theme.white(0.04) : .clear)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovered = $0 }
    }
}

/// FloatingToolbar: bold, italic, strikethrough, code and link over a selection.
struct FloatingToolbar: View {
    let view: RichTextView
    @State private var linkInput = ""
    @State private var editingLink: Bool
    @FocusState private var inputFocused: Bool

    init(view: RichTextView) {
        self.view = view
        _editingLink = State(initialValue: view.linkEditing)
        _linkInput = State(initialValue: view.linkEditing ? view.currentLink ?? "" : "")
    }

    var body: some View {
        HStack(spacing: 2) {
            if editingLink {
                HStack(spacing: 4) {
                    TextField("", text: $linkInput, prompt: Text("Paste link...").foregroundColor(Theme.gray600))
                        .textFieldStyle(.plain)
                        .font(.system(size: 12))
                        .foregroundStyle(Theme.gray200)
                        .focused($inputFocused)
                        .onSubmit(apply)
                        .onExitCommand(perform: cancel)
                        .padding(.horizontal, 8).padding(.vertical, 4)
                        .frame(width: 180)
                        .background(Theme.white(0.04), in: RoundedRectangle(cornerRadius: 4))
                        .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(Theme.white(inputFocused ? 0.15 : 0.08), lineWidth: 1))
                    ToolbarButton(glyph: .link, title: "Apply", active: false, action: apply)
                }
                .padding(.horizontal, 6).padding(.vertical, 4)
                .onAppear { inputFocused = true }
            } else {
                ToolbarButton(glyph: .bold, title: "Bold", active: view.isMarkActive(.bold)) { view.toggleMark(.bold) }
                ToolbarButton(glyph: .italic, title: "Italic", active: view.isMarkActive(.italic)) { view.toggleMark(.italic) }
                ToolbarButton(glyph: .strikethrough, title: "Strikethrough", active: view.isMarkActive(.strike)) { view.toggleMark(.strike) }
                ToolbarButton(glyph: .code, title: "Code", active: view.isMarkActive(.code)) { view.toggleMark(.code) }
                Rectangle().fill(Theme.white(0.08)).frame(width: 1, height: 16).padding(.horizontal, 2)
                let linked = view.isLinkActive
                ToolbarButton(glyph: linked ? .unlink : .link, title: linked ? "Remove link" : "Add link", active: linked) {
                    if linked {
                        view.setLink(nil)
                    } else {
                        view.linkEditing = true
                        view.refreshFloating()
                    }
                }
            }
        }
        .padding(2)
        .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Theme.white(0.1), lineWidth: 1))
    }

    private func apply() {
        let trimmed = linkInput.trimmingCharacters(in: .whitespaces)
        if !trimmed.isEmpty {
            let href = linkInput.range(of: "^https?://", options: .regularExpression) != nil ? linkInput : "https://\(linkInput)"
            view.window?.makeFirstResponder(view)
            view.setLink(href)
        }
        finish()
    }

    private func cancel() {
        finish()
        view.window?.makeFirstResponder(view)
    }

    private func finish() {
        view.linkEditing = false
        view.refreshFloating()
    }
}

private struct ToolbarButton: View {
    let glyph: LucideGlyph
    let title: String
    let active: Bool
    let action: () -> Void
    @State private var hovered = false

    var body: some View {
        Button(action: action) {
            LucideIcon(glyph, size: 14, strokeWidth: 2)
                .foregroundStyle(active || hovered ? Color.white : Theme.gray400)
                .padding(6)
                .background(active ? Theme.white(0.12) : hovered ? Theme.white(0.06) : .clear, in: RoundedRectangle(cornerRadius: 4))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(title)
        .onHover { hovered = $0 }
    }
}

/// Drives the first editor under a view into a state, for offscreen renders: "slash", "toolbar" or "link".
public enum RichEditorDriver {
    @MainActor public static func drive(_ root: NSView, _ state: String) {
        guard let view = find(root) else { return }
        view.window?.makeFirstResponder(view)
        switch state {
        case "slash":
            view.setSelectedRange(NSRange(location: max(0, (view.textStorage?.length ?? 1) - 1), length: 0))
            view.insertText(" /", replacementRange: view.selectedRange())
            view.refreshFloating()
        case "toolbar", "link":
            view.setSelectedRange(NSRange(location: 12, length: 10))
            if state == "link" { view.linkEditing = true }
            view.refreshFloating()
        default: break
        }
    }

    @MainActor private static func find(_ v: NSView) -> RichTextView? {
        if let r = v as? RichTextView { return r }
        for sub in v.subviews { if let r = find(sub) { return r } }
        return nil
    }
}

enum RichEditorGlyphs {
    static func glyph(_ name: String) -> LucideGlyph {
        switch name {
        case "heading1": .heading1
        case "heading2": .heading2
        case "heading3": .heading3
        case "list": .list
        case "listOrdered": .listOrdered
        case "squareCheckBig": .squareCheckBig
        case "code": .code
        case "quote": .quote
        default: .minus
        }
    }
}
#endif
