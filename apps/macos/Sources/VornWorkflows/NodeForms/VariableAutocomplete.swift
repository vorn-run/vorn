import AppKit
import SwiftUI
import VornCore
import VornUI

/// VariableAutocomplete.tsx's list logic: the items, the filter, the groups and what typing does.
public enum VarPicker {
    public struct Item: Hashable {
        public var group: String
        public var groupId: String
        public var key: String
        public var label: String
        public var description: String
        public var pattern: String
        public var disabled = false
        public var value: String?
    }

    public struct Group: Hashable {
        public var id: String
        public var name: String
        public var items: [Item]
        public var disabled: Bool
    }

    /// VAR_CATEGORY_GROUPS, in display order.
    static func categoryGroup(_ c: TemplateVariable.Category) -> (group: String, description: String) {
        switch c {
        case .task: ("Task", "")
        case .trigger: ("Trigger", "")
        case .context: ("Context", "Resolved from the launching card or terminal")
        case .inputs: ("Run Inputs", "Entered when the run is started")
        case .loop: ("Loop", "The pass this step is on")
        case .connectorItem: ("Connector Item", "The upstream item that triggered this run")
        }
    }

    public static func items(stepGroups: [StepVariableGroup], contextVars: [TemplateVariable]) -> [Item] {
        var out: [Item] = []
        for g in stepGroups {
            for k in g.keys {
                out.append(Item(
                    group: g.label, groupId: g.slug, key: k.key, label: k.label, description: k.description,
                    pattern: "{{steps.\(g.slug).\(k.key)}}", disabled: g.disabled, value: k.value))
            }
        }
        for c in TemplateVariable.Category.allCases {
            let meta = categoryGroup(c)
            for v in contextVars where v.category == c {
                out.append(Item(
                    group: meta.group, groupId: c.rawValue, key: v.key, label: v.label, description: meta.description,
                    pattern: v.key))
            }
        }
        return out
    }

    public static func filtered(_ items: [Item], _ filter: String) -> [Item] {
        guard !filter.isEmpty else { return items }
        let lf = filter.lowercased()
        return items.filter {
            $0.label.lowercased().contains(lf) || $0.group.lowercased().contains(lf) || $0.key.lowercased().contains(lf)
        }
    }

    /// Consecutive items of one group make a group, as the TSX walks them.
    public static func grouped(_ items: [Item]) -> [Group] {
        var out: [Group] = []
        for item in items {
            if out.last?.id != item.groupId {
                out.append(Group(id: item.groupId, name: item.group, items: [], disabled: item.disabled))
            }
            out[out.count - 1].items.append(item)
        }
        return out
    }

    public enum Typing: Equatable {
        case open(at: Int)
        case filter(String)
        case close
        case none
    }

    /// handleChange: `{{` before the cursor opens the list; while open, the text after it filters until `}}`.
    public static func afterTyping(_ value: String, cursor: Int, isOpen: Bool, triggerPos: Int?) -> Typing {
        let ns = value as NSString
        let cur = min(max(0, cursor), ns.length)
        if ns.substring(to: cur).hasSuffix("{{") { return .open(at: cur - 2) }
        guard isOpen, let start = triggerPos else { return .none }
        let from = min(start + 2, cur)
        let after = ns.substring(with: NSRange(location: from, length: cur - from))
        return after.contains("}}") ? .close : .filter(after)
    }

    /// insertPattern: the pattern replaces start..<end; the cursor lands after it. UTF-16 offsets, like the DOM.
    public static func insert(_ pattern: String, into value: String, start: Int, end: Int) -> (value: String, cursor: Int) {
        let ns = value as NSString
        let s = min(max(0, start), ns.length)
        let e = min(max(s, end), ns.length)
        let next = ns.substring(to: s) + pattern + ns.substring(from: e)
        return (next, s + (pattern as NSString).length)
    }

    /// ranAtLabel: `3:05 PM`.
    static func ranAt(_ iso: String?) -> String? {
        guard let iso, let d = FormHelpers.parseISO(iso) else { return nil }
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "h:mm a"
        return f.string(from: d)
    }
}

/// The open list's state for one field.
@MainActor @Observable
final class TemplateFieldState {
    var open = false
    var filter = "" { didSet { if filter != oldValue { highlight = 0 } } }
    var triggerPos: Int?
    var collapsed: Set<String> = []
    var highlight = 0
    @ObservationIgnored weak var textView: NSTextView?

