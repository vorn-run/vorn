import AppKit
import SwiftUI
import VornCore
import VornPanes
import VornSettings
import VornTerminals
import VornUI
import VornWorkflows
import WebKit

public let defaultWindowSize = CGSize(width: 1280, height: 800)

/// The app: one window onto the local vornd.
public struct VornMacApp: App {
    @State private var store: VornStore
    @State private var features: FeatureStores
    @State private var model = AppModel()
    @State private var engines = EngineHolder()
    @State private var keys = KeyMonitor()
    @State private var widget: WidgetController?

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
                .onAppear {
                    store.start()
                    keys.install(model: model)
                    OnboardingGate.bind(model: model, store: store)
                    Task { _ = await OnboardingGate.openIfUnseen(model: model, store: store) }
                    if widget == nil {
                        let controller = WidgetController(store: store, model: model)
                        controller.install()
                        model.toggleWidget = { controller.toggle() }
                        widget = controller
                    }
                }
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
        CommandGroup(replacing: .appSettings) {
            Button("Settings…") { model.settingsOpen = true }.keyboardShortcut(",")
        }
        CommandGroup(replacing: .newItem) {
            Button("New Session") { actions.newSession(actions.activeProject, actions.activeWorktree) }
                .keyboardShortcut("n")
            Button("New Terminal") {
                actions.newTerminal(actions.activeProject ?? store.projects.first, actions.activeWorktree)
            }
            .keyboardShortcut("`", modifiers: .control)
            Divider()
            Button("Stop Sessions and Server") { actions.stopSessionsAndServer() }
            Divider()
        }
        CommandGroup(after: .windowArrangement) {
            Button("Toggle Widget") { model.toggleWidget?() }
        }
        CommandGroup(after: .sidebar) {
            Button("Toggle Sidebar") { model.sidebarOpen.toggle() }.keyboardShortcut("b")
            Button("Command Palette") { model.toggle(.commandPalette) }.keyboardShortcut("k")
            Divider()
            Button("Sessions") { model.mainViewMode = .sessions }.keyboardShortcut("s")
            Button("Tasks") { model.mainViewMode = .tasks }.keyboardShortcut("t")
            Button("Workflows") { model.mainViewMode = .workflows }.keyboardShortcut("w", modifiers: [.command, .shift])
            Divider()
            Button("View Options") { model.toggle(.viewOptions) }.keyboardShortcut("j")
            Button("Recent Sessions") { model.toggle(.recent) }
            Button("Keyboard Shortcuts") { model.toggle(.shortcuts) }.keyboardShortcut("/")
            Divider()
            Menu("Filter") {
                ForEach(Array(StatusFilter.allCases.enumerated()), id: \.offset) { i, filter in
                    Button(filter == .all ? "Show All" : filter == .error ? "Show Errors" : "Show \(filter.label)") {
                        model.statusFilter = filter
                    }
                    .keyboardShortcut(KeyEquivalent(Character("\(i + 1)")), modifiers: .option)
                }
            }
            Divider()
        }
        CommandMenu("Session") {
            Button("Next Session") { model.cycleFocus(store, by: 1) }.keyboardShortcut("]")
            Button("Previous Session") { model.cycleFocus(store, by: -1) }.keyboardShortcut("[")
            Button("Expand Selected") {
                if model.layoutMode(store) == .grid, let id = model.selectedSessionID { model.expandedSessionID = id }
            }
            .keyboardShortcut("o")
            Button("Rename Session") { model.startRename() }
                .keyboardShortcut(KeyEquivalent(Character(UnicodeScalar(NSF2FunctionKey)!)), modifiers: [])
            Divider()
            ForEach(1..<10) { n in
                Button("Session \(n)") { model.jump(to: n, store) }
                    .keyboardShortcut(KeyEquivalent(Character("\(n)")))
            }
        }
    }
}

/// Esc, ⌘W and ⌘K, which the terminal keeps when it has focus (useKeyboardShortcuts.ts).
@MainActor
final class KeyMonitor {
    private var token: Any?

