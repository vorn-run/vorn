import SwiftUI
import VornCore
import VornUI

private let railHeading = Font.system(size: 11, weight: .semibold, design: .monospaced)
private let mono11 = Font.system(size: 11, design: .monospaced)

/// The rail's uppercase heading row.
struct RailHeading: View {
    let title: String
    var count: Int?

    var body: some View {
        HStack {
            Text(title.uppercased()).lineLimit(1)
            Spacer(minLength: 0)
            if let count { Text("\(count)") }
        }
        .font(railHeading).tracking(0.55)
        .foregroundStyle(Theme.inkFaint)
        .padding(.horizontal, 12).padding(.vertical, 8)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.white(0.04)).frame(height: 1) }
    }
}

/// A 260-wide aside beside the page.
private struct Aside<Content: View>: View {
    var width: CGFloat = 260
    @ViewBuilder let content: Content

    var body: some View {
        VStack(spacing: 0) { content }
            .frame(width: width)
            .frame(maxHeight: .infinity)
            .background(Theme.surfacePanel)
            .overlay(alignment: .leading) { Rectangle().fill(Theme.hairline).frame(width: 1) }
    }
}

/// The rail's primary button: solid ink.
struct SolidButton: View {
    let label: String
    var disabled = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(label).font(.ui(12, weight: .semibold)).foregroundStyle(Theme.surfaceBase).lineLimit(1)
                .frame(maxWidth: .infinity).frame(height: 28)
                .background(RoundedRectangle(cornerRadius: Theme.radius).fill(Theme.ink))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(disabled)
        .opacity(disabled ? 0.4 : 1)
    }
}

/// A small bordered text area, as the rail writes comments.
struct RailField: View {
    @Binding var text: String
    var placeholder = ""
    var rows: CGFloat = 2
    var label: String
    var onCommit: () -> Void = {}
    var onCancel: (() -> Void)?
    @FocusState private var focused: Bool

    var body: some View {
        ZStack(alignment: .topLeading) {
            if text.isEmpty {
                Text(placeholder).font(.ui(12)).foregroundStyle(Theme.inkFaint).padding(.horizontal, 8).padding(.vertical, 6)
                    .allowsHitTesting(false)
            }
            TextEditor(text: $text)
                .font(.ui(12)).foregroundStyle(Theme.ink)
                .scrollContentBackground(.hidden)
                .focused($focused)
                .padding(.horizontal, 3).padding(.vertical, 6)
                .onKeyPress(.return, phases: .down) { press in
                    guard press.modifiers.contains(.command) else { return .ignored }
                    onCommit()
                    return .handled
                }
                .onExitCommand { onCancel?() }
                .accessibilityLabel(label)
        }
        .frame(height: rows * 16 + 12)
        .background(RoundedRectangle(cornerRadius: Theme.radius).fill(Theme.surfaceSunken))
        .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(focused ? 0.24 : 0.12)))
    }
}

/// Where a comment points, as the rail quotes it.
private struct Target: View {
    let comment: ArtifactComment

    var body: some View {
        Text(comment.target)
            .font(.system(size: 11.5, design: .monospaced)).foregroundStyle(Theme.inkFaint).lineLimit(2)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.leading, 8)
            .overlay(alignment: .leading) { Rectangle().fill(Theme.statusBlue).frame(width: 2) }
    }
}

private struct LinkText: View {
    let label: String
    var hoverColor: Color = Theme.ink
    let action: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: action) { Text(label).foregroundStyle(hover ? hoverColor : Theme.inkFaint) }.buttonStyle(.plain)
        }
    }
}

/// ArtifactRail.tsx: the batch being gathered for the agent, and what became of the last one.
struct ArtifactRail: View {
    let model: BrowserModel
    let s: String
    let version: Int
    let agent: String
    let queued: Bool
    let sending: Bool
    let found: [String: Bool]
    let railEdit: String?
    var hint: String?
    @State private var note = ""
    @State private var editText = ""

