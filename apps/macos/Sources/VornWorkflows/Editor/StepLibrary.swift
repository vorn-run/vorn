import SwiftUI
import VornCore
import VornUI

/// step-library-memory.ts: what was picked lately and which groups were left open.
enum StepLibraryMemory {
    static let recentKey = "vorn:recentSteps"
    static let openKey = "vorn:stepLibraryOpen"
    static let keptRecent = 5

    struct RecentAction: Codable, Hashable {
        var connectorId: String
        var action: String
        var connectionId: String?
    }

    static func readRecent(_ m: EditorMemory) -> [RecentAction] { m.json(recentKey, as: [RecentAction].self) ?? [] }

    static func record(_ m: EditorMemory, _ picked: RecentAction) {
        let rest = readRecent(m).filter { $0.connectorId != picked.connectorId || $0.action != picked.action }
        m.setJSON(recentKey, Array(([picked] + rest).prefix(keptRecent)))
    }

    static func readOpen(_ m: EditorMemory) -> Set<String> { Set(m.json(openKey, as: [String].self) ?? []) }

    static func writeOpen(_ m: EditorMemory, _ open: Set<String>) { m.setJSON(openKey, Array(open)) }
}

/// StepLibrary.tsx LibraryScope: what the anchor that opened it allows.
struct LibraryScope: Equatable {
    var bodyOnly = false
    var insideBranch = false
    var triggers = false
    var replacing = false

    init(_ anchor: InsertAnchor) {
        bodyOnly = anchor.bodyOnly
        insideBranch = anchor.insideBranch
        triggers = anchor.isTrigger
        replacing = anchor.replaceNodeId != nil
    }

    init(bodyOnly: Bool = false, insideBranch: Bool = false, triggers: Bool = false, replacing: Bool = false) {
        self.bodyOnly = bodyOnly
        self.insideBranch = insideBranch
        self.triggers = triggers
        self.replacing = replacing
    }
}

/// The library's rows, built from the scope, the query and what this machine has (StepLibrary.tsx's memo).
enum LibraryEntries {
    struct Row: Identifiable, Hashable {
        var id: String
        var label: String
        var pick: LibraryPick
        var glyph: String?
        var look: ConnectorLook?
        var owner: String?
        var recent: StepLibraryMemory.RecentAction?
        var nested = false
    }

    struct Group: Hashable {
        var key: String
        var label: String
        var look: ConnectorLook?
        var detail: String?
        var rows: [Row]
    }

    enum Entry: Identifiable, Hashable {
        case heading(String, String)
        case group(Group, open: Bool)
        case row(Row)

        var id: String {
            switch self {
            case .heading(let k, _): return k
            case .group(let g, _): return g.key
            case .row(let r): return r.id
            }
        }

        var reachable: Bool { if case .heading = self { return false } else { return true } }
    }

    static let triggerItems: [(String, String, String)] = [
        ("manual", "Manual", "zap"), ("recurring", "Recurring schedule", "clock"), ("once", "Schedule once", "calendar"),
        ("taskCreated", "Task created", "listPlus"), ("taskStatusChanged", "Task moved", "arrowRightLeft"),
        ("sessionRestored", "Session restored", "rotateCcw"), ("webhook", "Webhook", "globe"),
    ]

    static let stepItems: [(AddableNodeType, String, String)] = [
        (.agent, "Agent", "play"), (.script, "Script", "terminal"), (.httpRequest, "HTTP request", "globe"),
        (.condition, "Condition", "gitFork"), (.approval, "Approval gate", "hand"), (.loop, "Loop", "repeatIcon"),
    ]

    static func glyph(_ name: String) -> LucideGlyph {
        switch name {
        case "clock": return .clock
        case "calendar": return .calendar
        case "listPlus": return .listPlus
        case "arrowRightLeft": return .arrowRightLeft
        case "rotateCcw": return .rotateCcw
        case "globe": return .globe
        case "play": return .play
        case "terminal": return .terminal
        case "gitFork": return .gitFork
        case "hand": return .hand
        case "repeatIcon": return .repeatIcon
        case "split": return .split
        default: return .zap
        }
    }

