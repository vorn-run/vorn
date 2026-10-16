import SwiftUI

/// A route over the screen, as a sheet.
struct Sheet: Identifiable, Hashable {
    let id: String
}

/// One navigation stack's routes and sheet; the core says where to go after each act.
final class Nav: ObservableObject {
    @Published var path: [String] = []
    @Published var sheet: Sheet?
    let core: Core
    /// The stack a sheet's stack returns to.
    weak var parent: Nav?
    /// On a split view's list column: the stack its rows open into.
    weak var detail: Nav?

    init(core: Core, parent: Nav? = nil) {
        self.core = core
        self.parent = parent
    }

    func push(_ route: String) {
        if let detail {
            detail.path = [route]
            core.front = detail
        } else {
            path.append(route)
        }
    }

    func present(_ route: String) { sheet = Sheet(id: route) }

    func dismiss() {
        guard let parent else { return }
        parent.sheet = nil
        core.front = parent
    }

    func act(_ screen: String, _ action: [String: Any]) {
        core.front = self
        core.act(screen, action)
        if let go = core.takeGo() { apply(go) }
    }

    /// `back`, `replace:<route>` or a route, from where this stack is.
    func apply(_ go: String) {
        if go == "back" {
            if parent != nil { dismiss() } else if !path.isEmpty { path.removeLast() }
        } else if go.hasPrefix("replace:") {
            let route = String(go.dropFirst("replace:".count))
            if let parent {
                dismiss()
                parent.push(route)
            } else {
                if !path.isEmpty { path.removeLast() }
                push(route)
            }
        } else if !go.isEmpty {
            push(go)
        }
    }
}

/// A NavigationStack over `nav`, opening routes and presenting its sheets.
struct Stack<Root: View>: View {
    let core: Core
    @ObservedObject var nav: Nav
    @ViewBuilder let root: () -> Root

    var body: some View {
        NavigationStack(path: $nav.path) {
            root().navigationDestination(for: String.self) { Destination(core: core, route: $0) }
        }
        .environmentObject(nav)
        .sheet(item: $nav.sheet) { SheetStack(core: core, route: $0.id, parent: nav) }
    }
}

struct SheetStack: View {
    let core: Core
    let route: String
    @StateObject private var nav: Nav

    init(core: Core, route: String, parent: Nav) {
        self.core = core
        self.route = route
        _nav = StateObject(wrappedValue: Nav(core: core, parent: parent))
    }

    var body: some View {
        Stack(core: core, nav: nav) {
            Screen(core: core, name: route)
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button("Cancel") { nav.dismiss() } }
                }
        }
        .presentationDetents(route.hasSuffix("-scope") || route.hasSuffix("-trigger") ? [.medium, .large] : [.large])
    }
}

struct Destination: View {
    let core: Core
    let route: String

    var body: some View {
        if route.hasPrefix("session:") {
            SessionScreen(core: core, id: String(route.dropFirst("session:".count)))
        } else if route.hasPrefix("settings:") {
            Screen(core: core, name: route, title: core.tree("settings")?.walk()
                .first { $0.value == route }?.walk().first { $0.k == "text" }?.label)
                .navigationBarTitleDisplayMode(.inline)
        } else {
            Screen(core: core, name: route).navigationBarTitleDisplayMode(.inline)
        }
    }
}

extension Node {
    /// The screen's own nodes, under the columns that only wrap them.
    var top: [Node] {
        var n = self
        while n.kids.count == 1, n.kids[0].k == "column" { n = n.kids[0] }
        return n.kids
    }

    /// The text a link or bar item shows.
    var caption: String { label?.isEmpty == false ? label! : kids.first { $0.k == "text" }?.label ?? "" }
    var icon: Node? { kids.first { $0.k == "icon" } }
}

/// One screen's tree as a List, re-read whenever vornd sends something.
struct Screen: View {
    @ObservedObject var core: Core
    @EnvironmentObject var nav: Nav
    let name: String
    var title: String?
    var extra: AnyView?

