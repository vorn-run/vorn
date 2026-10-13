import Foundation

/// The table a gate's JSON list draws, and the draft around it (GateJsonEditor.tsx).
struct GateTable: Equatable {
    struct Row: Equatable {
        var values: OObject
        var removed = false
        /// Text typed into number cells, by key, in the order typed.
        var typed: [(key: String, text: String)] = []
        var had: Set<String>
        var nullable: Set<String>

        static func == (a: Row, b: Row) -> Bool {
            a.values == b.values && a.removed == b.removed && a.had == b.had && a.nullable == b.nullable
                && a.typed.map(\.key) == b.typed.map(\.key) && a.typed.map(\.text) == b.typed.map(\.text)
        }

        func typedText(_ key: String) -> String? { typed.first { $0.key == key }?.text }

        mutating func setTyped(_ key: String, _ text: String) {
            if let i = typed.firstIndex(where: { $0.key == key }) { typed[i].text = text } else { typed.append((key, text)) }
        }
    }

    struct Failure: Error, Equatable {
        var error: String
        var line: Int?
    }

    var rows: [Row]
    var wrapper: (object: OObject, key: String)?

    static func == (a: GateTable, b: GateTable) -> Bool {
        a.rows == b.rows && a.wrapper?.key == b.wrapper?.key && a.wrapper?.object == b.wrapper?.object
    }

    /// parseRows: the rows a table can draw from `text`, or why it cannot; an empty list counts.
    static func parse(_ text: String) -> Result<GateTable, Failure> {
        let data: OJSON
        do {
            data = try OJSON.parse(text)
        } catch {
            return .failure(Failure(error: "Line \(error.line), column \(error.column): \(error.message)", line: error.line))
        }
        let list: (items: [OJSON], wrapperKey: String?)
        switch itemList(data) {
        case .failure(let f): return .failure(f)
        case .success(let l): list = l
        }
        if !list.items.isEmpty && !isRecordList(list.items) {
            return .failure(Failure(error: "Not a table: every item has to be an object."))
        }
        let rows = list.items.compactMap(\.objectValue).map { o in
            Row(values: o, had: Set(o.keys), nullable: Set(o.entries.filter { $0.value == .null }.map(\.key)))
        }
        if let key = list.wrapperKey, let object = data.objectValue {
            return .success(GateTable(rows: rows, wrapper: (object, key)))
        }
        return .success(GateTable(rows: rows))
    }

    /// toItemList from shared/item-list.ts.
    static func itemList(_ value: OJSON) -> Result<(items: [OJSON], wrapperKey: String?), Failure> {
        var data = value
        if case .string(let s) = value {
            let text = s.trimmingCharacters(in: .whitespacesAndNewlines)
            if text.isEmpty { return .success(([], nil)) }
            do {
                data = try OJSON.parse(text)
            } catch {
                return .failure(Failure(
                    error: "Not a list: the JSON is invalid at line \(error.line), column \(error.column)."))
            }
        }
        if let a = data.arrayValue { return .success((a, nil)) }
        if let o = data.objectValue {
            let lists = o.entries.filter { $0.value.arrayValue != nil }
            if lists.count == 1 { return .success((lists[0].value.arrayValue!, lists[0].key)) }
            if lists.count > 1 {
                return .failure(Failure(
                    error: "Not a list: the object holds several (\(lists.map(\.key).joined(separator: ", ")))."))
            }
        }
        return .failure(Failure(error: "Not a list: expected a JSON array, or an object holding one."))
    }

    static func isRecordList(_ items: [OJSON]) -> Bool {
        !items.isEmpty && items.allSatisfy { $0.objectValue != nil }
    }

    /// GateEditor: the gate's text is a list of records, so it opens as a table.
    static func isTable(_ text: String) -> Bool {
        guard case .success(let list) = itemList(.string(text)) else { return false }
        return isRecordList(list.items)
    }

    /// Every key any row has, in the order first seen.
    var columns: [String] {
        var seen: [String] = []
        for row in rows { for key in row.values.keys where !seen.contains(key) { seen.append(key) } }
        return seen
    }