    var body: some View {
        let (drafts, sent) = model.comments(s)
        let sentOn = sent.map(\.version).max() ?? 0
        Aside {
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    RailHeading(title: "To send", count: drafts.count)
                    if drafts.isEmpty {
                        Text(hint ?? "Select words on the page to comment on them.").font(.ui(12)).foregroundStyle(Theme.inkFaint)
                            .padding(12)
                    } else {
                        ForEach(drafts) { c in draft(c) }
                    }
                    if !sent.isEmpty {
                        RailHeading(title: "Sent with v\(sentOn)")
                        ForEach(sent) { c in sentRow(c) }
                    }
                }
            }
            VStack(spacing: 8) {
                RailField(text: $note, placeholder: "A note about the whole version", label: "Note about the whole version",
                          onCommit: addNote)
                if !note.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    BarButton(label: "Add note", action: addNote).frame(maxWidth: .infinity)
                }
                SolidButton(label: drafts.isEmpty ? "Nothing to send yet"
                    : "Send \(drafts.count) comment\(drafts.count == 1 ? "" : "s") to \(agent)",
                    disabled: drafts.isEmpty || sending) { model.send(s) }
                if queued {
                    Text("Queued. It goes once \(agent) is back at its prompt.").font(.ui(11.5)).foregroundStyle(Theme.inkFaint)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding(12)
            .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
        }
    }

    private func addNote() {
        model.addNote(s, note)
        note = ""
    }

    private func draft(_ c: ArtifactComment) -> some View {
        let missing: Bool = if case .quote = c.anchor { found[c.id] == false } else { false }
        return VStack(alignment: .leading, spacing: 6) {
            Button { model.reveal(s, c.id) } label: { Target(comment: c) }.buttonStyle(.plain).accessibilityLabel("Show on the page")
            if railEdit == c.id {
                RailField(text: $editText, rows: 3, label: "Edit comment",
                          onCommit: { model.railSave(s, editText) }, onCancel: { model.railEdit(s, nil) })
            } else {
                Text(c.body).font(.ui(12)).foregroundStyle(Theme.ink).fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: 8) {
                if missing { Text("words changed").foregroundStyle(Theme.danger) }
                if railEdit == c.id {
                    LinkText(label: "save") { model.railSave(s, editText) }
                    LinkText(label: "cancel") { model.railEdit(s, nil) }
                } else {
                    LinkText(label: "edit") {
                        editText = c.body
                        model.railEdit(s, c.id)
                    }
                    LinkText(label: "delete", hoverColor: Theme.danger) { model.railDelete(s, c.id) }
                }
            }
            .font(mono11).foregroundStyle(Theme.inkFaint)
        }
        .padding(.horizontal, 12).padding(.vertical, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.white(0.04)).frame(height: 1) }
    }

    private func sentRow(_ c: ArtifactComment) -> some View {
        let moved = version > c.version
        let lost: Bool = if case .quote = c.anchor { found[c.id] == false } else { false }
        return VStack(alignment: .leading, spacing: 6) {
            Button { model.reveal(s, c.id) } label: { Target(comment: c) }.buttonStyle(.plain).accessibilityLabel("Show on the page")
            Text(c.body).font(.ui(12)).foregroundStyle(Theme.inkSecondary).fixedSize(horizontal: false, vertical: true)
            Text(!moved ? "sent" : lost ? "words changed in v\(version)" : "still anchored")
                .font(mono11).foregroundStyle(lost && moved ? Theme.danger : Theme.inkFaint)
        }
        .padding(.horizontal, 12).padding(.vertical, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.white(0.04)).frame(height: 1) }
    }
}

/// DocEditBar: which version the edit starts from, and the way out.
struct DocEditBar: View {
    let version: Int
    let onDiscard: () -> Void

    var body: some View {
        Text("v\(version)").font(mono11).foregroundStyle(Theme.inkSecondary)
            .padding(.horizontal, 8).frame(height: 24)
            .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.08)))
        Text("doc \u{b7} you are editing").font(mono11).foregroundStyle(Theme.inkFaint).lineLimit(1).padding(.leading, 6)
        Spacer(minLength: 0)
        TextAction(label: "Discard", color: Theme.inkSecondary, action: onDiscard)
    }
}

/// DocEditor.tsx: a doc open for editing, with the edits it will send listed beside it.
struct DocEditor: View {
    let draft: DocDraft
    let drafts: Int
    let next: Int
    let agent: String
    let saving: Bool
    let onChange: (String) -> Void
    let onSave: (Bool) -> Void

