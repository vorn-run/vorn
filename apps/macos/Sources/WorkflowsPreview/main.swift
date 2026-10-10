import AppKit
import SwiftUI
import VornCore
import VornUI
import VornWorkflows

/// Where the data comes from: a vornd's data directory, or captured JSON.
enum Source {
    case vornd(URL, VorndClient.Access)
    case fixtures(URL)

    @MainActor
    func backend() async throws -> any WorkflowsBackend {
        switch self {
        case .vornd(let dir, let access):
            return try await VorndWorkflowsBackend.connect(dataDirectory: dir, access: access)
        case .fixtures(let dir):
            return try FixtureWorkflowsBackend(
                workflowsJSON: Data(contentsOf: dir.appendingPathComponent("workflows.json")),
                runsJSON: Data(contentsOf: dir.appendingPathComponent("runs.json")))
        }
    }
}

struct Options {
    var source = Source.vornd(VorndEndpoint.defaultDataDirectory, .readOnly)
    var render: String?
    var scene = "landing"
    var size = CGSize(width: 1280, height: 800)

    init(_ args: [String]) {
        var it = args.dropFirst().makeIterator()
        while let arg = it.next() {
            switch arg {
            case "--data-dir": if let v = it.next() { source = .vornd(URL(fileURLWithPath: v), .readWrite) }
            case "--fixtures": if let v = it.next() { source = .fixtures(URL(fileURLWithPath: v)) }
            case "--render": render = it.next()
            case "--scene": if let v = it.next() { scene = v }
            case "--size":
                let parts = it.next()?.split(separator: "x").compactMap { Double($0) } ?? []
                if parts.count == 2 { size = CGSize(width: parts[0], height: parts[1]) }
            default:
                FileHandle.standardError.write(Data("unknown argument \(arg)\n".utf8))
                exit(2)
            }
        }
    }
}

/// Stand-in window chrome, for the module alone: the sidebar with the Workflows section,
/// and the main top bar with the view pills and the Workflows controls.
struct PreviewShell: View {
    @Bindable var store: WorkflowsStore
    var fauxChrome = false

    var body: some View {
        HStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    if fauxChrome { FauxTrafficLights() }
                }
                .frame(height: Theme.toolbarHeight)
                .padding(.leading, 12)
                if fauxChrome {
                    WorkflowsSidebarSection(store: store).padding(.horizontal, 12)
                } else {
                    ScrollView { WorkflowsSidebarSection(store: store).padding(.horizontal, 12) }
                        .scrollIndicators(.never)
                }
            }
            .frame(width: 240)
            .frame(maxHeight: .infinity, alignment: .top)
            .background(Theme.surfacePanel)
            .overlay(alignment: .trailing) { Rectangle().fill(Theme.white(0.06)).frame(width: 1) }

            VStack(spacing: 0) {
                if store.editingWorkflow == nil {
                    HStack(spacing: 4) {
                        ToolbarGlyph(symbol: "sidebar.left")
                        ToolbarDivider()
                        ViewPills()
                        Spacer(minLength: 0)
                        WorkflowsHeader(store: store)
                    }
                    .padding(.horizontal, 12)
                    .frame(height: Theme.toolbarHeight)
                    .overlay(alignment: .bottom) { Rectangle().fill(Theme.white(0.06)).frame(height: 1) }
                }
                WorkflowsView(store: store)
            }
        }
        .background(Theme.surfaceBase)
        .workflowsMenuHost(store)
        .environment(\.colorScheme, .dark)
    }
}

private struct ToolbarGlyph: View {
    let symbol: String

    var body: some View {
        Image(systemName: symbol)
            .font(.system(size: 14))
            .foregroundStyle(Theme.gray400)
            .frame(width: 24, height: 24)
    }
}