    func close() {
        open = false
        filter = ""
        triggerPos = nil
    }
}

extension EnvironmentValues {
    /// The connections a step group's glyph is looked up in.
    @Entry var formConnections: [FormConnection] = []
    /// Offscreen renders only: which popup a form opens on its own (`vars:<placeholder prefix>`, `agent`, `model`...).
    @Entry var formRenderOpen: String?
    /// Offscreen renders only: how far the step panel's form is scrolled.
    @Entry var formRenderScroll: Double = 0
}

/// VariableAutocomplete.tsx: a template textarea with the `{{` picker, its toggle and the step preview.
struct VariableAutocomplete: View {
    let value: String
    let onChange: (String) -> Void
    var placeholder = ""
    var rows = 4
    let stepGroups: [StepVariableGroup]
    let contextVars: [TemplateVariable]
    var mono = false
    var minHeight: CGFloat?
    @State private var state = TemplateFieldState()
    @Environment(\.formRenderOpen) private var renderOpen

    private var allItems: [VarPicker.Item] { VarPicker.items(stepGroups: stepGroups, contextVars: contextVars) }
    /// `text-[13px]` wins over the mono variant's `text-[12px]`, so every template field is 13pt.
    private var fontSize: CGFloat { 13 }

    var body: some View {
        let items = allItems
        let has = !items.isEmpty
        let preview = value.contains("{{") ? TemplateVars.previewStepTokens(value, groups: stepGroups) : TemplatePreview()
        VStack(alignment: .leading, spacing: 0) {
            TemplateTextArea(
                text: value, placeholder: placeholder, size: fontSize, mono: mono, rows: rows, minHeight: minHeight,
                state: state, onEdit: edited, onCommand: { command($0, items: items) }
            )
            .overlay(alignment: .topTrailing) { if has { bracesButton } }
            .padding(.bottom, 6)
            .formPopup(
                isPresented: Binding(get: { state.open && has }, set: { if !$0 { state.close() } }), minWidth: 0
            ) {
                VarDropdown(state: state, items: items, stepGroups: stepGroups) { insert($0) }
            }
            if let broken = preview.broken {
                Text("\(broken.token) not found\(broken.suggestion.map { " — did you mean \($0)?" } ?? "")")
                    .font(.system(size: 10.5, design: .monospaced))
                    .foregroundStyle(Theme.danger.opacity(0.9))
                    .lineLimit(1)
                    .cssLineHeight(10.5, mono: true)
                    .padding(.top, 4)
            }
            if let resolved = preview.resolved {
                Text("→ " + String(resolved.replacing(#/\s+/#, with: " ").prefix(160)))
                    .font(.system(size: 10.5, design: .monospaced))
                    .foregroundStyle(Theme.inkFaint)
                    .lineLimit(1)
                    .cssLineHeight(10.5, mono: true)
                    .padding(.top, 4)
            }
        }
        .onAppear {
            if let r = renderOpen, r.hasPrefix("vars:"), !placeholder.isEmpty, placeholder.hasPrefix(r.dropFirst(5)) {
                state.open = true
            }
        }
    }

    private var bracesButton: some View {
        Hovering { hovered in
            Button {
                if state.open {
                    state.open = false
                } else {
                    state.triggerPos = state.textView?.selectedRange().location ?? (value as NSString).length
                    state.open = true
                    state.filter = ""
                }
            } label: {
                LucideIcon(.braces, size: 13)
                    .foregroundStyle(state.open ? Theme.ink : hovered ? Theme.gray300 : Theme.gray500)
                    .padding(4)
                    .background(
                        Theme.white(state.open || hovered ? 0.10 : 0.06), in: RoundedRectangle(cornerRadius: Theme.radius))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Insert variable (or type {{ in the editor)")
        }
        .padding(6)
    }

    private func edited(_ next: String, cursor: Int) {
        onChange(next)
        switch VarPicker.afterTyping(next, cursor: cursor, isOpen: state.open, triggerPos: state.triggerPos) {
        case .open(let at):
            state.triggerPos = at
            state.open = true
            state.filter = ""
        case .filter(let f): state.filter = f
        case .close: state.close()
        case .none: break
        }
    }

    private func visible(_ items: [VarPicker.Item]) -> [VarPicker.Item] {
        VarPicker.filtered(items, state.filter).filter { !state.collapsed.contains($0.groupId) }
    }

    /// handleKeyDown, while the list is open.
    private func command(_ selector: Selector, items: [VarPicker.Item]) -> Bool {
        guard state.open else { return false }
        let shown = visible(items)
        switch selector {
        case #selector(NSResponder.cancelOperation(_:)):
            state.close()
            return true
        case #selector(NSResponder.moveDown(_:)):
            state.highlight = min(state.highlight + 1, shown.count - 1)
            return true
        case #selector(NSResponder.moveUp(_:)):
            state.highlight = max(state.highlight - 1, 0)
            return true
        case #selector(NSResponder.insertNewline(_:)), #selector(NSResponder.insertTab(_:)):
            guard shown.indices.contains(state.highlight), !shown[state.highlight].disabled else { return false }
            insert(shown[state.highlight].pattern)
            return true
        default:
            return false
        }
    }

    private func insert(_ pattern: String) {
        guard let tv = state.textView else {
            onChange(value + pattern)
            state.open = false
            return
        }
        let sel = tv.selectedRange()
        let result = VarPicker.insert(pattern, into: value, start: state.triggerPos ?? sel.location, end: sel.location + sel.length)
        onChange(result.value)
        state.close()
        DispatchQueue.main.async {
            if tv.string != result.value { tv.string = result.value }
            tv.setSelectedRange(NSRange(location: min(result.cursor, (tv.string as NSString).length), length: 0))
            tv.window?.makeFirstResponder(tv)
        }
    }
}

/// The dropdown: a filter box over the groups, `max-h-[280px]`.
private struct VarDropdown: View {
    @Bindable var state: TemplateFieldState
    let items: [VarPicker.Item]
    let stepGroups: [StepVariableGroup]
    let onPick: (String) -> Void

    var body: some View {
        let filtered = VarPicker.filtered(items, state.filter)
        let groups = VarPicker.grouped(filtered)
        let visible = filtered.filter { !state.collapsed.contains($0.groupId) }
        let bySlug = Dictionary(stepGroups.map { ($0.slug, $0) }, uniquingKeysWith: { a, _ in a })
        VStack(alignment: .leading, spacing: 0) {
            BareTextInput(placeholder: "Filter variables...", text: $state.filter, size: 12, color: Theme.gray300)
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .background(Theme.white(0.06), in: RoundedRectangle(cornerRadius: Theme.radius))
                .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.08), lineWidth: 1))
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .overlay(alignment: .bottom) { Hairline(opacity: 0.08) }
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    if groups.isEmpty {
                        Text("No matching variables")
                            .font(.system(size: 12))
                            .foregroundStyle(Theme.gray600)
                            .frame(maxWidth: .infinity)
                            .padding(12)
                    }
                    ForEach(groups, id: \.id) { group in
                        header(group, step: bySlug[group.id])
                        if !state.collapsed.contains(group.id) {
                            ForEach(group.items, id: \.self) { item in
                                VarRow(item: item, highlighted: visible.firstIndex(of: item) == state.highlight) {
                                    if !item.disabled { onPick(item.pattern) }
                                }
                            }
                        }
                    }
                }
            }
            .scrollIndicators(.automatic)
            .frame(maxHeight: 230)
            .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxHeight: 280)
        .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusLg))
        .clipShape(RoundedRectangle(cornerRadius: Theme.radiusLg))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.12), lineWidth: 1))
        .shadow(color: .black.opacity(0.4), radius: 12, y: 8)
    }

    private func toggle(_ id: String) {
        if state.collapsed.contains(id) { state.collapsed.remove(id) } else { state.collapsed.insert(id) }
    }

    @ViewBuilder private func header(_ group: VarPicker.Group, step: StepVariableGroup?) -> some View {
        let collapsed = state.collapsed.contains(group.id)
        Hovering { hovered in
            Button {
                toggle(group.id)
            } label: {
                if let step {
                    HStack(spacing: 8) {
                        LucideIcon(collapsed ? .chevronRight : .chevronDown, size: 11)
                        StepGroupIcon(group: step)
                        Text(group.name)
                            .font(.system(size: 11.5, weight: .semibold))
                            .foregroundStyle(Theme.inkSecondary)
                            .lineLimit(1)
                        if let status = step.runStatus {
                            Circle().fill(StatusDot.color(WorkflowStatusKey(status))).frame(width: 6, height: 6)
                        }
                        Spacer(minLength: 0)
                        if let ranAt = VarPicker.ranAt(step.runCompletedAt) {
                            Text(ranAt).font(.system(size: 9.5, design: .monospaced)).foregroundStyle(Theme.gray600)
                        }
                    }
                    .foregroundStyle(hovered ? Theme.gray300 : Theme.gray400)
                    .frame(minHeight: 17)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(hovered ? Theme.white(0.03) : .clear)
                    .contentShape(Rectangle())
                } else {
                    HStack(spacing: 6) {
                        LucideIcon(collapsed ? .chevronRight : .chevronDown, size: 11)
                        Text(group.name.uppercased()).font(.system(size: 10, weight: .semibold)).tracking(0.5)
                        if group.disabled {
                            Text("(no output)").font(.system(size: 9)).foregroundStyle(Theme.gray600).padding(.leading, 4)
                        }
                        Spacer(minLength: 0)
                    }
                    .foregroundStyle(group.disabled ? Theme.gray600 : hovered ? Theme.gray400 : Theme.gray500)
                    .frame(minHeight: 15)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(hovered && !group.disabled ? Theme.white(0.03) : .clear)
                    .contentShape(Rectangle())
                }
            }
            .buttonStyle(.plain)
        }
    }
}