    var body: some View {
        let edits = Artifacts.mergeDocEdit(original: draft.original, edited: draft.edited).edits
        let total = edits.count + drafts
        HStack(spacing: 0) {
            RichMarkdownEditor(value: Binding(get: { draft.edited }, set: onChange), placeholder: "")
                .padding(16)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            Aside {
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        RailHeading(title: "To send", count: total)
                        if edits.isEmpty {
                            Text("Change the words; each changed paragraph goes as its own edit.").font(.ui(12))
                                .foregroundStyle(Theme.inkFaint).padding(12)
                        } else {
                            ForEach(Array(edits.enumerated()), id: \.offset) { _, e in
                                VStack(alignment: .leading, spacing: 4) {
                                    Text("Your edit").font(mono11).foregroundStyle(Theme.inkFaint)
                                    if !e.before.isEmpty { Text(e.before).strikethrough().foregroundStyle(Theme.inkFaint).lineLimit(3) }
                                    if !e.after.isEmpty { Text(e.after).foregroundStyle(Theme.ink).lineLimit(3) }
                                }
                                .font(.ui(12))
                                .padding(.horizontal, 12).padding(.vertical, 10)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .overlay(alignment: .bottom) { Rectangle().fill(Theme.white(0.04)).frame(height: 1) }
                            }
                        }
                        if drafts > 0 {
                            Text("And \(drafts) comment\(drafts == 1 ? "" : "s") already waiting.").font(.ui(12))
                                .foregroundStyle(Theme.inkFaint).padding(.horizontal, 12).padding(.vertical, 10)
                        }
                    }
                }
                VStack(spacing: 8) {
                    SolidButton(label: "Save v\(next) and send \(total) to \(agent)", disabled: edits.isEmpty || saving) { onSave(true) }
                    TextAction(label: "Save v\(next) without sending", color: Theme.inkSecondary) { onSave(false) }
                        .disabled(edits.isEmpty || saving)
                        .opacity(edits.isEmpty || saving ? 0.4 : 1)
                }
                .padding(12)
                .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
            }
        }
        .background(Theme.surfaceBase)
    }
}

/// TweakBar.tsx: the controls a design declared, in a row or a column.
struct TweakBar: View {
    let manifest: Manifest
    let values: [String: JSONValue]
    var column = false
    let onChange: (String, JSONValue) -> Void

    var body: some View {
        if column {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(manifest.tweaks, id: \.0) { name, tweak in
                    VStack(alignment: .leading, spacing: 6) { item(name, tweak) }
                        .padding(.horizontal, 12).padding(.vertical, 10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .overlay(alignment: .bottom) { Rectangle().fill(Theme.white(0.04)).frame(height: 1) }
                }
            }
        } else if !manifest.tweaks.isEmpty {
            HStack(spacing: 12) {
                ForEach(manifest.tweaks, id: \.0) { name, tweak in
                    HStack(spacing: 6) { item(name, tweak) }
                }
            }
            .padding(.horizontal, 8).padding(.vertical, 4)
            .accessibilityLabel("Design controls")
        }
    }

    @ViewBuilder
    private func item(_ name: String, _ tweak: Tweak) -> some View {
        let label = tweak.label ?? name
        Text(label).font(.ui(10.5)).foregroundStyle(Theme.inkFaint).lineLimit(1)
        TweakControl(label: label, tweak: tweak, value: tweak.usable(values[name]) ?? tweak.defaultValue) { onChange(name, $0) }
    }
}

private struct TweakControl: View {
    let label: String
    let tweak: Tweak
    let value: JSONValue
    let onChange: (JSONValue) -> Void