    /// The kept rows, wrapped back in the object they came from.
    var value: OJSON {
        let kept = OJSON.array(rows.filter { !$0.removed }.map { .object($0.values) })
        guard var wrapper else { return kept }
        wrapper.object[wrapper.key] = kept
        return .object(wrapper.object)
    }

    /// The first kept cell holding something other than the number it should.
    var badCell: String? {
        for (i, row) in rows.enumerated() where !row.removed {
            for (key, text) in row.typed where Self.parseNumber(text) == nil {
                return "Row \(i + 1), \(key): not a number."
            }
        }
        return nil
    }

    static func parseNumber(_ text: String) -> Double? {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty, let n = Double(t), n.isFinite else { return nil }
        return n
    }

    // MARK: Cell edits

    mutating func setBool(_ row: Int, _ key: String, _ on: Bool) { rows[row].values[key] = .bool(on) }

    /// A number cell keeps the last number it held, so an invalid one never reaches the JSON.
    mutating func setNumberText(_ row: Int, _ key: String, _ text: String) {
        rows[row].setTyped(key, text)
        if let n = Self.parseNumber(text) { rows[row].values[key] = .number(n) }
    }

    /// Emptying a cell restores null where the row held null, and drops a key the row never had.
    mutating func setText(_ row: Int, _ key: String, _ text: String) {
        if text.isEmpty && rows[row].nullable.contains(key) {
            rows[row].values[key] = .null
        } else if text.isEmpty && !rows[row].had.contains(key) {
            rows[row].values[key] = nil
        } else {
            rows[row].values[key] = .string(text)
        }
    }

    mutating func setRemoved(_ row: Int, _ removed: Bool) { rows[row].removed = removed }
}

/// What the JSON editor holds: the table or its source, and what it would send.
struct GateJsonDraft: Equatable {
    enum Mode { case table, source }

    let original: String
    var table: GateTable?
    var mode: Mode
    var source: String

    init(original: String, initial: String) {
        self.original = original
        if case .success(let t) = GateTable.parse(initial) { table = t } else { table = nil }
        mode = table == nil ? .source : .table
        source = initial
    }

    var sourceParse: Result<GateTable, GateTable.Failure>? { mode == .source ? GateTable.parse(source) : nil }

    var sourceError: GateTable.Failure? {
        if case .failure(let f)? = sourceParse { return f }
        return nil
    }

    /// Why the draft cannot be saved or approved, or nil when it can.
    var invalid: String? { mode == .source ? sourceError?.error : table?.badCell }

    /// The pretty-printed text the draft makes, or nil while the source does not parse.
    var text: String? {
        if mode == .source {
            guard case .success? = sourceParse, let v = try? OJSON.parse(source) else { return nil }
            return v.stringify(indent: 2)
        }
        return table?.value.stringify(indent: 2)
    }

    /// The rewrite to send; nil when it says what the gate already had.
    var edited: String? {
        guard let text, Self.canonical(text) != Self.canonical(original) else { return nil }
        return text
    }

    /// Same value whatever the spacing; unparseable text only matches itself.
    static func canonical(_ text: String) -> String {
        (try? OJSON.parse(text))?.stringify() ?? text
    }

    var count: String {
        if mode == .source {
            if case .success(let t)? = sourceParse { return "\(t.rows.count) of \(t.rows.count) kept" }
            return "—"
        }
        let rows = table?.rows ?? []
        return "\(rows.filter { !$0.removed }.count) of \(rows.count) kept"
    }

    var canShowTable: Bool {
        if mode == .table { return true }
        if case .success? = sourceParse { return true }
        return false
    }

    mutating func toSource() {
        guard mode != .source else { return }
        if let table { source = table.value.stringify(indent: 2) }
        mode = .source
    }

    mutating func toTable() {
        guard mode != .table, case .success(let t)? = sourceParse else { return }
        table = t
        mode = .table
    }

    mutating func revert() {
        if case .success(let t) = GateTable.parse(original) {
            table = t
            mode = .table
        } else {
            table = nil
            mode = .source
        }
        source = original
    }
}