private struct VarRow: View {
    let item: VarPicker.Item
    let highlighted: Bool
    let action: () -> Void

    var body: some View {
        Hovering { hovered in
            Button(action: action) {
                HStack(spacing: 8) {
                    Text(item.key == item.pattern ? item.label : item.key)
                        .font(.system(size: 12, design: .monospaced))
                        .foregroundStyle(Theme.inkSecondary)
                        .lineLimit(1)
                        .frame(minWidth: 50, alignment: .leading)
                    if let value = item.value {
                        Spacer(minLength: 0)
                        Text(value)
                            .font(.system(size: 10.5, design: .monospaced))
                            .foregroundStyle(Theme.gray500)
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .containerRelativeFrame(.horizontal, alignment: .trailing) { w, _ in w * 0.45 }
                    } else if !item.description.isEmpty {
                        Text(item.description).font(.system(size: 11)).foregroundStyle(Theme.gray600).lineLimit(1)
                        Spacer(minLength: 0)
                    } else {
                        Spacer(minLength: 0)
                    }
                }
                .frame(minHeight: 18)
                .padding(.leading, 24)
                .padding(.trailing, 12)
                .padding(.vertical, 6)
                .background(highlighted && !item.disabled ? Theme.white(0.08) : hovered ? Theme.white(0.04) : .clear)
                .opacity(item.disabled ? 0.4 : 1)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(item.disabled)
        }
    }
}

/// StepGroupIcon: the connector's mark when the step has one, else its node glyph.
private struct StepGroupIcon: View {
    let group: StepVariableGroup
    @Environment(\.formConnections) private var connections

