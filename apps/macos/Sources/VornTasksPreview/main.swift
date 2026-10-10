import AppKit
import SwiftUI
import VornCore
import VornTasks
import VornUI

// A window that hosts TasksView on its own, against a test vornd or a sample board.
//
//   VornTasksPreview --sample
//   VornTasksPreview --data-dir DIR --seed           (fills a test vornd with the sample board)
//   VornTasksPreview --data-dir DIR                  (read-only when DIR is the app's own ~/.vorn)
//   VornTasksPreview --data-dir DIR --render out.png [--state list|kanban|dialog|detail|options] [--size 1200x700]

struct Options {
    var dataDir: URL?
    var sample = false
    var seed = false
    var renderPath: String?
    var state = "list"
    var size = CGSize(width: 1200, height: 700)

    init(_ args: [String]) {
        var it = args.dropFirst().makeIterator()
        while let a = it.next() {
            switch a {
            case "--data-dir": dataDir = it.next().map { URL(fileURLWithPath: ($0 as NSString).expandingTildeInPath) }
            case "--sample": sample = true
            case "--seed": seed = true
            case "--render": renderPath = it.next()
            case "--state": state = it.next() ?? state
            case "--size":
                let parts = (it.next() ?? "").split(separator: "x").compactMap { Double($0) }
                if parts.count == 2 { size = CGSize(width: parts[0], height: parts[1]) }
            default: break
            }
        }
        if dataDir == nil { sample = true }
    }
}

let options = Options(CommandLine.arguments)

/// The service the options name; the app's own data directory is only ever read.
func makeService() async throws -> any TaskService {
    guard let dir = options.dataDir, !options.sample else { return InMemoryTaskService.sample() }
    let own = VornEndpoint.defaultDataDirectory.standardizedFileURL.resolvingSymlinksInPath()
    let readOnly = dir.standardizedFileURL.resolvingSymlinksInPath() == own
    let client = VornClient(endpoint: try VornEndpoint.read(dataDirectory: dir), readOnly: readOnly)
    try await client.connect()
    if readOnly { FileHandle.standardError.write(Data("connected read-only to \(dir.path)\n".utf8)) }
    return client
}

/// Copies the sample board into a test vornd: its projects into the config, then each task.
func seed(_ dir: URL) async throws {
    let client = VornClient(endpoint: try VornEndpoint.read(dataDirectory: dir))
    try await client.connect()
    let sample = InMemoryTaskService.sample()
    guard case .object(var config) = try await client.call("config:load") else { throw VornError.badResponse("config:load") }
    var projects = config["projects"]?.arrayValue ?? []
    let existing = Set(projects.compactMap { $0["name"]?.stringValue })
    for p in try await sample.listProjects() where !existing.contains(p.name) {
        let path = dir.appendingPathComponent("projects/\(p.name)")
        try FileManager.default.createDirectory(at: path, withIntermediateDirectories: true)
        var entry: [String: JSONValue] = ["name": .string(p.name), "path": .string(path.path), "preferredAgents": .array([])]
        if let icon = p.icon { entry["icon"] = .string(icon) }
        if let color = p.iconColor { entry["iconColor"] = .string(color) }
        projects.append(.object(entry))
    }
    config["projects"] = .array(projects)
    try await client.call("config:save", .object(config))
    for t in try await sample.listTasks().sorted(by: { $0.order < $1.order }) {
        let created = try await client.createTask(TaskDraft(
            projectName: t.projectName, title: t.title, description: t.description, status: t.status,
            assignedAgent: t.assignedAgent))
        if t.isArchived { try await client.archiveTask(id: created.id, archived: true) }
    }
    print("seeded \(dir.path)")
}

/// Stand-in for the shell's top bar: the window buttons' room and the view options button.
struct PreviewChrome: View {
    let store: TasksStore
    var fauxLights = false
    @State private var toast: String?

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                if fauxLights {
                    HStack(spacing: 8) {
                        ForEach([0xFF5F57, 0xFEBC2E, 0x28C840], id: \.self) {
                            Circle().fill(Color(hex: UInt32($0))).frame(width: 12, height: 12)
                        }
                    }
                    .padding(.leading, 14)
                }
                Spacer()
                TaskViewOptionsButton(store: store)
            }
            .padding(.trailing, 12)
            .frame(height: Theme.toolbarHeight)
            .background(Theme.surfaceBase)
            TasksView(store: store)
        }
        .overlay(alignment: .bottom) {
            if let toast {
                Text(toast).font(.system(size: 12)).foregroundStyle(Theme.ink)
                    .padding(.horizontal, 14).padding(.vertical, 8)
                    .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusLg))
                    .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.1)))
                    .padding(.bottom, 20)
                    .transition(.opacity)
            }
        }
        .onAppear {
            store.onToast = { message, _ in
                withAnimation { toast = message }
                Task { @MainActor in
                    try? await Task.sleep(for: .seconds(2))
                    withAnimation { if toast == message { toast = nil } }
                }
            }
        }
        .preferredColorScheme(.dark)
    }
}

