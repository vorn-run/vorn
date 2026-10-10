import AppKit
import SwiftUI
import VornCore
import VornTerminals
import VornUI

public let defaultWindowSize = CGSize(width: 1280, height: 800)

/// The app: one window onto the local vornd.
public struct VornMacApp: App {
    @State private var store = VornStore()
    @State private var model = AppModel()
    @State private var engines = EngineHolder()

    public init() {}

    public var body: some Scene {
        WindowGroup("Vorn") {
            RootView(model: model, store: store, engines: engines)
                .frame(minWidth: 800, minHeight: 500)
                .ignoresSafeArea()
                .background(WindowConfigurator())
                .onAppear { store.start() }
        }
        .windowStyle(.hiddenTitleBar)
        .defaultSize(defaultWindowSize)
        .commands { ShellCommands(model: model, store: store) }
    }

    /// Runs the app, or `--render <png>` writes the main screen and exits.
    @MainActor
    public static func run() {
        let args = CommandLine.arguments
        if let i = args.firstIndex(of: "--render"), i + 1 < args.count {
            _ = NSApplication.shared
            Task { @MainActor in
                let code = await renderMainScreen(to: args[i + 1], arguments: args)
                exit(code)
            }
            RunLoop.main.run()
        }
        main()
    }
}

struct ShellCommands: Commands {
    let model: AppModel
    let store: VornStore

    private var actions: ShellActions { ShellActions(store: store, model: model) }

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("New Session") { actions.newSession(actions.activeProject, actions.activeWorktree) }
                .keyboardShortcut("n")
            Button("New Terminal") {
                actions.newTerminal(actions.activeProject ?? store.projects.first, actions.activeWorktree)
            }
            .keyboardShortcut("`", modifiers: .control)
        }
        CommandGroup(after: .sidebar) {
            Button("Toggle Sidebar") { model.sidebarOpen.toggle() }.keyboardShortcut("b")
            Divider()
            Button("Sessions") { model.mainViewMode = .sessions }.keyboardShortcut("s")
            Button("Tasks") { model.mainViewMode = .tasks }.keyboardShortcut("t")
            Button("Workflows") { model.mainViewMode = .workflows }.keyboardShortcut("w", modifiers: [.command, .shift])
            Divider()
            ForEach(1..<10) { n in
                Button("Session \(n)") {
                    let visible = model.visibleSessions(store)
                    if n <= visible.count { model.focus(visible[n - 1].id) }
                }
                .keyboardShortcut(KeyEquivalent(Character("\(n)")))
            }
        }
    }
}

/// Gives the hidden titlebar the height of the 40pt top bar, so the window
/// buttons sit centred on it.
struct WindowConfigurator: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        DispatchQueue.main.async {
            guard let window = view.window else { return }
            window.titleVisibility = .hidden
            window.titlebarAppearsTransparent = true
            window.styleMask.insert(.fullSizeContentView)
            window.toolbar = NSToolbar(identifier: "main")
            window.toolbarStyle = .unifiedCompact
            window.backgroundColor = NSColor(red: 0x0D / 255, green: 0x0D / 255, blue: 0x0F / 255, alpha: 1)
            window.appearance = NSAppearance(named: .darkAqua)
        }
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {}
}

/// Renders the main screen offscreen with live data from vornd (read-only),
/// terminals included. Options: `--size WxH`, `--project NAME`, `--no-sidebar`, `--tip N`.
@MainActor
func renderMainScreen(to path: String, arguments args: [String]) async -> Int32 {
    func value(_ flag: String) -> String? {
        args.firstIndex(of: flag).flatMap { $0 + 1 < args.count ? args[$0 + 1] : nil }
    }
    var size = defaultWindowSize
    if let s = value("--size")?.split(separator: "x"), s.count == 2, let w = Double(s[0]), let h = Double(s[1]) {
        size = CGSize(width: w, height: h)
    }

    let store = VornStore(readOnly: true)
    let model = AppModel()
    let engines = EngineHolder()
    model.sidebarOpen = !args.contains("--no-sidebar")
    model.activeProject = value("--project")
    if let tip = value("--tip").flatMap(Int.init) { model.launcherTip = tip }
    store.start()
    let deadline = Date().addingTimeInterval(8)
    while store.phase != .connected && Date() < deadline { try? await Task.sleep(for: .milliseconds(50)) }
    guard store.phase == .connected, let endpoint = store.endpoint else {
        FileHandle.standardError.write(Data("vornd not reachable: \(store.phase)\n".utf8))
        return 1
    }
    try? await Task.sleep(for: .milliseconds(300))
    engines.update(socket: endpoint.gridSocket, readOnly: true)

    let visible = model.visibleSessions(store)
    if let engine = engines.engine, !visible.isEmpty {
        let gridW = size.width - (model.sidebarOpen ? model.sidebarWidth : 0)
        let gridH = size.height - Theme.toolbarHeight
        let layout = AutoLayout.pick(visible.count, width: gridW, height: gridH)
        let cardH = layout.mode == .fit ? gridH / CGFloat(layout.rows) : AutoLayout.scrollRowHeight(gridH)
        let body = CGSize(width: gridW / CGFloat(layout.cols), height: cardH - 41 - 22 - 2)
        for s in visible { engine.attachSnapshot(s.id, size: body) }
        await engine.settle()
        if args.contains("--dump-text") {
            for s in visible { print("--- \(s.title)\n\(engine.snapshotText(s.id) ?? "")") }
        }
    }

    let content = RootView(model: model, store: store, engines: engines, fauxChrome: true)
        .environment(\.terminalSnapshot, true)
        .frame(width: size.width, height: size.height)
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Color.white.opacity(0.1), lineWidth: 0.5))
    let renderer = ImageRenderer(content: content)
    renderer.scale = 2
    renderer.isOpaque = false
    guard let image = renderer.cgImage,
          let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else {
        FileHandle.standardError.write(Data("render failed\n".utf8))
        return 1
    }
    do {
        try png.write(to: URL(fileURLWithPath: path))
    } catch {
        FileHandle.standardError.write(Data("\(error)\n".utf8))
        return 1
    }
    engines.engine?.shutdown()
    store.stop()
    print("wrote \(path) (\(image.width)x\(image.height)), \(visible.count) sessions")
    return 0
}