    var body: some View {
        switch tweak {
        case .boolean:
            let on = value.boolValue ?? false
            Button { onChange(.bool(!on)) } label: {
                Capsule().fill(on ? Theme.bronzo : Theme.white(0.12)).frame(width: 28, height: 16)
                    .overlay(alignment: .leading) { Circle().fill(.white).frame(width: 10, height: 10).offset(x: on ? 15 : 3) }
                    .animation(.easeOut(duration: 0.15), value: on)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(label)
            .accessibilityAddTraits(.isToggle)
        case .number(_, _, let unit, let lo, let hi, _):
            HStack(spacing: 4) {
                NumberField(value: value.numberValue ?? 0, lo: lo, hi: hi) { onChange(.number($0)) }
                    .accessibilityLabel(label)
                if let unit { Text(unit).font(.ui(10.5)).foregroundStyle(Theme.inkFaint) }
            }
        case .select(_, _, let options):
            let picked = value.stringValue.flatMap { options.contains($0) ? $0 : nil } ?? options.first ?? ""
            Menu {
                ForEach(options, id: \.self) { o in Button(o) { onChange(.string(o)) } }
            } label: {
                Text(picked).font(.ui(11)).foregroundStyle(Theme.ink)
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
            .modifier(Field())
            .accessibilityLabel(label)
        case .color(_, _, let options):
            let colour = value.stringValue ?? ""
            if let options, !options.isEmpty {
                HStack(spacing: 4) {
                    ForEach(options, id: \.self) { c in
                        Button { onChange(.string(c)) } label: {
                            RoundedRectangle(cornerRadius: Theme.radiusSm).fill(Color(css: c) ?? .clear).frame(width: 16, height: 16)
                                .overlay(RoundedRectangle(cornerRadius: Theme.radiusSm).strokeBorder(Theme.white(0.2)))
                                .overlay(RoundedRectangle(cornerRadius: Theme.radiusSm + 1).strokeBorder(Theme.white(c == colour ? 0.7 : 0))
                                    .padding(-1.5))
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(c)
                    }
                }
                .accessibilityLabel(label)
            } else {
                ColorPicker("", selection: Binding(get: { Color(css: colour) ?? .white }, set: { onChange(.string(Self.hex($0))) }),
                            supportsOpacity: false)
                    .labelsHidden()
                    .frame(width: 20, height: 20)
                    .accessibilityLabel(label)
            }
        }
    }

    static func hex(_ c: Color) -> String {
        let n = NSColor(c).usingColorSpace(.sRGB) ?? .white
        func b(_ x: CGFloat) -> Int { Int((x * 255).rounded()) }
        return String(format: "#%02x%02x%02x", b(n.redComponent), b(n.greenComponent), b(n.blueComponent))
    }
}

/// The FIELD look: a faint fill, a hairline, rounded.
private struct Field: ViewModifier {
    func body(content: Content) -> some View {
        content
            .padding(.horizontal, 6).padding(.vertical, 2)
            .background(RoundedRectangle(cornerRadius: Theme.radius).fill(Theme.white(0.04)))
            .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.08)))
    }
}

/// A number field that ignores half-typed text instead of writing 0.
private struct NumberField: View {
    let value: Double
    let lo: Double?
    let hi: Double?
    let onChange: (Double) -> Void
    @State private var text: String?

    var body: some View {
        TextField("", text: Binding(get: { text ?? Self.format(value) }, set: { t in
            text = t
            if let n = Double(t.trimmingCharacters(in: .whitespaces)), n.isFinite { onChange(n) }
        }))
        .textFieldStyle(.plain)
        .font(.ui(11).monospacedDigit())
        .foregroundStyle(Theme.ink)
        .frame(width: 56)
        .modifier(Field())
        .onSubmit { text = nil }
        .onChange(of: value) { _, _ in if text.flatMap(Double.init) != value { text = nil } }
    }

    static func format(_ v: Double) -> String { v == v.rounded() && abs(v) < 1e15 ? String(Int(v)) : String(v) }
}

/// The canvas's side panel: every artboard takes the same values.
struct TweaksAside: View {
    let label: String
    let manifest: Manifest
    let values: [String: JSONValue]
    let onChange: (String, JSONValue) -> Void

    var body: some View {
        Aside(width: 220) {
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    RailHeading(title: "Tweaks \u{b7} \(label)")
                    TweakBar(manifest: manifest, values: values, column: true, onChange: onChange)
                }
            }
            Text("Every artboard takes the same values.").font(.ui(11.5)).foregroundStyle(Theme.inkFaint)
                .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 12).padding(.vertical, 10)
        }
        .accessibilityLabel("Tweaks")
    }
}