    var body: some View {
        if let id = group.connectionId, let conn = connections.first(where: { $0.id == id }) {
            ConnectorMark(connectorId: conn.realConnectorId, size: 12)
        } else {
            NodeTypeIcon(type: group.nodeType, size: 12, strokeWidth: 2).foregroundStyle(Theme.inkSecondary)
        }
    }
}

/// node-visuals.ts NODE_TYPE_ICON.
struct NodeTypeIcon: View {
    let type: WorkflowNodeType
    var size: CGFloat = 14
    var strokeWidth: CGFloat = 2

    var body: some View {
        LucideIcon(Self.glyph(type), size: size, strokeWidth: strokeWidth)
    }

    static func glyph(_ type: WorkflowNodeType) -> LucideGlyph {
        switch type {
        case .trigger, .callConnectorAction: .zap
        case .launchAgent: .play
        case .script: .terminal
        case .condition: .gitFork
        case .approval: .hand
        case .createTaskFromItem: .listPlus
        case .httpRequest: .globe
        case .loop: .repeatIcon
        default: .zap
        }
    }
}

/// A textarea with `rows` lines of CSS line height; reports edits with the cursor and lets the list take keys.
struct TemplateTextArea: View {
    let text: String
    var placeholder = ""
    var size: CGFloat = 13
    var mono = false
    var rows = 4
    var minHeight: CGFloat?
    let state: TemplateFieldState
    let onEdit: (String, Int) -> Void
    var onCommand: (Selector) -> Bool = { _ in false }
    var invalid = false
    @State private var focused = false
    @Environment(\.workflowsStaticLayout) private var isStatic

