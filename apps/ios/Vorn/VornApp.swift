import SwiftUI

/// Approach B: the screens are SwiftUI built from vornui's semantic trees; the terminal stays drawn.
@main
struct NativeApp: App {
    var body: some Scene {
        WindowGroup {
            Paired { core, computers in Root(core: core, computers: computers) }
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
        case "good": .green
        case "warn", "busy": bronze
        case "bad": .red
        default: .secondary
        }
    }
}

/// The app's three places.
enum Place: String, CaseIterable, Identifiable {
    case sessions, tasks, workflows
    var id: String { rawValue }
    var title: String { rawValue.capitalized }
    var icon: String {
        switch self {
        case .sessions: "monitor"
        case .tasks: "list-todo"
        case .workflows: "workflow"
        }
    }
}

/// Tabs on a phone, a sidebar, list and detail on a tablet or a wide window.
struct Root: View {
    @ObservedObject var core: Core
    let computers: Computers
    @Environment(\.horizontalSizeClass) private var width
    @State private var place: Place = .sessions
    @State private var settings = false
    @StateObject private var tabs: TabNavs
    @StateObject private var list: Nav
    @StateObject private var detail: Nav
    @StateObject private var prefs: Nav

    init(core: Core, computers: Computers) {
        self.core = core
        self.computers = computers
        _tabs = StateObject(wrappedValue: TabNavs(core: core))
        _list = StateObject(wrappedValue: Nav(core: core))
        _detail = StateObject(wrappedValue: Nav(core: core))
        _prefs = StateObject(wrappedValue: Nav(core: core))
    }

    var body: some View {
        Group {
            if width == .regular { split } else { phone }
        }
        .sheet(isPresented: $settings) {
            Settings(core: core, nav: prefs, computers: computers, done: { settings = false })
        }
        .background { shortcuts }
        .onAppear {
            list.detail = detail
            Report.start(core, label: "native")
            if Bench.wanted {
                Bench(core: core, label: "native") { open("session:\($0)") }.start()
            }
        }
    }

    private var phone: some View {
        TabView(selection: $place) {
            ForEach(Place.allCases) { p in
                Tab(p.title, image: p.icon, value: p) {
                    Stack(core: core, nav: tabs.of(p)) {
                        Screen(core: core, name: p.rawValue).toolbar { gear }
                    }
                }
                .badge(p == .workflows ? waiting : 0)
            }
        }
        .tabBarMinimizeBehavior(.onScrollDown)
    }

    private var split: some View {
        NavigationSplitView {
            List(selection: Binding(get: { place }, set: { if let p = $0 { go(p) } })) {
                ForEach(Place.allCases) { p in
                    Label(p.title, image: p.icon).tag(p)
                        .badge(p == .workflows ? waiting : 0)
                }
            }
            .navigationTitle("Vorn")
            .toolbar { gear }
        } content: {
            Stack(core: core, nav: list) {
                Screen(core: core, name: place.rawValue)
            }
            .id(place)
            .navigationSplitViewColumnWidth(min: 320, ideal: 380)
        } detail: {
            Stack(core: core, nav: detail) {
                ContentUnavailableView("Nothing open", image: place.icon,
                                       description: Text("Pick something from the list to see it here."))
            }
        }
    }

    /// Runs waiting on an approval, for the Workflows badge.
    private var waiting: Int {
        let _ = core.revision
        return Int(core.tree("workflows")?.walk().first { $0.k == "option" && $0.label?.hasPrefix("Review ") == true }?
            .label?.dropFirst("Review ".count) ?? "") ?? 0
    }

    private func go(_ p: Place) {
        place = p
        detail.path = []
    }

    private func open(_ route: String) {
        if width == .regular {
            go(.sessions)
            detail.path = [route]
        } else {
            place = .sessions
            tabs.of(.sessions).path = [route]
        }
    }

    /// A hardware keyboard's shortcuts.
    private var shortcuts: some View {
        Group {
            Button("Sessions") { width == .regular ? go(.sessions) : (place = .sessions) }.keyboardShortcut("1")
            Button("Tasks") { width == .regular ? go(.tasks) : (place = .tasks) }.keyboardShortcut("2")
            Button("Workflows") { width == .regular ? go(.workflows) : (place = .workflows) }.keyboardShortcut("3")
            Button("New session") { open("launch") }.keyboardShortcut("n")
            Button("Settings") { settings = true }.keyboardShortcut(",")
        }
        .opacity(0)
        .frame(width: 0, height: 0)
        .accessibilityHidden(true)
    }

    private var gear: some ToolbarContent {
        ToolbarItem(placement: .topBarLeading) {
            Button { settings = true } label: { VornIcon(name: "settings") }
                .accessibilityLabel("Settings")
        }
    }
}

/// Each tab's stack, kept while the app runs.
final class TabNavs: ObservableObject {
    private var navs: [Place: Nav]

    init(core: Core) {
        navs = Dictionary(uniqueKeysWithValues: Place.allCases.map { ($0, Nav(core: core)) })
    }

