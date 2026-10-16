import SwiftUI

/// Approach B: the screens are SwiftUI built from vornui's semantic trees; the terminal stays drawn.
@main
struct NativeApp: App {
    var body: some Scene {
        WindowGroup {
            Paired { core in Root(core: core) }
                .preferredColorScheme(.dark)
                .tint(Palette.ink)
        }
    }
}

enum Palette {
    static let ink = Color(white: 0.93)
    static let bronze = Color(red: 0.83, green: 0.62, blue: 0.24)
    static let bg = Color(red: 0x0b / 255, green: 0x0b / 255, blue: 0x0b / 255)

    static func tone(_ t: String?) -> Color {
        switch t {
        case "ok", "success": .green
        case "warn", "warning", "attention": bronze
        case "danger", "error": .red
        case "info": .blue
        default: .secondary
        }
    }
}

struct Root: View {
    @ObservedObject var core: Core
    @State private var sessions: [String] = []
    @State private var settings = false

    var body: some View {
        TabView {
            Tab("Sessions", systemImage: "display") {
                NavigationStack(path: $sessions) {
                    Screen(core: core, name: "sessions")
                        .navigationTitle("Sessions")
                        .toolbar { gear }
                        .navigationDestination(for: String.self) { SessionScreen(core: core, id: $0) }
                }
            }
            Tab("Tasks", systemImage: "checklist") {
                NavigationStack { Screen(core: core, name: "tasks").navigationTitle("Tasks").toolbar { gear } }
            }
            Tab("Workflows", systemImage: "point.3.connected.trianglepath.dotted") {
                NavigationStack { Screen(core: core, name: "workflows").navigationTitle("Workflows").toolbar { gear } }
            }
        }
        .tabBarMinimizeBehavior(.onScrollDown)
        .sheet(isPresented: $settings) {
            NavigationStack {
                Screen(core: core, name: "settings")
                    .navigationTitle("Settings")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar { Button("Done") { settings = false } }
                    .navigationDestination(for: String.self) { key in
                        Screen(core: core, name: "settings:\(key)")
                            .navigationTitle(core.tree("settings")?.walk()
                                .first { $0.value == "settings:\(key)" }?.walk()
                                .first { $0.k == "text" }?.label ?? "")
                            .navigationBarTitleDisplayMode(.inline)
                    }
            }
        }
        .onAppear {
            Report.start(core, label: "native")
            if Bench.wanted {
                Bench(core: core, label: "native", drawnList: false) { sessions = [$0] }.start()
            }
        }
    }

    private var gear: some ToolbarContent {
        ToolbarItem(placement: .topBarTrailing) {
            Button { settings = true } label: { Image(systemName: "gearshape") }
                .accessibilityLabel("Settings")
        }
    }
}

/// One screen's tree as a List, re-read whenever vornd sends something.
struct Screen: View {
    @ObservedObject var core: Core
    let name: String

    var body: some View {
        let _ = core.revision
        let root = core.tree(name)
        List {
            if let root { Rows(core: core, screen: name, nodes: root.kids) }
        }
        .listStyle(.insetGrouped)
        .overlay {
            if let root, root.kids.allSatisfy({ $0.k == "column" && $0.kids.isEmpty }) || root.kids.isEmpty {
                ContentUnavailableView("Nothing here yet", systemImage: "tray")
            }
        }
    }
}

/// Nodes as List content: sections become Sections, lists and columns their rows.
struct Rows: View {
    let core: Core
    let screen: String
    let nodes: [Node]

    var body: some View {
        ForEach(nodes) { n in
            switch n.k {
            case "section":
                Section(n.label ?? "") { Rows(core: core, screen: screen, nodes: n.kids) }
            case "list", "column" where n.kids.contains(where: Self.isRow):
                Rows(core: core, screen: screen, nodes: n.kids)
            case "divider":
                EmptyView()
            default:
                Control(core: core, screen: screen, node: n)
            }
        }
    }