    private var lineHeight: CGFloat { size * 1.5 }
    private var height: CGFloat { max(CGFloat(rows) * lineHeight, (minHeight ?? 0) - 18) }

    var body: some View {
        Group {
            if isStatic {
                Text(text.isEmpty ? placeholder : text)
                    .font(.system(size: size, design: mono ? .monospaced : .default))
                    .foregroundStyle(text.isEmpty ? Theme.gray600 : .white)
                    .cssLineHeight(size, mono: mono)
                    .frame(maxWidth: .infinity, alignment: .topLeading)
                    .frame(height: height, alignment: .topLeading)
                    .clipped()
            } else {
                TemplateTextViewRep(
                    text: text, size: size, mono: mono, lineHeight: lineHeight, state: state, focused: $focused,
                    onEdit: onEdit, onCommand: onCommand
                )
                .frame(height: height)
                .overlay(alignment: .topLeading) {
                    if text.isEmpty {
                        Text(placeholder)
                            .font(.system(size: size, design: mono ? .monospaced : .default))
                            .foregroundStyle(Theme.gray600)
                            .cssLineHeight(size, mono: mono)
                            .allowsHitTesting(false)
                    }
                }
            }
        }
        .fieldChrome(focused: focused, invalid: invalid)
    }
}

private struct TemplateTextViewRep: NSViewRepresentable {
    let text: String
    let size: CGFloat
    let mono: Bool
    let lineHeight: CGFloat
    let state: TemplateFieldState
    @Binding var focused: Bool
    let onEdit: (String, Int) -> Void
    let onCommand: (Selector) -> Bool

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    private var font: NSFont {
        mono ? .monospacedSystemFont(ofSize: size, weight: .regular) : .systemFont(ofSize: size)
    }

    private var attributes: [NSAttributedString.Key: Any] {
        let p = NSMutableParagraphStyle()
        p.minimumLineHeight = lineHeight
        p.maximumLineHeight = lineHeight
        let natural = font.ascender - font.descender
        return [
            .font: font, .foregroundColor: NSColor.white, .paragraphStyle: p,
            .baselineOffset: (lineHeight - natural) / 2,
        ]
    }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSTextView.scrollableTextView()
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = false
        scroll.verticalScrollElasticity = .none
        let tv = scroll.documentView as! NSTextView
        tv.delegate = context.coordinator
        tv.drawsBackground = false
        tv.isRichText = false
        tv.allowsUndo = true
        tv.isAutomaticQuoteSubstitutionEnabled = false
        tv.isAutomaticDashSubstitutionEnabled = false
        tv.isAutomaticTextReplacementEnabled = false
        tv.isAutomaticSpellingCorrectionEnabled = false
        tv.isContinuousSpellCheckingEnabled = false
        tv.textContainerInset = .zero
        tv.textContainer?.lineFragmentPadding = 0
        tv.insertionPointColor = .white
        tv.font = font
        tv.typingAttributes = attributes
        tv.string = text
        tv.textStorage?.setAttributes(attributes, range: NSRange(location: 0, length: (text as NSString).length))
        state.textView = tv
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        context.coordinator.parent = self
        guard let tv = scroll.documentView as? NSTextView else { return }
        state.textView = tv
        if tv.string != text {
            let sel = tv.selectedRange()
            tv.string = text
            tv.textStorage?.setAttributes(attributes, range: NSRange(location: 0, length: (text as NSString).length))
            let len = (text as NSString).length
            tv.setSelectedRange(NSRange(location: min(sel.location, len), length: 0))
        }
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: TemplateTextViewRep
        init(_ parent: TemplateTextViewRep) { self.parent = parent }

        func textDidChange(_ note: Notification) {
            guard let tv = note.object as? NSTextView else { return }
            parent.onEdit(tv.string, tv.selectedRange().location)
        }

        func textView(_ textView: NSTextView, doCommandBy selector: Selector) -> Bool {
            parent.onCommand(selector)
        }

        func textDidBeginEditing(_ notification: Notification) { parent.focused = true }
        func textDidEndEditing(_ notification: Notification) { parent.focused = false }
    }
}