    func of(_ p: Place) -> Nav { navs[p]! }
}

struct Settings: View {
    let core: Core
    @ObservedObject var nav: Nav
    @ObservedObject var computers: Computers
    let done: () -> Void
    @State private var forgetting: Computer?
    @State private var adding = false

    var body: some View {
        Stack(core: core, nav: nav) {
            Screen(core: core, name: "settings", title: "Settings", extra: AnyView(machines))
                .navigationBarTitleDisplayMode(.inline)
                .toolbar { Button("Done", action: done) }
        }
        .sheet(isPresented: $adding) { PairFlow(computers: computers) { adding = false } }
    }

    private var machines: some View {
        Section("Computers") {
            ForEach(computers.list) { c in
                Button { computers.select(c.id) } label: {
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(c.name).foregroundStyle(Palette.ink)
                            Text(Pairing.hostOf(c.host)).font(.caption).fontDesign(.monospaced).foregroundStyle(.secondary)
                        }
                        Spacer()
                        if c.id == computers.active { VornIcon(name: "check", ink: Palette.bronze) }
                    }
                }
                .swipeActions { Button("Forget", role: .destructive) { forgetting = c } }
            }
            Button("Pair another computer") { adding = true }
            if let c = computers.current {
                Button("Forget this computer", role: .destructive) { forgetting = c }
            }
        }
        .confirmationDialog("Forget \(forgetting?.name ?? "this computer")?", isPresented: Binding(
            get: { forgetting != nil }, set: { if !$0 { forgetting = nil } }
        ), titleVisibility: .visible, presenting: forgetting) { c in
            Button("Forget", role: .destructive) { forget(c) }
        } message: { _ in
            Text("This phone gives up its token. You'll need a new pairing code from Settings › Remote Access to connect again.")
        }
    }

    /// Tells the computer in use to revoke this phone's token, then drops it here.
    private func forget(_ c: Computer) {
        guard c.id == computers.active, let token = computers.token(c), let id = Computers.tokenId(token),
              let params = try? JSONSerialization.data(withJSONObject: id, options: .fragmentsAllowed) else {
            return computers.remove(c.id)
        }
        vp_call(core.handle, "token:revoke", String(decoding: params, as: UTF8.self))
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [core] in
            _ = core
            computers.remove(c.id)
        }
    }
}

/// A session: a native header, its open requests and a composer around the drawn terminal.
struct SessionScreen: View {
    @ObservedObject var core: Core
    @EnvironmentObject var nav: Nav
    let id: String
    @State private var draft = ""
    @State private var ctrl = false
    @FocusState private var composing: Bool

    private var screen: String { "session:\(id)" }

    var body: some View {
        let _ = core.revision
        let top = core.tree(screen)?.top ?? []
        let bar = top.first { $0.k == "bar" }
        let asks = top.filter { $0.k == "section" }
        VStack(spacing: 0) {
            ForEach(asks) { Ask(screen: screen, node: $0) }
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
                    VornIcon(name: "arrow-up")
                        .frame(width: 44, height: 44)
                        .background(draft.isEmpty ? Color(white: 0.2) : Palette.bronze, in: Circle())
                        .foregroundStyle(.black)
                }
                .disabled(draft.isEmpty)
                .keyboardShortcut(.return, modifiers: .command)
                .accessibilityLabel("Send")
            }
            .padding(.horizontal, 12).padding(.vertical, 8)
        }
        .background(Palette.bg)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .principal) {
                HStack(spacing: 6) {
                    if let mark = top.first(where: { $0.k == "icon" }) { VornIcon(mark) }
                    Text(bar?.label ?? "Session").font(.headline).lineLimit(1)
                    if let s = top.first(where: { $0.k == "status" }) {
                        Circle().fill(Palette.tone(s.style)).frame(width: 8, height: 8)
                            .accessibilityLabel(s.value ?? "")
                    }
                }
            }
            if let more = bar?.kids.first(where: { $0.k == "more" }) {
                ToolbarItem(placement: .topBarTrailing) { More(screen: screen, node: more) }
            }
        }
        .onAppear { vp_open_session(core.handle, id) }
        .onDisappear { vp_open_session(core.handle, nil) }
    }
}

/// A permission an agent asks for, answered above its terminal.
struct Ask: View {
    let screen: String
    let node: Node

    var body: some View {
        let texts = node.kids.filter { $0.k == "text" }
        let buttons = node.kids.first { $0.k == "row" }?.kids ?? []
        VStack(alignment: .leading, spacing: 8) {
            ForEach(texts) { Leaf.text($0) }
            HStack(spacing: 10) {
                ForEach(buttons) { b in
                    Control(screen: screen, node: b)
                        .buttonStyle(.bordered)
                        .tint(b.style == "danger" ? .red : Palette.bronze)
                        .frame(minHeight: 44)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .background(Palette.bronze.opacity(0.12), in: RoundedRectangle(cornerRadius: 12))
        .padding(.horizontal, 12).padding(.top, 8)
    }
}

/// The keys a phone keyboard lacks.
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