    static func isRow(_ n: Node) -> Bool {
        ["section", "list", "link", "toggle", "choice", "field", "row"].contains(n.k)
    }
}

/// One node as a control or a row's content.
struct Control: View {
    let core: Core
    let screen: String
    let node: Node
    @State private var text = ""
    @State private var asking = false

    private func act(_ extra: [String: Any] = [:]) {
        guard let id = node.id_ else { return }
        core.act(screen, ["id": id].merging(extra) { $1 })
    }

    var body: some View {
        switch node.k {
        case "link":
            NavigationLink(value: String(node.value?.split(separator: ":", maxSplits: 1).last ?? "")) {
                HStack(spacing: 12) { inline(node.kids) }
            }
        case "toggle":
            Toggle(node.label ?? "", isOn: Binding(get: { node.on ?? false }, set: { _ in act() }))
        case "choice":
            Picker("", selection: Binding(
                get: { node.kids.first { $0.on == true }?.id_ ?? "" },
                set: { id in core.act(screen, ["id": id]) }
            )) {
                ForEach(node.kids) { Text($0.label ?? "").tag($0.id_ ?? "") }
            }
            .pickerStyle(.segmented)
        case "field":
            Group {
                if node.secret == true {
                    SecureField(node.label ?? "", text: $text)
                } else {
                    TextField(node.label ?? "", text: $text)
                }
            }
            .onAppear { text = node.value ?? "" }
            .onSubmit { act(["value": text, "submit": true]) }
            .textInputAutocapitalization(.never)
        case "button":
            Button(role: node.style == "danger" ? .destructive : nil) {
                if node.confirm != nil { asking = true } else { act() }
            } label: {
                if let icon = node.value, !icon.isEmpty, (node.label ?? "").isEmpty {
                    Image(systemName: Symbols.of(icon))
                } else {
                    Text(node.label ?? "")
                }
            }
            .buttonStyle(.borderless)
            .confirmationDialog(node.confirm ?? "", isPresented: $asking, titleVisibility: .visible) {
                Button(node.label ?? "OK", role: .destructive) { act(["confirmed": true]) }
            }
        case "more":
            Menu {
                ForEach(node.kids) { Control(core: core, screen: screen, node: $0) }
            } label: {
                Image(systemName: "ellipsis").frame(minWidth: 44, minHeight: 44)
            }
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
            case "text": Leaf.text(n)
            case "icon": Image(systemName: Symbols.of(n.value ?? "")).foregroundStyle(.secondary)
            case "status": Circle().fill(Palette.tone(n.style)).frame(width: 8, height: 8)
                .accessibilityLabel(n.value ?? "")
            case "badge": Text(n.value ?? "").font(.caption2.weight(.semibold))
                .padding(.horizontal, 6).padding(.vertical, 2)
                .background(Palette.tone(n.style).opacity(0.2), in: Capsule())
            case "column": VStack(alignment: .leading, spacing: 2) { inline(n.kids) }
            case "row": HStack(spacing: 8) { inline(n.kids) }
            case "drawn", "divider": EmptyView()
            default: Control(core: core, screen: screen, node: n)
            }
            if n.k == "text" && n.style == "body" && nodes.count > 1 && n.id == nodes.first?.id { Spacer(minLength: 8) }
        })
    }
}

enum Leaf {
    @ViewBuilder static func text(_ n: Node) -> some View {
        let s = n.label ?? ""
        switch n.style {
        case "title": Text(s).font(.body.weight(.medium)).lineLimit(1)
        case "secondary": Text(s).font(.footnote).foregroundStyle(.secondary).lineLimit(1)
        case "mono": Text(s).font(.footnote.monospaced()).foregroundStyle(.secondary)
        default: Text(s)
        }
    }
}

