import SwiftUI

/// A `prose` node: its field, written in Markdown, with its `key`s over the keyboard and a preview.
struct Prose: View {
    @EnvironmentObject var nav: Nav
    let screen: String
    let node: Node
    @State private var text = ""
    @State private var selection: TextSelection?
    @State private var previewing = false
    @FocusState private var focused: Bool

    private var field: Node? { node.kids.first { $0.k == "field" } }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Picker("Mode", selection: $previewing) {
                Text("Write").tag(false)
                Text("Preview").tag(true)
            }
            .pickerStyle(.segmented)
            if previewing {
                Group {
                    if text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        Text("Nothing to preview.").foregroundStyle(.secondary)
                    } else {
                        Markdown(source: text)
                    }
                }
                .frame(maxWidth: .infinity, minHeight: 72, alignment: .topLeading)
            } else {
                TextField(field?.label ?? "", text: $text, selection: $selection, axis: .vertical)
                    .lineLimit(3...16)
                    .focused($focused)
                    .toolbar {
                        if focused {
                            ToolbarItemGroup(placement: .keyboard) { keys }
                        }
                    }
            }
        }
        .onAppear { text = field?.value ?? "" }
        .onChange(of: text) { _, v in
            if let id = field?.id_, v != (field?.value ?? "") { nav.act(screen, ["id": id, "value": v]) }
        }
    }

    private var keys: some View {
        HStack(spacing: 0) {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    ForEach(node.kids.filter { $0.k == "key" }) { k in
                        Button(k.label ?? "") { apply(k.value ?? "") }.buttonStyle(KeyStyle(on: false))
                    }
                }
            }
            Button { focused = false } label: { VornIcon(name: "keyboard").frame(width: 44, height: 44) }
                .accessibilityLabel("Hide the keyboard")
        }
    }

    /// The selection as char offsets, the caret at the end when there is none.
    private var range: (Int, Int) {
        let s = text.unicodeScalars
        guard case let .selection(r)? = selection?.indices else { return (s.count, s.count) }
        return (s.distance(from: s.startIndex, to: r.lowerBound), s.distance(from: s.startIndex, to: r.upperBound))
    }

    private func apply(_ key: String) {
        let (start, end) = range
        guard let p = vp_markdown_key(text, key, UInt32(start), UInt32(end)) else { return }
        defer { vp_free_string(p) }
        guard let out = try? JSONSerialization.jsonObject(with: Data(String(cString: p).utf8)) as? [String: Any],
              let next = out["text"] as? String, let a = out["start"] as? Int, let b = out["end"] as? Int else { return }
        text = next
        let s = text.unicodeScalars
        let lo = s.index(s.startIndex, offsetBy: min(a, s.count))
        let hi = s.index(s.startIndex, offsetBy: min(b, s.count))
        selection = TextSelection(range: lo..<hi)
    }
}

/// Markdown as blocks: headings, lists, tasks, quotes, fences and rules, each line's marks inline.
struct Markdown: View {
    let source: String

    private enum Block: Hashable {
        case heading(Int, String), bullet(String), task(Bool, String), quote(String)
        case code(String), rule, para(String), gap
    }

    private var blocks: [Block] {
        var out: [Block] = []
        var fence: [String]?
        for line in source.components(separatedBy: "\n") {
            let t = line.trimmingCharacters(in: .whitespaces)
            if t.hasPrefix("```") {
                if let f = fence { out.append(.code(f.joined(separator: "\n"))); fence = nil } else { fence = [] }
                continue
            }
            if fence != nil { fence!.append(line); continue }
            if t.isEmpty { if out.last != .gap { out.append(.gap) }; continue }
            if t == "---" || t == "***" { out.append(.rule); continue }
            if let n = t.firstIndex(where: { $0 != "#" }), (1...3).contains(t.distance(from: t.startIndex, to: n)), t[n] == " " {
                out.append(.heading(t.distance(from: t.startIndex, to: n), String(t[n...].dropFirst())))
            } else if t.hasPrefix("- [ ] ") || t.hasPrefix("- [x] ") || t.hasPrefix("- [X] ") {
                out.append(.task(!t.hasPrefix("- [ ]"), String(t.dropFirst(6))))
            } else if t.hasPrefix("- ") || t.hasPrefix("* ") {
                out.append(.bullet(String(t.dropFirst(2))))
            } else if t.hasPrefix("> ") {
                out.append(.quote(String(t.dropFirst(2))))
            } else {
                out.append(.para(t))
            }
        }
        if let f = fence { out.append(.code(f.joined(separator: "\n"))) }
        return out
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { _, b in
                switch b {
                case let .heading(level, s):
                    inline(s).font(level == 1 ? .title3.weight(.semibold) : level == 2 ? .headline : .subheadline.weight(.semibold))
                        .padding(.top, 4)
                case let .bullet(s):
                    HStack(alignment: .firstTextBaseline, spacing: 8) { Text("•"); inline(s) }
                case let .task(done, s):
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        VornIcon(name: done ? "square-check" : "square", ink: .secondary).scaleEffect(0.8)
                        inline(s).strikethrough(done).foregroundStyle(done ? .secondary : Palette.ink)
                    }
                case let .quote(s):
                    inline(s).foregroundStyle(.secondary).padding(.leading, 10)
                        .overlay(alignment: .leading) { Rectangle().fill(Color(white: 0.3)).frame(width: 3) }
                case let .code(s):
                    Text(s).font(.footnote.monospaced()).padding(8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Color(white: 0.12), in: RoundedRectangle(cornerRadius: 6))
                case .rule:
                    Divider().padding(.vertical, 4)
                case let .para(s):
                    inline(s)
                case .gap:
                    Spacer().frame(height: 4)
                }
            }
        }
        .textSelection(.enabled)
    }

    private func inline(_ s: String) -> Text {
        let opts = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        return Text((try? AttributedString(markdown: s, options: opts)) ?? AttributedString(s))
    }
}