    var body: some View {
        let _ = core.revision
        let top = core.tree(name)?.top ?? []
        let bar = top.first { $0.k == "bar" }
        List {
            Rows(screen: name, nodes: top.filter { $0.k != "bar" })
            if let extra { extra }
        }
        .listStyle(.insetGrouped)
        .navigationTitle(bar?.label ?? title ?? "")
        .toolbar {
            if let bar, !bar.kids.isEmpty {
                ToolbarItemGroup(placement: .topBarTrailing) {
                    ForEach(bar.kids) { BarItem(screen: name, node: $0) }
                }
            }
        }
        .refreshable { nav.act(name, ["refresh": true]) }
    }
}

/// A navigation bar control: a link, a button or a More menu.
struct BarItem: View {
    @EnvironmentObject var nav: Nav
    let screen: String
    let node: Node

    var body: some View {
        switch node.k {
        case "link":
            Button {
                if node.style == "sheet" { nav.present(node.value ?? "") } else { nav.push(node.value ?? "") }
            } label: {
                if let i = node.icon { VornIcon(i) } else { Text(node.caption) }
            }
            .accessibilityLabel(node.caption)
        case "more":
            More(screen: screen, node: node)
        default:
            Control(screen: screen, node: node)
        }
    }
}

/// A More menu; entries that ask first ask once the menu closes.
struct More: View {
    @EnvironmentObject var nav: Nav
    let screen: String
    let node: Node
    @State private var asking: Node?

    var body: some View {
        Menu {
            ForEach(node.kids) { n in
                if n.k == "link" {
                    Button {
                        if n.style == "sheet" { nav.present(n.value ?? "") } else { nav.push(n.value ?? "") }
                    } label: {
                        if let i = n.icon?.value { Label(n.caption, vorn: i) } else { Text(n.caption) }
                    }
                } else if n.k == "button" || n.k == "toggle" {
                    Button(role: n.style == "danger" ? .destructive : nil) {
                        if n.confirm != nil { asking = n } else if let id = n.id_ { nav.act(screen, ["id": id]) }
                    } label: {
                        if let i = n.value, !i.isEmpty { Label(n.label ?? "", vorn: i) } else { Text(n.label ?? "") }
                    }
                }
            }
        } label: {
            VornIcon(name: "ellipsis").frame(minWidth: 44, minHeight: 44)
                .contentShape(.hoverEffect, Circle()).hoverEffect()
        }
        .accessibilityLabel("More")
        .confirmationDialog(asking?.confirm ?? "", isPresented: Binding(get: { asking != nil }, set: { if !$0 { asking = nil } }),
                            titleVisibility: .visible) {
            Button(asking?.label ?? "OK", role: .destructive) {
                if let id = asking?.id_ { nav.act(screen, ["id": id, "confirmed": true]) }
            }
        }
    }
}

/// Nodes as List content: sections become Sections, lists and columns their rows.
struct Rows: View {
    let screen: String
    let nodes: [Node]

    var body: some View {
        ForEach(nodes) { n in
            switch n.k {
            case "section":
                if (n.label ?? "").isEmpty {
                    Section { Rows(screen: screen, nodes: n.kids) }
                } else {
                    Section(n.label ?? "") { Rows(screen: screen, nodes: n.kids) }
                }
            case "list":
                Rows(screen: screen, nodes: n.kids)
            case "column" where n.kids.contains(where: Self.isRow):
                Rows(screen: screen, nodes: n.kids)
            case "divider":
                EmptyView()
            case "empty":
                Empty(screen: screen, node: n)
            case "choice" where (n.label ?? "").isEmpty:
                Section { Tabs(screen: screen, node: n) }
                    .listRowInsets(EdgeInsets())
                    .listRowBackground(Color.clear)
            default:
                Control(screen: screen, node: n)
            }
        }
    }

    static func isRow(_ n: Node) -> Bool {
        ["section", "list", "link", "toggle", "choice", "field", "row", "empty", "option", "button"].contains(n.k)
    }
}

struct Empty: View {
    let screen: String
    let node: Node

    var body: some View {
        ContentUnavailableView {
            Label(node.label ?? "", vorn: (node.value ?? "").isEmpty ? "loader-circle" : "inbox")
        } description: {
            if let b = node.value, !b.isEmpty { Text(b) }
        } actions: {
            ForEach(node.kids) { Control(screen: screen, node: $0).buttonStyle(.borderedProminent) }
        }
        .listRowBackground(Color.clear)
    }
}