    func install(model: AppModel) {
        guard token == nil else { return }
        token = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            let mods = event.modifierFlags.intersection([.command, .shift, .option, .control])
            if event.keyCode == 13, mods == .command, model.expandedSessionID != nil {
                model.selectedSessionID = model.expandedSessionID
                model.expandedSessionID = nil
                return nil
            }
            if event.keyCode == 40, mods == .command {
                model.toggle(.commandPalette)
                return nil
            }
            guard event.keyCode == 53, mods.isEmpty else { return event }
            let responder = event.window?.firstResponder
            let typing = responder is TerminalNSView || responder is NSTextView
            if typing, model.contextMenu == nil, model.overlay == nil { return event }
            return model.escape() ? nil : event
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
/// `--view sessions|tasks|workflows`, `--layout grid|tabs`, `--focused N`, `--filter STATUS`, `--columns N`,
/// `--workflow NAME [--history] [--full-output] [--select LABEL] [--library [--library-query Q]] [--icon-picker] [--more] [--form-open more|agent|model|vars:PLACEHOLDER] [--panel-scroll PT]` opens a workflow's page,
/// `--new-workflow [--trigger-library]` a blank editor, `--workflow-menu NAME@X,Y` a sidebar row's menu, `--run-prompt NAME` the run prompt, `--missed JSON` the missed-schedules dialog, `--export-workflow NAME@PATH` and `--drop-file PATH` export and import without a dialog, `--task TITLE` opens a task's detail (`--editor slash|toolbar|link` drives its editor), `--sidebar-mode MODE`, `--overlay NAME` (`--palette-query TEXT` types into the command palette), `onboarding` takes `--onboarding-step N`, `--mcp-open`, `--installed a,b`; `--widget [--widget-compact] [--widget-permission]` the floating widget alone, `--dock [--dock-collapsed]` loads waiting gates into the session dock, `--context grid|card:N [--submenu NAME]`.
@MainActor
func renderMainScreen(to path: String, arguments args: [String]) async -> Int32 {
    func value(_ flag: String) -> String? {
        args.firstIndex(of: flag).flatMap { $0 + 1 < args.count ? args[$0 + 1] : nil }
    }
    var size = defaultWindowSize
    if let s = value("--size")?.split(separator: "x"), s.count == 2, let w = Double(s[0]), let h = Double(s[1]) {
        size = CGSize(width: w, height: h)
    }

    // An import writes, so a render that drops a file needs a writable connection to its test vornd.
    let store = VornStore(readOnly: !args.contains("--drop-file"))
    let features = FeatureStores(client: store.client)
    let model = AppModel()
    model.persistsSidebar = false
    model.panes = PanesModel(persists: false)
    model.panes.confirmDiscard = { true }
    model.sidebarViewMode = value("--sidebar-mode").flatMap(SidebarViewMode.init) ?? .worktreesSessions
    model.projectSort = .manual
    model.worktreeSort = .name
    model.worktreeFilter = .all
    if let layout = value("--layout").flatMap(LayoutMode.init) { model.layoutOverride = layout }
    if let filter = value("--filter").flatMap(StatusFilter.init) { model.statusFilter = filter }
    if let cols = value("--columns").flatMap(Int.init) { model.gridColumns = cols }
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
    engines.update(socket: endpoint.gridSocket, readOnly: true, fontSize: store.defaults?.fontSize)

    features.bind(store: store, model: model)
    switch model.mainViewMode {
    case .sessions:
        if args.contains("--dock") {
            await features.workflows.reload()
            model.sessionDockCollapsed = args.contains("--dock-collapsed")
        }
    case .tasks:
        features.tasks.scope = features.scope(model, store)
        await features.tasks.reload()
        if let title = value("--task"), let task = features.tasks.tasks.first(where: { $0.title == title }) {
            features.tasks.selectedTaskId = task.id
            try? await Task.sleep(for: .milliseconds(800))
        }
    case .workflows:
        features.workflows.editorMemory = .ephemeral()
        await features.workflows.reload()
        if args.contains("--new-workflow") {
            await features.workflows.awaitFreshCatalog()
            features.workflows.newWorkflow()
            if args.contains("--trigger-library") { features.workflows.editor?.driveForRender(triggerLibrary: true) }
        }
        if let spec = value("--export-workflow")?.split(separator: "@"), spec.count == 2,
           let wf = features.workflows.workflows.first(where: { $0.name == spec[0] }) {
            let to = URL(fileURLWithPath: String(spec[1]))
            features.workflows.workflowFiles = WorkflowFileIO(
                save: { _, contents, _ in try contents.write(to: to, atomically: true, encoding: .utf8); return true },
                pick: { nil })
            features.workflows.exportWorkflow(wf)
            try? await Task.sleep(for: .milliseconds(800))
        }
        if let path = value("--drop-file") {
            features.workflows.projects = store.projects
            features.workflows.importDroppedWorkflow([URL(fileURLWithPath: path)])
            try? await Task.sleep(for: .milliseconds(800))
        }
        if let name = value("--run-prompt"), let wf = features.workflows.workflows.first(where: { $0.name == name }) {
            features.workflows.projects = store.projects
            features.workflows.runNow(wf)
        }
        if let raw = value("--missed"), let json = try? JSONDecoder().decode(JSONValue.self, from: Data(raw.utf8)) {
            features.workflows.surfaces.missed = MissedSchedule.list(json)
            features.workflows.surfaces.missedSelected = Set(features.workflows.surfaces.missed.map(\.workflowId))
            if let off = value("--missed-unselect") { features.workflows.surfaces.missedSelected.remove(off) }
        }
        if let target = value("--workflow-menu")?.split(separator: "@"), target.count == 2,
           let wf = features.workflows.workflows.first(where: { $0.name == target[0] }) {
            let xy = target[1].split(separator: ",").compactMap { Double($0) }
            if xy.count == 2 { features.workflows.menu = WorkflowsMenu(kind: .workflow(wf.id), at: CGPoint(x: xy[0], y: xy[1])) }
        }
        if let name = value("--workflow"), let wf = features.workflows.workflows.first(where: { $0.name == name }) {
            await features.workflows.openWorkflow(wf.id).value
            features.workflows.historyOpen = args.contains("--history")
            features.workflows.editor?.driveForRender(
                select: value("--select"), library: args.contains("--library"), query: value("--library-query"),
                iconPicker: args.contains("--icon-picker"), more: args.contains("--more"),
                formOpen: value("--form-open"), panelScroll: value("--panel-scroll").flatMap(Double.init) ?? 0)
            if args.contains("--full-output"),
               let logs = features.workflows.workflowRuns.first?.nodeStates.compactMap(\.logs).first(where: { !$0.isEmpty }) {
                features.workflows.showFullOutput(logs)
            }
        }
        if args.contains("--review") {
            let wf = features.workflows
            if let run = (wf.runs + wf.workflowRuns).first(where: { $0.nodeStates.contains { $0.viewToken != nil } }),
               let gate = run.nodeStates.first(where: { $0.viewToken != nil }) {
                wf.selectedRunId = run.runId
                wf.openReview(run, nodeId: gate.nodeId, workflowName: wf.ref(for: run)?.name)
            }
        }
    }

    if let s = value("--update-status") {
        let parts = s.split(separator: ":", maxSplits: 1).map(String.init)
        let v = parts.count > 1 ? parts[1] : "0.8.1"
        features.settings.updateStatus = switch parts[0] {
        case "idle": .idle(lastCheckedAt: Date().timeIntervalSince1970 * 1000 - 180_000)
        case "checking": .checking
        case "available": .available(version: v)
        case "downloading": .downloading(version: v, percent: 42)
        case "ready": .ready(version: v)
        case "error": .error(v)
        default: .unsupported
        }
    }
    if let cat = value("--settings") {
        guard let category = SettingsCategory(rawValue: cat) else {
            FileHandle.standardError.write(Data("unknown settings category \(cat)\n".utf8))
            return 2
        }
        let settings = features.settings
        settings.category = category
        if let hints = value("--settings-hints") { settings.renderHints = Set(hints.split(separator: ",").map(String.init)) }
        model.settingsOpen = true
        await settings.load()
        switch category {
        case .general: await settings.detectAgents(); await settings.loadShells()
        case .agents: await settings.detectAgents()
        case .ssh: await settings.loadSSHKeys()
        case .keys: await settings.loadConnectorKeys()
        case .about: await settings.loadCore()
        case .worktrees: await settings.wtLoad(refresh: false)
        default: break
        }
    }

    // Welcome guide: --installed a,b stands in for agent detection, which a read-only render cannot ask for.
    if let list = value("--installed") {
        let on = Set(list.split(separator: ",").map(String.init))
        features.settings.stage(installedAgents: Dictionary(uniqueKeysWithValues: AgentDefinition.all.map {
            ($0.type.rawValue, on.contains($0.type.rawValue))
        }))
    }
    if let step = value("--onboarding-step").flatMap(Int.init) { model.onboardingStep = Onboarding.clamp(step) }
    OnboardingModal.renderMcpOpen = args.contains("--mcp-open")
    if args.contains("--widget") {
        let feed = WidgetFeed(store: store)
        feed.mode = args.contains("--widget-compact") ? .compact : .full
        if args.contains("--widget-permission"), let s0 = store.sessions.first {
            feed.apply("widget:permission-request", [
                "requestId": "r1", "terminalId": .string(s0.id), "toolName": "Bash",
                "toolInput": ["command": "yarn test --watch=false"],
                "permissionSuggestions": [["type": "addRules", "rules": [["toolName": "Bash", "ruleContent": "yarn test:*"]]]],
            ])
            feed.apply("widget:permission-request", [
                "requestId": "r2", "terminalId": .string(s0.id), "toolName": "AskUserQuestion",
                "toolInput": [:],
                "questions": [["header": "Scope", "question": "Which tests should run?",
                               "options": [["label": "Unit only"], ["label": "Everything"]]]],
            ])
        }
        let widgetSize = feed.mode.size
        let view = WidgetView(feed: feed, agents: feed.agents)
            .frame(width: widgetSize.width, height: widgetSize.height, alignment: .top)
        guard let image = await hostedImage(of: view, size: widgetSize, settle: 600),
              let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else { return 1 }
        try? png.write(to: URL(fileURLWithPath: path))
        store.stop()
        print("wrote \(path) (\(image.width)x\(image.height)), \(feed.agents.count) widget rows")
        return 0
    }

    // Panes: --files N[:rel,..], --claim N:M, --promote N:rel, --changes N, --commit, --maximize files|terminals, --preview-tab.
    if let names = value("--expand") { FileTreeModel.renderExpand = Set(names.split(separator: ",").map(String.init)) }
    let all = model.mainViewMode == .sessions ? model.visibleSessions(store, includeMinimized: true) : []
    func cwd(_ s: TerminalSession) -> String { s.worktreePath ?? s.projectPath }
    for path in Set(all.map(cwd)) { await store.refreshDiffStat(path) }
    if !all.isEmpty { OpenInButton.cache = try? await store.client.detectIDEs() }
    if args.contains("--files") || args.contains("--promote") { _ = await SyntaxHighlighter.shared.tokens("", language: "typescript") }
    if let spec = value("--claim") {
        let parts = spec.split(separator: ":").compactMap { Int($0) }
        if parts.count == 2, all.indices.contains(parts[0]), all.indices.contains(parts[1]) {
            model.panes.openTerminalsPane(all[parts[0]].id, terminal: all[parts[1]].id)
        }
    }
    if let spec = value("--files") {
        let parts = spec.split(separator: ":", maxSplits: 1).map(String.init)
        if let n = Int(parts[0]), all.indices.contains(n) {
            let s = all[n]
            model.panes.toggleFilesPane(s.id)
            for rel in parts.count > 1 ? parts[1].split(separator: ",").map(String.init) : [] {
                model.panes.openFileTab(s.id, (cwd(s) as NSString).appendingPathComponent(rel), pin: !args.contains("--preview-tab"))
            }
            if parts.count > 1 && !args.contains("--tree") { model.panes.setFileTreeVisible(s.id, false) }
        }
    }
    if let spec = value("--promote") {
        let parts = spec.split(separator: ":", maxSplits: 1).map(String.init)
        if parts.count == 2, let n = Int(parts[0]), all.indices.contains(n) {
            model.panes.promoteFile(all[n].id, (cwd(all[n]) as NSString).appendingPathComponent(parts[1]))
        }
    }
    let browser = model.panes.browser
    let usesBrowser = ["--browser", "--artifact", "--device"].contains(where: args.contains)
    if usesBrowser {
        BrowserModel.inlinePopovers = true
        ShellActions(store: store, model: model).bindBrowser()
        browser.deviceBackend = .rendering
    }
    if let spec = value("--browser") {
        let parts = spec.split(separator: ":", maxSplits: 1).map(String.init)
        if let n = Int(parts[0]), all.indices.contains(n) { browser.open(all[n].id, url: parts.count > 1 ? parts[1] : nil) }
    }
    if let spec = value("--artifact") {
        let parts = spec.split(separator: ":", maxSplits: 1).map(String.init)
        if parts.count == 2, let n = Int(parts[0]), all.indices.contains(n),
           let list = try? await store.client.call("artifact:list", ["limit": 50]),
           let a = (list.arrayValue ?? list["artifacts"]?.arrayValue ?? []).first(where: { $0["title"]?.stringValue == parts[1] }),
           let id = a["id"]?.stringValue, let version = a["latestVersion"]?.numberValue,
           let url = try? await store.client.call("artifact:versionUrl", ["artifactId": .string(id), "version": .number(version)])["url"] {
            _ = await browser.bridge("browser:openPane", [
                "sessionId": .string(all[n].id), "url": url,
                "artifact": ["id": .string(id), "version": .number(version), "kind": a["kind"] ?? "doc", "title": .string(parts[1])],
            ])
        }
    }
    if let n = value("--device").flatMap(Int.init), all.indices.contains(n) {
        await browser.renderStep(all[n].id, "claim:\(DeviceBackend.renderingDevice.udid)")
    }
    if let kind = value("--maximize"), let first = all.first(where: { !model.panes.columnEntries($0.id).isEmpty }) {
        model.panes.maximizedPaneId = switch kind {
        case "files": PaneID.files(first.id)
        case "browser": PaneID.browser(first.id)
        case "device": PaneID.device(first.id)
        default: PaneID.terminals(first.id)
        }
    }
    let browserSteps = value("--browser-step").map { $0.split(separator: ",").map(String.init) } ?? []
    if let n = value("--changes").flatMap(Int.init), all.indices.contains(n) {
        model.panes.diffSessionID = all[n].id
        await store.refreshDiffStat(cwd(all[n]))
        model.commitOpen = args.contains("--commit")
    }

    let visible = model.mainViewMode == .sessions ? model.visibleSessions(store) : []
    if let n = value("--focused").flatMap(Int.init), visible.indices.contains(n) {
        model.selectedSessionID = visible[n].id
        model.expandedSessionID = visible[n].id
    }
    if model.layoutMode(store) == .tabs { model.activeTabID = visible.first?.id }
    if let engine = engines.engine, !visible.isEmpty {
        let gridW = size.width - (model.sidebarOpen ? model.sidebarWidth : 0)
        let body: CGSize
        if model.expandedSessionID != nil {
            body = CGSize(width: gridW, height: size.height - 41 - 22 - 2)
        } else if model.layoutMode(store) == .tabs {
            body = CGSize(width: gridW, height: size.height - Theme.toolbarHeight - 36 - 22 - 2)
        } else {
            let gridH = size.height - Theme.toolbarHeight
            let layout = AutoLayout.pick(visible.count, width: gridW, height: gridH)
            let cardH = layout.mode == .fit ? gridH / CGFloat(layout.rows) : AutoLayout.scrollRowHeight(gridH)
            body = CGSize(width: gridW / CGFloat(layout.cols), height: cardH - 41 - 22 - 2)
        }
        for s in visible { engine.attachSnapshot(s.id, size: body) }
        for id in model.panes.claimedTerminalIDs { engine.attachSnapshot(id, size: CGSize(width: body.width / 2, height: body.height / 2)) }
        await engine.settle()
        if args.contains("--dump-text") {
            for s in visible { print("--- \(s.title)\n\(engine.snapshotText(s.id) ?? "")") }
        }
    }

    if let name = value("--overlay") {
        let overlays: [String: Overlay] = [
            "view-options": .viewOptions, "recent": .recent, "shortcuts": .shortcuts,
            "new-menu": .newMenu, "sidebar-options": .sidebarOptions, "dock": .dock, "command-palette": .commandPalette,
            "onboarding": .onboarding,
            "add-project": .addProject(editing: nil), "edit-project": .addProject(editing: store.projects.first?.name),
            "project-menu": .projectMenu(store.projects.first?.name ?? ""),
            "remove-worktree": store.sessions.first(where: { $0.worktreePath != nil }).map {
                .removeWorktree(projectPath: $0.projectPath, worktreePath: $0.worktreePath ?? "")
            } ?? .viewOptions,
        ]
        guard let overlay = overlays[name] else {
            FileHandle.standardError.write(Data("unknown overlay \(name)\n".utf8))
            return 2
        }
        CommandPaletteView.renderQuery = value("--palette-query")
        if overlay == .recent || overlay == .commandPalette {
            let now = Date().timeIntervalSince1970 * 1000
            let path = store.projects.first?.path ?? "/tmp/vorn"
            model.recentFixture = [
                RecentSession(sessionId: "r1", agentType: .claude, display: "Fix the flaky sidebar test", projectPath: path,
                              timestamp: now - 120_000, activityCount: 3, activityLabel: "prompt", canResumeExact: true),
                RecentSession(sessionId: "r2", agentType: .codex, display: "Port the status bar", projectPath: path,
                              timestamp: now - 7_200_000, activityCount: 1, activityLabel: "prompt", canResumeExact: true),
            ]
        }
        model.overlay = overlay
    }
    if let target = value("--context") {
        let card = target.hasPrefix("card:") ? Int(target.dropFirst(5)).flatMap { visible.indices.contains($0) ? visible[$0].id : nil } : nil
        model.contextMenu = ContextMenuState(target: card.map { .card($0) } ?? .grid,
                                             point: CGPoint(x: size.width * 0.45, y: size.height * 0.3),
                                             submenu: value("--submenu"))
    }

    if args.contains("--cleanup-toast"), let s = all.first(where: { $0.worktreePath != nil }), let wt = s.worktreePath {
        ShellActions(store: store, model: model).offerWorktreeCleanup(projectPath: s.projectPath, worktreePath: wt)
        try? await Task.sleep(for: .milliseconds(500))
    }
    if let name = value("--component"), let s0 = all.first(where: { $0.branch != nil }) ?? all.first {
        let actions = ShellActions(store: store, model: model)
        let view: AnyView
        switch name {
        case "branch-picker":
            view = AnyView(BranchPicker(projectPath: s0.projectPath, currentBranch: s0.branch, git: actions.gitActions) { _ in })
        case "open-in":
            view = AnyView(OpenInButton.Menu(ides: OpenInButton.cache ?? [], projectPath: s0.projectPath, git: actions.gitActions) {})
        case "error-screen":
            view = AnyView(ErrorScreen(error: "TypeError: Cannot read properties of undefined (reading 'sessions')",
                                       reload: {}).frame(width: size.width, height: size.height))
        default:
            FileHandle.standardError.write(Data("unknown component \(name)\n".utf8))
            return 2
        }
        let framed = view.environment(\.colorScheme, .dark).fixedSize()
        guard let image = await hostedImage(of: framed.frame(width: size.width, height: size.height, alignment: .topLeading)
            .background(Theme.surfaceBase), size: size, settle: 1200),
              let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else { return 1 }
        try? png.write(to: URL(fileURLWithPath: path))
        print("wrote \(path)")
        return 0
    }

    let content = RootView(model: model, store: store, engines: engines, features: features, fauxChrome: true)
        .environment(\.terminalSnapshot, true)
        .environment(\.workflowsStaticLayout, true)
        .inlinePopoverHost()
        .frame(width: size.width, height: size.height)
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Color.white.opacity(0.1), lineWidth: 0.5))
    // ImageRenderer leaves scroll views blank, so Tasks and the scrolling panels are drawn through a hosting view.
    let scrolls = model.mainViewMode != .sessions || model.overlay == .shortcuts || model.overlay == .recent
        || model.overlay == .commandPalette || model.overlay == .onboarding || model.settingsOpen
        || model.panes.diffSessionID != nil || !model.panes.filesPanes.isEmpty || !model.panes.terminalsPanes.isEmpty
        || !model.panes.editorCards.isEmpty || usesBrowser
    let editorState = value("--editor")
    let settle = args.contains("--review") ? 2500 : usesBrowser ? 3000
        : model.panes.filesPanes.isEmpty && model.panes.editorCards.isEmpty ? 300 : 1500
    let stepped = all.first { browser.isOpen($0.id) || browser.hasDevicePane($0.id) }?.id
    let image = scrolls ? await hostedImage(of: content, size: size, settle: settle) { root in
        if let editorState { RichEditorDriver.drive(root, editorState) }
        if let stepped { Task { for step in browserSteps { await browser.renderStep(stepped, step) } } }
    } : {
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
private func hostedImage(
    of content: some View, size: CGSize, settle: Int = 300, prepare: (NSView) -> Void = { _ in }
) async -> CGImage? {
    let host = NSHostingView(rootView: content)
    host.frame = CGRect(origin: .zero, size: size)
    let window = NSWindow(contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: false)
    window.appearance = NSAppearance(named: .darkAqua)
    window.backgroundColor = .clear
    window.isOpaque = false
    window.contentView = host
    host.layoutSubtreeIfNeeded()
    try? await Task.sleep(for: .milliseconds(400))
    prepare(host)
    try? await Task.sleep(for: .milliseconds(settle))
    host.layoutSubtreeIfNeeded()
    guard let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: Int(size.width * 2), pixelsHigh: Int(size.height * 2),
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
    ) else { return nil }
    rep.size = size
    await freezeWebViews(in: host)
    host.cacheDisplay(in: host.bounds, to: rep)
    return rep.cgImage
}

/// Page views don't draw through `cacheDisplay`, so each is swapped for an image of its snapshot first.
@MainActor
private func freezeWebViews(in host: NSView) async {
    func webViews(_ v: NSView) -> [WKWebView] {
        (v as? WKWebView).map { [$0] } ?? v.subviews.flatMap(webViews)
    }
    for web in webViews(host) where web.window != nil && !web.isHiddenOrHasHiddenAncestor && web.bounds.width > 1 && web.bounds.height > 1 {
        guard let image = try? await web.takeSnapshot(configuration: nil), let parent = web.superview else { continue }
        let still = NSImageView(frame: web.frame)
        still.image = image
        still.imageScaling = .scaleAxesIndependently
        parent.addSubview(still, positioned: .above, relativeTo: web)
        web.isHidden = true
    }
}