/// DesignCanvas.tsx: a design's artboards side by side on a canvas that scrolls and zooms.
struct DesignCanvasView: View {
    let model: BrowserModel
    let s: String
    let boards: [Artboard]
    let selected: String
    let area: CGSize

    var body: some View {
        let zoom = model.canvasZoom(s, area: area)
        let (placed, width, height) = DesignCanvas.place(boards, zoom: zoom)
        ZStack(alignment: .bottomLeading) {
            ScrollView([.horizontal, .vertical]) {
                ZStack(alignment: .topLeading) {
                    ForEach(Array(boards.enumerated()), id: \.element.id) { i, b in
                        let at = placed[i]
                        let w = DesignCanvas.jsRound(Double(b.width) * zoom), h = DesignCanvas.jsRound(Double(b.height) * zoom)
                        let isSelected = b.id == selected
                        Button { model.selectBoard(s, b.id) } label: {
                            HStack(alignment: .firstTextBaseline, spacing: 6) {
                                Text(b.label).foregroundStyle(isSelected ? Theme.ink : Theme.inkSecondary)
                                Text("\(b.width) \u{d7} \(b.height)")
                            }
                            .font(mono11).foregroundStyle(Theme.inkFaint).fixedSize()
                            .frame(height: DesignCanvas.labelHeight - 4, alignment: .bottomLeading)
                        }
                        .buttonStyle(.plain)
                        .offset(x: at.x, y: at.y - DesignCanvas.labelHeight)
                        WebSlot(model: model, key: "\(s)#board:\(b.id)", token: b.id, zoom: zoom)
                            .frame(width: w, height: h)
                            .background(Color.white)
                            .overlay(RoundedRectangle(cornerRadius: 0).stroke(Theme.statusBlue, lineWidth: 2).padding(-3)
                                .opacity(isSelected ? 1 : 0))
                            .offset(x: at.x, y: at.y)
                    }
                }
                .frame(width: width, height: height, alignment: .topLeading)
            }
            .background(DotGrid())
            ZoomPill(zoom: zoom, onOut: { model.zoom(s, -1, area: area) }, onIn: { model.zoom(s, 1, area: area) },
                     onFit: { model.zoom(s, 0, area: area) })
                .padding(10)
        }
        .background(Theme.surfaceSunken)
    }
}

/// `radial-gradient(rgba(255,255,255,0.10) 1px, transparent 1px)` every 16px.
private struct DotGrid: View {
    var body: some View {
        Canvas { ctx, size in
            var y = 0.0
            while y < size.height {
                var x = 0.0
                while x < size.width {
                    ctx.fill(Path(ellipseIn: CGRect(x: x + 7, y: y + 7, width: 2, height: 2)), with: .color(Theme.white(0.10)))
                    x += 16
                }
                y += 16
            }
        }
    }
}

private struct ZoomPill: View {
    let zoom: Double
    let onOut: () -> Void
    let onIn: () -> Void
    let onFit: () -> Void

    var body: some View {
        HStack(spacing: 2) {
            step(.minus, "Zoom out", onOut)
            Text("\(Int(DesignCanvas.jsRound(zoom * 100)))%").font(.system(size: 11.5, design: .monospaced).monospacedDigit())
                .foregroundStyle(Theme.ink).padding(.horizontal, 6).accessibilityLabel("Zoom")
            step(.plus, "Zoom in", onIn)
            step(.maximize, "Fit every artboard", onFit)
        }
        .padding(2)
        .background(RoundedRectangle(cornerRadius: Theme.radius).fill(Theme.surfaceOverlay))
        .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.12)))
    }

    private func step(_ icon: LucideGlyph, _ label: String, _ action: @escaping () -> Void) -> some View {
        Hovering { hover in
            Button(action: action) {
                LucideIcon(icon, size: 12, strokeWidth: 2).foregroundStyle(Theme.inkSecondary).frame(width: 24, height: 24)
                    .background(RoundedRectangle(cornerRadius: Theme.radius).fill(hover ? Theme.white(0.06) : .clear))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .help(label)
        .accessibilityLabel(label)
    }
}