    /// The loose rows and the connector groups that match.
    @MainActor static func build(_ scope: LibraryScope, query: String, directory: ConnectorDirectory) -> (flat: [Row], groups: [Group]) {
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        func matches(_ texts: String...) -> Bool { q.isEmpty || texts.contains { $0.lowercased().contains(q) } }
        func matchesAny(_ texts: [String]) -> Bool { q.isEmpty || texts.contains { $0.lowercased().contains(q) } }
        var flat: [Row] = []
        var groups: [Group] = []
        func connectionGroup(_ conn: FormConnection, _ rows: [Row]) {
            guard !rows.isEmpty else { return }
            let look = directory.look(conn.id)
            groups.append(Group(key: "group:\(conn.id)", label: conn.name, look: look,
                                rows: rows.map { var r = $0; r.look = look; r.owner = conn.name; return r }))
        }

        if scope.triggers {
            for (type, label, glyph) in triggerItems where matches(label) {
                flat.append(Row(id: "trigger:\(type)", label: label, pick: .triggerType(type), glyph: glyph))
            }
            for conn in directory.connections {
                let events = (directory.triggers[conn.connectorId] ?? []).filter { matches($0.label.isEmpty ? $0.type : $0.label, conn.name) }
                connectionGroup(conn, events.map { t in
                    Row(id: "event:\(conn.id):\(t.type)", label: t.label.isEmpty ? t.type : t.label,
                        pick: .connectorTrigger(connectionId: conn.id, event: t.type))
                })
            }
            return (flat, groups)
        }

        for (type, label, glyph) in stepItems {
            if scope.bodyOnly && (type == .approval || type == .loop) { continue }
            if scope.insideBranch && type == .loop { continue }
            if scope.replacing && (type == .condition || type == .loop) { continue }
            if !matches(label) { continue }
            flat.append(Row(id: "type:\(type.rawValue)", label: label, pick: .type(type), glyph: glyph))
        }
        if !scope.bodyOnly && !scope.insideBranch && !scope.replacing && matches("Parallel branch") {
            flat.append(Row(id: "parallel", label: "Parallel branch", pick: .parallel, glyph: "split"))
        }
        if !scope.replacing {
            let profiles = directory.connections.filter { $0.realConnectorId == "http" && matches("Call \($0.name)") }.map {
                Row(id: "profile:\($0.id)", label: "Call \($0.name)",
                    pick: .httpProfile(profileConnectionId: $0.id, profileName: $0.name), glyph: "globe")
            }
            let at = flat.firstIndex { $0.id == "type:httpRequest" }.map { $0 + 1 } ?? flat.count
            flat.insert(contentsOf: profiles, at: at)
        }
        for conn in directory.connections {
            let real = conn.realConnectorId
            let rows = (directory.actions[conn.id] ?? []).filter { matches($0.label.isEmpty ? $0.type : $0.label, conn.name) }.map { a in
                let label = a.label.isEmpty ? a.type : a.label
                return Row(id: "action:\(conn.id):\(a.type)", label: label,
                           pick: .connectorAction(connectionId: conn.id, action: a.type, actionLabel: label),
                           recent: real.isEmpty ? nil : .init(connectorId: real, action: a.type, connectionId: conn.id))
            }
            connectionGroup(conn, rows)
        }
        if !scope.replacing {
            let connected = Set(directory.connections.map(\.realConnectorId))
            var unvouched: [Group] = []
            for entry in directory.catalog {
                if connected.contains(entry.id) || entry.actions.isEmpty { continue }
                let actions = matchesAny([entry.name] + entry.keywords) ? entry.actions : entry.actions.filter { matches($0.label) }
                if actions.isEmpty { continue }
                let installed = directory.packs.contains { $0.id == entry.id }
                let released = entry.packUrl?.isEmpty == false
                let look = ConnectorLook(connectorId: entry.id, icon: entry.icon, packaged: false)
                let group = Group(
                    key: "catalog-group:\(entry.id)", label: entry.name, look: look,
                    detail: installed ? "add connection" : !released ? "not released yet" : "install on add",
                    rows: actions.map { a in
                        Row(id: "catalog:\(entry.id):\(a.type)", label: a.label, pick: .catalogAction(connectorId: entry.id, action: a.type, actionLabel: a.label),
                            look: look, owner: entry.name, recent: .init(connectorId: entry.id, action: a.type))
                    })
                if entry.verified { groups.append(group) } else { unvouched.append(group) }
            }
            groups += unvouched
        }
        return (flat, groups)
    }

