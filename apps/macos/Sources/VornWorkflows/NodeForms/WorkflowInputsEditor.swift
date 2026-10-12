import SwiftUI
import VornCore
import VornUI

/// The edits WorkflowInputsEditor.tsx makes on the manual trigger's `inputs`.
enum InputEdits {
    static let types: [(value: String, label: String)] = [
        ("text", "Text"), ("textarea", "Long text"), ("number", "Number"), ("select", "Select"),
        ("boolean", "Toggle"), ("project", "Project"), ("branch", "Branch"),
    ]

    /// normalizeKey: keys become `{{inputs.<key>}}`, so identifier characters only, never a leading digit.
    static func normalizeKey(_ raw: String) -> String {
        var out = String(raw.map { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "_") ? $0 : "_" })
        if let first = out.first, first.isASCII, first.isNumber { out = "_" + out }
        return out
    }

    /// Keys used by more than one input.
    static func duplicateKeys(_ inputs: [JSONValue]) -> Set<String> {
        var seen = Set<String>()
        var dupes = Set<String>()
        for key in inputs.map({ $0.str("key") ?? "" }) where !key.isEmpty {
            if !seen.insert(key).inserted { dupes.insert(key) }
        }
        return dupes
    }

    static func add(_ inputs: [JSONValue]) -> [JSONValue] {
        let key = TemplateVars.ensureUniqueSlug("input", existing: Set(inputs.compactMap { $0.str("key") }))
        return inputs + [["key": .string(key), "label": "", "type": "text"]]
    }

    /// Options only mean something for a select; the default is hidden for select and toggle.
    static func setType(_ input: JSONValue, _ type: String) -> JSONValue {
        let keepsDefault = type != "boolean" && type != "select"
        return input.merged([
            "type": .string(type),
            "options": type == "select" ? (input["options"] ?? .array([])) : nil,
            "defaultValue": keepsDefault ? input["defaultValue"] : nil,
        ])
    }

    static func setOptions(_ input: JSONValue, _ text: String) -> JSONValue {
        let values = text.split(separator: ",", omittingEmptySubsequences: false)
            .map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
        return input.merged(["options": .array(values.map { ["value": .string($0), "label": .string($0)] })])
    }

    static func optionsText(_ input: JSONValue) -> String {
        (input["options"]?.arrayValue ?? []).compactMap { $0.str("value") }.joined(separator: ", ")
    }

    static func defaultText(_ input: JSONValue) -> String {
        switch input["defaultValue"] {
        case .string(let s): s
        case .number(let n): n == n.rounded() && abs(n) < 1e15 ? String(Int64(n)) : String(n)
        case .bool(let b): b ? "true" : "false"
        default: ""
        }
    }
}

/// WorkflowInputsEditor.tsx: the manual trigger's run inputs.
struct WorkflowInputsEditor: View {
    let inputs: [JSONValue]
    let onChange: ([JSONValue]) -> Void

    var body: some View {
        let dupes = InputEdits.duplicateKeys(inputs)
        VStack(alignment: .leading, spacing: 0) {
            FormLabel("Run Inputs")
            (Text("Values the user is asked for before the run starts. Use them anywhere as ")
                + Text("{{inputs.key}}").font(.system(size: 11, design: .monospaced)).foregroundColor(Theme.gray400)
                + Text("."))
                .font(.system(size: 11))
                .foregroundStyle(Theme.gray500)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.bottom, 8)
            VStack(spacing: 8) {
                ForEach(Array(inputs.enumerated()), id: \.offset) { index, input in
                    InputCard(input: input, duplicate: dupes.contains(input.str("key") ?? "")) { next in
                        var all = inputs
                        all[index] = next
                        onChange(all)
                    } onRemove: {
                        var all = inputs
                        all.remove(at: index)
                        onChange(all)
                    }
                }
            }
            FormLinkButton(symbol: .plus, title: "Add input") { onChange(InputEdits.add(inputs)) }
                .padding(.top, 8)
        }
    }
}

private struct InputCard: View {
    let input: JSONValue
    let duplicate: Bool
    let onChange: (JSONValue) -> Void
    let onRemove: () -> Void

    private var type: String { input.str("type") ?? "text" }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                field("key", text: bind("key", transform: InputEdits.normalizeKey), mono: true, invalid: duplicate)
                Hovering { hovered in
                    Button(action: onRemove) {
                        LucideIcon(.trash2, size: 12)
                            .foregroundStyle(hovered ? Theme.danger : Theme.gray500)
                            .padding(6)
                            .background(hovered ? Theme.white(0.06) : .clear, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .help("Remove input \(input.str("key") ?? "")")
                }
            }
            if duplicate {
                (Text("Duplicate key — only one input named ")
                    + Text(input.str("key") ?? "").font(.system(size: 11, design: .monospaced))
                    + Text(" will reach the run."))
                    .font(.system(size: 11))
                    .foregroundStyle(Theme.danger)
                    .fixedSize(horizontal: false, vertical: true)
            }
            field("Label shown in the run dialog", text: bind("label"))
            HStack(spacing: 8) {
                FormSelect(value: type, options: InputEdits.types.map { SelectOption($0.value, $0.label) }) {
                    onChange(InputEdits.setType(input, $0))
                }
                Button {
                    onChange(input.merged(["required": input.bool("required") == true ? nil : .bool(true)]))
                } label: {
                    HStack(spacing: 6) {
                        Image(systemName: input.bool("required") == true ? "checkmark.square.fill" : "square")
                            .font(.system(size: 12))
                            .foregroundStyle(input.bool("required") == true ? Theme.white(0.8) : Theme.gray500)
                        Text("Required").font(.system(size: 12)).foregroundStyle(Theme.gray400)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .fixedSize()
            }
            if type == "select" {
                field(
                    "Comma-separated choices",
                    text: Binding(get: { InputEdits.optionsText(input) }, set: { onChange(InputEdits.setOptions(input, $0)) }))
            }
            if type != "boolean" && type != "select" {
                field(
                    "Default value (optional)",
                    text: Binding(
                        get: { InputEdits.defaultText(input) },
                        set: { onChange(input.merged(["defaultValue": $0.isEmpty ? nil : .string($0)])) }))
            }
        }
        .padding(10)
        .background(Theme.white(0.02), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.06), lineWidth: 1))
    }

    private func bind(_ key: String, transform: @escaping (String) -> String = { $0 }) -> Binding<String> {
        Binding(get: { input.str(key) ?? "" }, set: { onChange(input.merged([key: .string(transform($0))])) })
    }

    private func field(_ placeholder: String, text: Binding<String>, mono: Bool = false, invalid: Bool = false) -> some View {
        BareTextInput(placeholder: placeholder, text: text, size: 12, mono: mono)
            .fieldChrome(invalid: invalid, horizontal: 10, vertical: 6)
    }
}