/// The Sessions / Tasks / Workflows pills, Workflows active.
private struct ViewPills: View {
    var body: some View {
        HStack(spacing: 2) {
            pill("display", active: false)
            pill("checklist", active: false)
            pill("flowchart", active: true)
        }
        .padding(2)
        .background(Theme.white(0.04), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
    }

    private func pill(_ symbol: String, active: Bool) -> some View {
        Image(systemName: symbol)
            .font(.system(size: 12, weight: .medium))
            .frame(width: 14, height: 14)
            .foregroundStyle(active ? Color.white : Theme.gray500)
            .padding(.horizontal, 10)
            .padding(.vertical, 4)
            .background(active ? Theme.white(0.1) : .clear, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
    }
}

/// Puts the store in the state a scene shows.
@MainActor
func stage(_ store: WorkflowsStore, scene: String) async {
    await store.reload()
    switch scene {
    case "review":
        store.tab = .review
        store.selectedRunId = store.visibleRuns.first?.runId
    case "failed":
        store.selectedRunId = store.runs.first { $0.status == .error }?.runId
    case "page":
        let id = store.runs.first { $0.waitingStep != nil }?.workflowId ?? store.workflows.first?.id
        if let id { await store.openWorkflow(id).value }
    case "toast":
        if let wf = store.workflows.first { store.toast = "\"\(wf.name)\" has no trigger — add one in the editor first" }
    case "menu":
        if let wf = store.sidebarWorkflows.first {
            store.menu = WorkflowsMenu(kind: .workflow(wf.id), anchor: CGRect(x: 200, y: 160, width: 20, height: 20), edge: .leading)
        }
    default:
        break
    }
}

@MainActor
func render(_ options: Options, to path: String) async -> Int32 {
    _ = NSApplication.shared
    let store: WorkflowsStore
    do {
        store = WorkflowsStore(backend: try await options.source.backend())
    } catch {
        FileHandle.standardError.write(Data("cannot load data: \(error)\n".utf8))
        return 1
    }
    await stage(store, scene: options.scene)
    if let error = store.connectionError { FileHandle.standardError.write(Data("\(error)\n".utf8)) }
    let content = PreviewShell(store: store, fauxChrome: true)
        .environment(\.workflowsStaticLayout, true)
        .frame(width: options.size.width, height: options.size.height)
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Theme.white(0.1), lineWidth: 0.5))
    let renderer = ImageRenderer(content: content)
    renderer.scale = 2
    renderer.isOpaque = false
    guard let image = renderer.cgImage,
          let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])
    else {
        FileHandle.standardError.write(Data("render failed\n".utf8))
        return 1
    }
    do {
        try png.write(to: URL(fileURLWithPath: path))
    } catch {
        FileHandle.standardError.write(Data("\(error)\n".utf8))
        return 1
    }
    print("wrote \(path) (\(image.width)x\(image.height))")
    return 0
}

/// Gives the hidden titlebar the 40pt top bar's height so the window buttons sit on it.
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

struct PreviewWindow: View {
    let options: Options
    @State private var store: WorkflowsStore?
    @State private var failure: String?

    var body: some View {
        Group {
            if let store {
                PreviewShell(store: store)
            } else {
                Text(failure ?? "Connecting…")
                    .font(.system(size: 13))
                    .foregroundStyle(Theme.inkSecondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(Theme.surfaceBase)
            }
        }
        .ignoresSafeArea()
        .background(WindowConfigurator())
        .task {
            do {
                let store = WorkflowsStore(backend: try await options.source.backend())
                await stage(store, scene: options.scene)
                self.store = store
            } catch {
                failure = "Cannot reach vornd: \(error)"
            }
        }
    }
}

struct WorkflowsPreviewApp: App {
    static let options = Options(CommandLine.arguments)

    var body: some Scene {
        WindowGroup {
            PreviewWindow(options: Self.options).frame(minWidth: 900, minHeight: 560)
        }
        .windowStyle(.hiddenTitleBar)
        .defaultSize(Self.options.size)
    }
}

let options = Options(CommandLine.arguments)
if let path = options.render {
    Task { @MainActor in exit(await render(options, to: path)) }
    RunLoop.main.run()
} else {
    _ = NSApplication.shared
    NSApp.setActivationPolicy(.regular)
    WorkflowsPreviewApp.main()
}