    /// Searching lists every match in one run; otherwise groups fold under what was picked lately.
    static func entries(flat: [Row], groups: [Group], query: String, triggers: Bool,
                        recent: [StepLibraryMemory.RecentAction], open: Set<String>) -> [Entry]
    {
        if !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return (flat + groups.flatMap(\.rows)).map(Entry.row)
        }
        var out: [Entry] = []
        if !flat.isEmpty {
            out.append(.heading("heading:top", triggers ? "Triggers" : "Steps"))
            out += flat.map(Entry.row)
        }
        let offered = groups.flatMap(\.rows)
        let lately = recent.compactMap { r -> Row? in
            let same = offered.filter { $0.recent?.connectorId == r.connectorId && $0.recent?.action == r.action }
            guard var row = same.first(where: { $0.recent?.connectionId == r.connectionId }) ?? same.first else { return nil }
            row.id = "recent:\(row.id)"
            return row
        }
        if !lately.isEmpty {
            out.append(.heading("heading:recent", "Recent"))
            out += lately.map(Entry.row)
        }
        for g in groups {
            let isOpen = open.contains(g.key)
            out.append(.group(g, open: isOpen))
            if isOpen { out += g.rows.map { var r = $0; r.nested = true; return Entry.row(r) } }
        }
        return out
    }
}

/// StepLibrary.tsx: the docked library every + opens.
struct StepLibraryPanel: View {
    let editor: WorkflowEditorModel
    let scope: LibraryScope
    let onPick: (LibraryPick) -> Void
    let onClose: () -> Void

    @State private var query = ""
    @State private var highlight = 0
    @State private var openGroups: Set<String> = []
    @State private var recent: [StepLibraryMemory.RecentAction] = []
    @FocusState private var searchFocused: Bool

    private var memory: EditorMemory { editor.store.editorMemory }