/// The screen's tabs: segments when few, else a row of capsules that scrolls.
struct Tabs: View {
    @EnvironmentObject var nav: Nav
    let screen: String
    let node: Node

    var body: some View {
        if node.kids.count <= 3 {
            Picker("", selection: Binding(
                get: { node.kids.first { $0.on == true }?.id_ ?? "" },
                set: { nav.act(screen, ["id": $0]) }
            )) {
                ForEach(node.kids) { Text($0.label ?? "").tag($0.id_ ?? "") }
            }
            .pickerStyle(.segmented)
            .padding(.vertical, 4)
        } else {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(node.kids) { o in
                        Button { if let id = o.id_ { nav.act(screen, ["id": id]) } } label: {
                            Text(o.label ?? "").font(.subheadline.weight(.medium))
                                .padding(.horizontal, 14).frame(minHeight: 36)
                                .background(o.on == true ? Palette.ink : Color(white: 0.16), in: Capsule())
                                .foregroundStyle(o.on == true ? Color.black : Palette.ink)
                        }
                        .buttonStyle(.plain)
                        .accessibilityAddTraits(o.on == true ? .isSelected : [])
                        .hoverEffect(.highlight)
                    }
                }
                .padding(.vertical, 4)
            }
        }
    }
}

/// One node as a control or a row's content.
struct Control: View {
    @EnvironmentObject var nav: Nav
    let screen: String
    let node: Node
    @State private var text = ""
    @State private var asking = false
    @State private var copied = false

    private func act(_ extra: [String: Any] = [:]) {
        guard let id = node.id_ else { return }
        nav.act(screen, ["id": id].merging(extra) { $1 })
    }

    var body: some View {
        switch node.k {
        case "link":
            if node.style == "sheet" {
                Button { nav.present(node.value ?? "") } label: {
                    HStack(spacing: 12) { inline(node.kids) }.contentShape(Rectangle())
                }
                .foregroundStyle(Palette.ink)
            } else if nav.detail != nil {
                Button { nav.push(node.value ?? "") } label: {
                    HStack(spacing: 12) {
                        inline(node.kids)
                        Spacer(minLength: 0)
                        VornIcon(name: "chevron-right", ink: Color(white: 0.35)).scaleEffect(0.75)
                    }
                    .contentShape(Rectangle())
                }
                .foregroundStyle(Palette.ink)
                .modifier(Swipes(screen: screen, row: node))
            } else {
                NavigationLink(value: node.value ?? "") {
                    HStack(spacing: 12) { inline(node.kids) }
                }
                .modifier(Swipes(screen: screen, row: node))
            }
        case "toggle":
            Toggle(node.label ?? "", isOn: Binding(get: { node.on ?? false }, set: { _ in act() }))
        case "option":
            Button { act() } label: {
                HStack(spacing: 12) {
                    if let i = node.icon { VornIcon(i, ink: .secondary) }
                    Text(node.label ?? "").foregroundStyle(Palette.ink)
                    Spacer()
                    if node.on == true { VornIcon(name: "check", ink: Palette.bronze) }
                }
                .contentShape(Rectangle())
            }
            .accessibilityAddTraits(node.on == true ? .isSelected : [])
        case "choice":
            Picker(node.label ?? "", selection: Binding(
                get: { node.kids.first { $0.on == true }?.id_ ?? "" },
                set: { nav.act(screen, ["id": $0]) }
            )) {
                ForEach(node.kids) { Text($0.label ?? "").tag($0.id_ ?? "") }
            }
            .pickerStyle(.menu)
        case "field":
            Group {
                if node.secret == true {
                    SecureField(node.label ?? "", text: $text)
                } else if node.style == "multiline" {
                    TextField(node.label ?? "", text: $text, axis: .vertical).lineLimit(3...12)
                } else {
                    TextField(node.label ?? "", text: $text)
                }
            }
            .onAppear { text = node.value ?? "" }
            .onChange(of: text) { _, v in if v != (node.value ?? "") { act(["value": v]) } }
            .onSubmit { act(["value": text, "submit": true]) }
            .textInputAutocapitalization(node.style == "multiline" ? .sentences : .never)
        case "button":
            Button(role: node.style == "danger" ? .destructive : nil) {
                if node.confirm != nil { asking = true } else { act() }
            } label: {
                if let icon = node.value, !icon.isEmpty, (node.label ?? "").isEmpty {
                    VornIcon(name: icon)
                } else if node.style == "primary" {
                    Text(node.label ?? "").fontWeight(.semibold).foregroundStyle(Palette.bronze)
                } else {
                    Text(node.label ?? "")
                }
            }
            .buttonStyle(.borderless)
            .confirmationDialog(node.confirm ?? "", isPresented: $asking, titleVisibility: .visible) {
                Button(node.label ?? "OK", role: .destructive) { act(["confirmed": true]) }
            }
        case "copy":
            Button {
                UIPasteboard.general.string = node.value
                copied = true
                DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { copied = false }
            } label: {
                Label { Text(copied ? "Copied" : (node.label ?? "Copy")) } icon: { VornIcon(name: copied ? "check" : "copy") }
            }
        case "more":
            More(screen: screen, node: node)
        case "empty":
            Empty(screen: screen, node: node)
        case "row", "column":
            HStack(spacing: 12) { inline(node.kids) }
        default:
            inline([node])
        }
    }

