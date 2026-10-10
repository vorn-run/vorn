import AppKit
import SwiftUI
import VornCore
import VornTerminals
import VornUI
import VornWorkflows

public let defaultWindowSize = CGSize(width: 1280, height: 800)

/// The app: one window onto the local vornd.
public struct VornMacApp: App {
    @State private var store: VornStore
    @State private var features: FeatureStores
    @State private var model = AppModel()
    @State private var engines = EngineHolder()

    public init() {
        let store = VornStore()
        _store = State(initialValue: store)
        _features = State(initialValue: FeatureStores(client: store.client))
    }

    public var body: some Scene {
        WindowGroup("Vorn") {
            RootView(model: model, store: store, engines: engines, features: features)
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
/// terminals included. Options: `--size WxH`, `--project NAME`, `--no-sidebar`, `--tip N`,
/// `--view sessions|tasks|workflows`.
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
    let features = FeatureStores(client: store.client)
    let model = AppModel()
    let engines = EngineHolder()
    if let view = value("--view") {
        guard let mode = MainViewMode(rawValue: view) else {
            FileHandle.standardError.write(Data("unknown view \(view)\n".utf8))
            return 2
        }
        model.mainViewMode = mode
    }
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

    features.bind(store: store, model: model)
    switch model.mainViewMode {
    case .sessions: break
    case .tasks:
        features.tasks.scope = features.scope(model, store)
        await features.tasks.reload()
    case .workflows:
        await features.workflows.reload()
    }

    let visible = model.mainViewMode == .sessions ? model.visibleSessions(store) : []
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

    let content = RootView(model: model, store: store, engines: engines, features: features, fauxChrome: true)
        .environment(\.terminalSnapshot, true)
        .environment(\.workflowsStaticLayout, true)
        .frame(width: size.width, height: size.height)
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Color.white.opacity(0.1), lineWidth: 0.5))
    // ImageRenderer leaves the board's scroll views blank, so Tasks is drawn through a hosting view.
    let image = model.mainViewMode == .tasks ? await hostedImage(of: content, size: size) : {
        let renderer = ImageRenderer(content: content)
        renderer.scale = 2
        renderer.isOpaque = false
        return renderer.cgImage
    }()
    guard let image,
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
    features.tasks.stop()
    features.workflows.stop()
    store.stop()
    print("wrote \(path) (\(image.width)x\(image.height)), \(visible.count) sessions")
    return 0
}

/// Draws `content` in an offscreen window at 2x, for views ImageRenderer cannot draw.
@MainActor
private func hostedImage(of content: some View, size: CGSize) async -> CGImage? {
    let host = NSHostingView(rootView: content)
    host.frame = CGRect(origin: .zero, size: size)
    let window = NSWindow(contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: false)
    window.appearance = NSAppearance(named: .darkAqua)
    window.backgroundColor = .clear
    window.isOpaque = false
    window.contentView = host
    host.layoutSubtreeIfNeeded()
    try? await Task.sleep(for: .milliseconds(400))
    host.layoutSubtreeIfNeeded()
    guard let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: Int(size.width * 2), pixelsHigh: Int(size.height * 2),
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
    ) else { return nil }
    rep.size = size
    host.cacheDisplay(in: host.bounds, to: rep)
    return rep.cgImage
}