    var body: some View {
        let built = LibraryEntries.build(scope, query: query, directory: editor.directory)
        let entries = LibraryEntries.entries(
            flat: built.flat, groups: built.groups, query: query, triggers: scope.triggers, recent: recent, open: openGroups)
        let reachable = entries.filter(\.reachable)
        let clamped = min(highlight, max(0, reachable.count - 1))
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text(scope.triggers ? "Add a trigger" : scope.replacing ? "Replace step" : "Add a step")
                        .font(.system(size: 13, weight: .medium)).foregroundStyle(.white)
                    Spacer()
                    PanelCloseButton(help: "Close", action: onClose)
                }
                .frame(height: 19.5)
                HStack(spacing: 6) {
                    LucideIcon(.search, size: 12).foregroundStyle(Theme.gray500)
                    TextField("", text: $query, prompt: Text(scope.triggers ? "Search triggers" : "Search steps and actions")
                        .foregroundStyle(Theme.gray600))
                        .textFieldStyle(.plain)
                        .font(.system(size: 12))
                        .foregroundStyle(.white)
                        .focused($searchFocused)
                        .onChange(of: query) { _, _ in highlight = 0 }
                        .onKeyPress(.downArrow) {
                            highlight = min(clamped + 1, max(0, reachable.count - 1))
                            return .handled
                        }
                        .onKeyPress(.upArrow) {
                            highlight = max(clamped - 1, 0)
                            return .handled
                        }
                        .onKeyPress(.rightArrow) { toggleCurrent(reachable, clamped, open: true) }
                        .onKeyPress(.leftArrow) { toggleCurrent(reachable, clamped, open: false) }
                        .onKeyPress(.return) {
                            guard reachable.indices.contains(clamped) else { return .ignored }
                            switch reachable[clamped] {
                            case .group(let g, _): toggle(g.key)
                            case .row(let r): choose(r)
                            case .heading: break
                            }
                            return .handled
                        }
                        .onKeyPress(.escape) {
                            onClose()
                            return .handled
                        }
                }
                .frame(height: 18)
                .padding(.horizontal, 9)
                .padding(.vertical, 7)
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.08), lineWidth: 1))
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .overlay(alignment: .bottom) { Rectangle().fill(Theme.white(0.08)).frame(height: 1) }

            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    if reachable.isEmpty {
                        Text("Nothing matches").font(.system(size: 11)).foregroundStyle(Theme.gray500)
                            .padding(.horizontal, 10).padding(.vertical, 12)
                    }
                    ForEach(Array(entries.enumerated()), id: \.element.id) { i, entry in
                        let index = reachable.firstIndex { $0.id == entry.id } ?? -1
                        entryView(entry, first: i == 0, lit: index == clamped) { highlight = index }
                    }
                }
                .padding(8)
            }
            .scrollIndicators(.never)
        }
        .frame(width: 280)
        .frame(maxHeight: .infinity, alignment: .top)
        .background(Theme.surfaceNode)
        .overlay(alignment: .leading) { Rectangle().fill(Theme.white(0.08)).frame(width: 1) }
        .onAppear {
            openGroups = StepLibraryMemory.readOpen(memory)
            recent = StepLibraryMemory.readRecent(memory)
            if let q = editor.renderQuery { query = q }
            searchFocused = true
        }
    }

    private func toggleCurrent(_ reachable: [LibraryEntries.Entry], _ i: Int, open: Bool) -> KeyPress.Result {
        guard reachable.indices.contains(i), case .group(let g, _) = reachable[i] else { return .ignored }
        toggle(g.key, open: open)
        return .handled
    }

    private func toggle(_ key: String, open: Bool? = nil) {
        let want = open ?? !openGroups.contains(key)
        guard want != openGroups.contains(key) else { return }
        if want { openGroups.insert(key) } else { openGroups.remove(key) }
        StepLibraryMemory.writeOpen(memory, openGroups)
    }

    private func choose(_ row: LibraryEntries.Row) {
        if let r = row.recent { StepLibraryMemory.record(memory, r) }
        onPick(row.pick)
    }

    @ViewBuilder
    private func entryView(_ entry: LibraryEntries.Entry, first: Bool, lit: Bool, hover: @escaping () -> Void) -> some View {
        switch entry {
        case .heading(_, let label):
            Text(label.uppercased())
                .font(.system(size: 10, design: .monospaced))
                .tracking(0.5)
                .foregroundStyle(Theme.gray600)
                .frame(height: 15)
                .padding(.horizontal, 8)
                .padding(.top, first ? 4 : 12)
                .padding(.bottom, 4)
        case .group(let g, let open):
            Button { toggle(g.key) } label: {
                HStack(spacing: 8) {
                    LucideIcon(.chevronRight, size: 12).foregroundStyle(Theme.gray500)
                        .rotationEffect(.degrees(open ? 90 : 0))
                    mark(g.look)
                    Text(g.label).lineLimit(1)
                    Spacer(minLength: 8)
                    HStack(spacing: 8) {
                        if let d = g.detail { Text(d) }
                        Text("\(g.rows.count)").font(.system(size: 10, design: .monospaced)).monospacedDigit()
                    }
                    .font(.system(size: 10))
                    .foregroundStyle(Theme.gray600)
                    .fixedSize()
                }
                .font(.system(size: 12.5))
                .frame(minHeight: 18.75)
                .foregroundStyle(lit ? Color.white : Theme.inkSecondary)
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
                .background(lit ? Theme.white(0.06) : .clear, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .onHover { if $0 { hover() } }
        case .row(let row):
            Button { choose(row) } label: {
                HStack(spacing: 10) {
                    if let glyph = row.glyph {
                        LucideIcon(LibraryEntries.glyph(glyph), size: 14).foregroundStyle(Theme.inkSecondary)
                    } else if !row.nested {
                        mark(row.look)
                    }
                    Text(row.label).lineLimit(1)
                    if !row.nested, let owner = row.owner {
                        Spacer(minLength: 8)
                        Text(owner).font(.system(size: 10)).foregroundStyle(Theme.gray600).lineLimit(1)
                            .frame(maxWidth: 99, alignment: .trailing)
                            .layoutPriority(1)
                    } else {
                        Spacer(minLength: 0)
                    }
                }
                .font(.system(size: 12.5))
                .frame(minHeight: 18.75)
                .foregroundStyle(lit ? Color.white : Theme.gray300)
                .padding(.leading, row.nested ? 32 : 8)
                .padding(.trailing, 8)
                .padding(.vertical, 6)
                .background(lit ? Theme.white(0.06) : .clear, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .onHover { if $0 { hover() } }
        }
    }

    @ViewBuilder private func mark(_ look: ConnectorLook?) -> some View {
        if let look {
            ConnectorGlyph(connectorId: look.connectorId, icon: look.icon, packaged: look.packaged, size: 14)
        } else {
            LucideIcon(.zap, size: 14).foregroundStyle(Theme.inkSecondary)
        }
    }
}