    /// Erased, since rows nest rows.
    private func inline(_ nodes: [Node]) -> AnyView {
        AnyView(ForEach(nodes) { n in
            switch n.k {
            case "text": Leaf.text(n, row: node.k == "link")
            case "icon": VornIcon(n, ink: .secondary)
            case "status": Circle().fill(Palette.tone(n.style)).frame(width: 8, height: 8)
                .accessibilityLabel(n.value ?? "")
            case "badge": Text(n.value ?? "").font(.caption2.weight(.semibold))
                .padding(.horizontal, 6).padding(.vertical, 2)
                .background(Palette.tone(n.style).opacity(0.2), in: Capsule())
            case "column": VStack(alignment: .leading, spacing: 2) { inline(n.kids) }
            case "row": HStack(spacing: 8) { inline(n.kids) }
            case "drawn", "divider", "swipe": EmptyView()
            default: Control(screen: screen, node: n)
            }
            if n.k == "text" && n.style == "body" && nodes.count > 1 && n.id == nodes.first?.id { Spacer(minLength: 8) }
            if n.k == "column" && nodes.count > 1 && n.id != nodes.last?.id { Spacer(minLength: 8) }
        })
    }
}

/// A row's swipe actions, from the `swipe` nodes inside it; one that asks first asks once the row closes.
struct Swipes: ViewModifier {
    @EnvironmentObject var nav: Nav
    let screen: String
    let row: Node
    @State private var asking: Node?

    func body(content: Content) -> some View {
        content
            .swipeActions(edge: .leading, allowsFullSwipe: true) { buttons("leading") }
            .swipeActions(edge: .trailing, allowsFullSwipe: false) { buttons("trailing") }
            .confirmationDialog(asking?.confirm ?? "", isPresented: Binding(get: { asking != nil }, set: { if !$0 { asking = nil } }),
                                titleVisibility: .visible) {
                Button(asking?.label ?? "OK", role: .destructive) {
                    if let id = asking?.id_ { nav.act(screen, ["id": id, "confirmed": true]) }
                }
            }
    }

    private func buttons(_ edge: String) -> some View {
        ForEach(row.kids.filter { $0.k == "swipe" && $0.style == edge }.flatMap(\.kids)) { b in
            Button {
                if b.confirm != nil { asking = b } else if let id = b.id_ { nav.act(screen, ["id": id]) }
            } label: {
                Label(b.label ?? "", vorn: b.value ?? "")
            }
            .tint(b.style == "danger" ? .red : b.style == "primary" ? .green : .gray)
        }
    }
}

enum Leaf {
    /// `row`: a list row's line, kept to one.
    @ViewBuilder static func text(_ n: Node, row: Bool = false) -> some View {
        let s = n.label ?? ""
        switch n.style {
        case "title": Text(s).font(.body.weight(.medium)).lineLimit(row ? 1 : nil)
        case "secondary": Text(s).font(.footnote).foregroundStyle(.secondary).lineLimit(row ? 1 : nil)
        case "mono": Text(s).font(.footnote.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
        case "error": Text(s).font(.footnote).foregroundStyle(.red)
        default: Text(s).textSelection(.enabled)
        }
    }
}