struct PreviewApp: App {
    @State private var store: TasksStore?
    @State private var failure: String?

    var body: some Scene {
        WindowGroup {
            Group {
                if let store {
                    PreviewChrome(store: store)
                } else {
                    Text(failure ?? "Connecting…").foregroundStyle(Theme.inkFaint)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
            .frame(minWidth: 900, minHeight: 500)
            .background(Theme.surfaceBase)
            .ignoresSafeArea()
            .background(WindowConfigurator())
            .task {
                do {
                    store = TasksStore(service: try await makeService())
                } catch {
                    failure = "Could not reach vornd: \(error.localizedDescription)"
                }
            }
        }
        .windowStyle(.hiddenTitleBar)
        .defaultSize(options.size)
    }
}

/// The hidden titlebar the native look spike uses: buttons centred on the 40pt bar.
private struct WindowConfigurator: NSViewRepresentable {
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

/// Draws one state of the board offscreen and writes it as a PNG.
@MainActor
func render(to path: String) async -> Int32 {
    _ = NSApplication.shared
    let store: TasksStore
    do {
        store = TasksStore(service: try await makeService())
    } catch {
        FileHandle.standardError.write(Data("could not reach vornd: \(error)\n".utf8))
        return 1
    }
    await store.reload()
    switch options.state {
    case "kanban": store.showViewMode(.kanban)
    case "list": store.showViewMode(.list)
    case "dialog": store.openNewTask()
    case "detail": store.selectedTaskId = store.tasks.first { $0.status == .inProgress }?.id ?? store.tasks.first?.id
    case "options": store.viewOptionsOpen = true
    default: break
    }
    let size = options.size
    let content = PreviewChrome(store: store, fauxLights: true)
        .frame(width: size.width, height: size.height)
        .background(Theme.surfaceBase)
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Color.white.opacity(0.1), lineWidth: 0.5))
        .environment(\.colorScheme, .dark)

    // ImageRenderer leaves scroll views and text fields blank, so the board is drawn through a hosting view.
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
    let image = bitmap(of: host, scale: 2)
    guard let cgImage = image,
          let png = NSBitmapImageRep(cgImage: cgImage).representation(using: .png, properties: [:]) else {
        FileHandle.standardError.write(Data("render failed\n".utf8))
        return 1
    }
    do {
        try png.write(to: URL(fileURLWithPath: path))
    } catch {
        FileHandle.standardError.write(Data("\(error)\n".utf8))
        return 1
    }
    print("wrote \(path) (\(cgImage.width)x\(cgImage.height))")
    return 0
}

/// Draws `view` at `scale` regardless of the screen the window would land on.
@MainActor
func bitmap(of view: NSView, scale: CGFloat) -> CGImage? {
    let bounds = view.bounds
    guard let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: Int(bounds.width * scale), pixelsHigh: Int(bounds.height * scale),
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
    ) else { return nil }
    rep.size = bounds.size
    view.cacheDisplay(in: bounds, to: rep)
    return rep.cgImage
}

if options.seed {
    guard let dir = options.dataDir,
          dir.standardizedFileURL.resolvingSymlinksInPath()
            != VornEndpoint.defaultDataDirectory.standardizedFileURL.resolvingSymlinksInPath() else {
        FileHandle.standardError.write(Data("--seed needs the --data-dir of a test vornd\n".utf8))
        exit(2)
    }
    Task {
        do { try await seed(dir); exit(0) } catch {
            FileHandle.standardError.write(Data("seed failed: \(error)\n".utf8))
            exit(1)
        }
    }
    RunLoop.main.run()
} else if let path = options.renderPath {
    Task { @MainActor in exit(await render(to: path)) }
    RunLoop.main.run()
} else {
    PreviewApp.main()
}