enum Symbols {
    /// The app's icon names to the system's.
    static func of(_ name: String) -> String {
        [
            "monitor": "display", "list-todo": "checklist", "workflow": "point.3.connected.trianglepath.dotted",
            "settings": "gearshape", "chevron-right": "chevron.right", "terminal": "terminal",
            "circle": "circle", "check": "checkmark", "x": "xmark", "play": "play.fill", "pause": "pause.fill",
            "ellipsis": "ellipsis", "puzzle": "puzzlepiece.extension", "bot": "cpu", "git-branch": "arrow.triangle.branch",
        ][name] ?? "circle.dashed"
    }
}

/// A session: a native header and composer around the drawn terminal.
struct SessionScreen: View {
    @ObservedObject var core: Core
    let id: String
    @State private var draft = ""
    @State private var ctrl = false
    @FocusState private var composing: Bool

    var body: some View {
        let _ = core.revision
        let head = core.tree("session:\(id)")?.walk()
        VStack(spacing: 0) {
            Metal(core: core, terminalOnly: true)
                .background(Palette.bg)
            KeyBar(core: core, ctrl: $ctrl)
            HStack(spacing: 8) {
                TextField("Message", text: $draft, axis: .vertical)
                    .lineLimit(1...5)
                    .focused($composing)
                    .textInputAutocapitalization(.never)
                    .padding(.horizontal, 14).padding(.vertical, 10)
                    .background(Color(white: 0.12), in: RoundedRectangle(cornerRadius: 20))
                Button {
                    vp_send(core.handle, draft)
                    draft = ""
                } label: {
                    Image(systemName: "arrow.up").font(.body.weight(.semibold))
                        .frame(width: 36, height: 36)
                        .background(draft.isEmpty ? Color(white: 0.2) : Palette.bronze, in: Circle())
                        .foregroundStyle(.black)
                }
                .disabled(draft.isEmpty)
                .accessibilityLabel("Send")
            }
            .padding(.horizontal, 12).padding(.vertical, 8)
        }
        .background(Palette.bg)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .principal) {
                HStack(spacing: 6) {
                    Text(head?.first { $0.k == "text" && $0.style == "title" }?.label ?? "Session")
                        .font(.headline).lineLimit(1)
                    if let s = head?.first(where: { $0.k == "status" }) {
                        Circle().fill(Palette.tone(s.style)).frame(width: 8, height: 8)
                    }
                }
            }
        }
        .onAppear { vp_open_session(core.handle, id) }
        .onDisappear { vp_open_session(core.handle, nil) }
    }
}

/// The keys a phone keyboard lacks, as on the drawn app's bar.
struct KeyBar: View {
    let core: Core
    @Binding var ctrl: Bool

    private let keys: [(String, String, String)] = [
        ("esc", "Escape", ""), ("tab", "Tab", ""), ("^C", "KeyC", "c"), ("^D", "KeyD", "d"), ("^L", "KeyL", "l"),
        ("↑", "ArrowUp", ""), ("↓", "ArrowDown", ""), ("←", "ArrowLeft", ""), ("→", "ArrowRight", ""),
    ]

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                Button("ctrl") { ctrl.toggle() }
                    .buttonStyle(KeyStyle(on: ctrl))
                ForEach(keys, id: \.0) { k in
                    Button(k.0) {
                        let mods: UInt16 = k.2.isEmpty ? (ctrl ? 4 : 0) : 4
                        vp_key(core.handle, k.1, mods)
                        ctrl = false
                    }
                    .buttonStyle(KeyStyle(on: false))
                }
            }
            .padding(.horizontal, 12).padding(.vertical, 6)
        }
    }
}

struct KeyStyle: ButtonStyle {
    let on: Bool
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.footnote.monospaced().weight(.medium))
            .frame(minWidth: 44, minHeight: 36)
            .padding(.horizontal, 4)
            .background(on || configuration.isPressed ? Color(white: 0.28) : Color(white: 0.14),
                        in: RoundedRectangle(cornerRadius: 8))
            .foregroundStyle(Palette.ink)
    }
}
