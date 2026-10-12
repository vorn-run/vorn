import AppKit
import SwiftUI
import VornCore
import VornUI

/// WorkflowEditor.tsx inline: the top bar, the canvas, and the one right-hand panel slot.
struct WorkflowPage<Leading: View>: View {
    @Bindable var store: WorkflowsStore
    @Bindable var editor: WorkflowEditorModel
    @ViewBuilder var leading: Leading
    @Environment(\.workflowsStaticLayout) private var isStatic
    @State private var portable: [PortableConnection] = []

    var body: some View {
        VStack(spacing: 0) {
            WorkflowPageBar(store: store, editor: editor, leading: { leading })
                .zIndex(1)
            HStack(spacing: 0) {
                WorkflowCanvasView(editor: editor, staticLayout: isStatic)
                    .id(editor.loadKey ?? "new")
                slot
            }
        }
        .background(Theme.surfaceBase)
        .onChange(of: store.editingWorkflow) { _, wf in if let wf { editor.sync(wf) } }
        .onChange(of: store.runs) { _, _ in editor.followRuns() }
        .onChange(of: store.workflowRuns) { _, _ in editor.followRuns() }
        .task(id: editor.launchingSince) {
            guard editor.launchingSince != nil else { return }
            try? await Task.sleep(for: .seconds(8.1))
            editor.followRuns()
        }
        .task(id: editor.loadGeneration) { portable = await store.portableConnections() }
    }

    /// Everything the canvas still cannot run: an import's leftovers and unbound connector steps.
    private var stillNeeded: [TemplateRequirement] {
        let imported = store.importedRequirements?.workflowId == editor.editingId ? store.importedRequirements?.requirements ?? [] : []
        let all = TemplateRequirements.mergeRequirements(TemplateRequirements.requirementsOfDefinition(editor.nodes), imported)
        return TemplateRequirements.requirementsWithBindings(all, connections: portable).filter { $0.connectionId == nil }
    }

    @ViewBuilder private var slot: some View {
        let pending = editor.pendingInsert
        let history = editor.showRunHistory
        let selected = editor.selectedNode
        let startFrom = editor.startFromOpen
        if let pending {
            StepLibraryPanel(editor: editor, scope: LibraryScope(pending), onPick: editor.pick) { editor.pendingInsert = nil }
                .id("\(pending.afterNodeId)|\(pending.beforeNodeId ?? "")|\(pending.replaceNodeId ?? "")|\(pending.isTrigger)")
        }
        if startFrom {
            StartFromPanel(store: store, onPick: editor.applySeed) { editor.showStartFrom = false }
        }
        let needs = stillNeeded
        if !startFrom && pending == nil && !history && selected == nil && !needs.isEmpty {
            StillNeedsPanel(requirements: needs, dismissable: store.importedRequirements?.workflowId == editor.editingId) {
                store.importedRequirements = nil
            }
        }
        if history && pending == nil, let wf = editor.existing {
            RunHistoryPanel(store: store, workflow: wf)
        }
        if let selected, !history, pending == nil {
            NodeConfigPanel(
                store: store,
                node: Binding(get: { editor.selectedNode ?? selected }, set: { editor.updateNode($0) }),
                context: NodeEditorContext(
                    nodes: editor.nodes, edges: editor.edges, latestRun: editor.runs.first,
                    onOpenTriggerLibrary: { editor.openLibrary(.trigger) },
                    onOpenReplaceLibrary: { id in
                        editor.openLibrary(InsertAnchor(afterNodeId: id, beforeNodeId: nil, replaceNodeId: id))
                    },
                    onRunToStep: EditorOps.runToStepEligible(editor.nodes, selected.id) ? { editor.run(toStep: $0) } : nil),
                menuOpen: editor.renderFormOpen == "more",
                onDelete: { editor.deleteNode(selected.id) },
                onClose: { editor.selectedNodeId = nil })
            .environment(\.formRenderOpen, editor.renderFormOpen)
            .environment(\.formRenderScroll, editor.renderPanelScroll)
            .id(selected.id)
        }
        if selected == nil && !history && pending == nil && !startFrom && editor.showProperties {
            WorkflowPropertiesPanel(
                store: store,
                workflow: Binding(
                    get: {
                        var wf = WorkflowDefinition(id: editor.editingId ?? "", name: editor.name, nodes: editor.nodes,
                                                    edges: editor.edges, enabled: editor.enabled)
                        wf.staggerDelayMs = editor.staggerDelayMs
                        wf.autoCleanupWorktrees = editor.autoCleanupWorktrees
                        return wf
                    },
                    set: { wf in
                        editor.enabled = wf.enabled
                        editor.staggerDelayMs = wf.staggerDelayMs
                        editor.autoCleanupWorktrees = wf.autoCleanupWorktrees ?? false
                    }),
                onClose: { editor.showProperties = false },
                onSelectTrigger: { if let t = editor.triggerNode { editor.selectedNodeId = t.id } })
        }
    }
}

/// The 40pt bar: icon, editable name; run or the live timer, history, save and the overflow menu.
struct WorkflowPageBar<Leading: View>: View {
    let store: WorkflowsStore
    @Bindable var editor: WorkflowEditorModel
    @ViewBuilder var leading: Leading
    @State private var closedAt = Date.distantPast
    @FocusState private var nameFocused: Bool

    var body: some View {
        let running = editor.runningRun
        HStack(spacing: 0) {
            HStack(spacing: 4) {
                leading
                iconButton
                nameField
            }
            Spacer(minLength: 8)
            HStack(spacing: 4) {
                if running != nil || editor.launchingSince != nil {
                    HStack(spacing: 2) {
                        HStack(spacing: 6) {
                            Spinning(active: true) { LucideIcon(.loaderCircle, size: 13) }
                            TimelineView(.periodic(from: .now, by: 1)) { ctx in
                                Text(Self.elapsed(since: running.flatMap { TimeFormat.parse($0.startedAt) } ?? editor.launchingSince ?? ctx.date, now: ctx.date))
                                    .font(.system(size: 11, design: .monospaced))
                                    .monospacedDigit()
                            }
                        }
                        .foregroundStyle(Theme.gray300)
                        .padding(.horizontal, 6)
                        BarButton(glyph: .square, size: 13, help: "Stop run", disabled: running == nil, disabledOpacity: 0.5, hoverTint: Theme.danger) {
                            editor.stopRun()
                        }
                    }
                } else {
                    BarButton(glyph: .play, size: 15, help: editor.hasTrigger ? "Run workflow" : "Add a trigger before running",
                              disabled: !editor.hasTrigger) { editor.run() }
                }
                if !editor.isNew {
                    BarButton(glyph: .historyClock, size: 15,
                              help: "Run history\(editor.runs.isEmpty ? "" : " (\(editor.runs.count))")",
                              pressed: editor.showRunHistory) { editor.toggleHistory() }
                }
                Hovering { hovered in
                    Button { editor.save() } label: {
                        HStack(spacing: 6) {
                            LucideIcon(.save, size: 13, strokeWidth: 1.5)
                            Text("Save")
                        }
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(Color.white)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(Theme.white(hovered ? 0.18 : 0.12), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
                .padding(.leading, 4)
                BarButton(glyph: .ellipsis, size: 15, help: "More options", pressed: editor.overflowOpen) {
                    if Date().timeIntervalSince(closedAt) > 0.3 { editor.overflowOpen.toggle() }
                }
                .overlay(alignment: .topTrailing) {
                    if editor.overflowOpen {
                        overflowMenu
                            .fixedSize()
                            .offset(y: 31)
                            .background(OutsideClick { closedAt = Date(); editor.overflowOpen = false })
                    }
                }
                .padding(.leading, 2)
            }
        }
        .padding(.horizontal, 12)
        .frame(height: Theme.toolbarHeight)
        .overlay(alignment: .bottom) { Hairline(opacity: 0.08) }
    }

    /// The timer: minutes and zero-padded seconds since the run (or the click) started.
    static func elapsed(since start: Date, now: Date) -> String {
        let total = max(0, Int(now.timeIntervalSince(start)))
        return "\(total / 60):\(String(format: "%02d", total % 60))"
    }

    private var iconButton: some View {
        Hovering { hovered in
            Button {
                if Date().timeIntervalSince(closedAt) > 0.3 { editor.iconPickerOpen.toggle() }
            } label: {
                WorkflowIconGlyph(icon: editor.icon, color: editor.iconColor, size: 16)
                    .padding(6)
                    .background(hovered ? Theme.white(0.08) : .clear, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Change icon")
            .accessibilityLabel("Change icon")
        }
        .overlay(alignment: .topLeading) {
            if editor.iconPickerOpen {
                WorkflowIconPicker(icon: $editor.icon, color: $editor.iconColor)
                    .fixedSize()
                    .offset(y: 32)
                    .background(OutsideClick { closedAt = Date(); editor.iconPickerOpen = false })
            }
        }
    }

    private var nameField: some View {
        Hovering { hovered in
            TextField("", text: $editor.name, prompt: Text("Workflow name").foregroundStyle(Theme.gray600))
                .textFieldStyle(.plain)
                .font(.system(size: 15, weight: .medium))
                .foregroundStyle(Color.white)
                .focused($nameFocused)
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .frame(width: 240)
                .background(
                    nameFocused ? Theme.white(0.06) : hovered ? Theme.white(0.04) : .clear,
                    in: RoundedRectangle(cornerRadius: Theme.radiusMd))
        }
    }

    private var overflowMenu: some View {
        DropdownPanel(minWidth: 180) {
            EditorMenuRow(glyph: .settings, label: "Workflow settings") { editor.showSettings() }
            if !editor.isNew {
                EditorMenuRow(glyph: .upload, label: "Export as file…") {
                    editor.overflowOpen = false
                    editor.exportFile()
                }
                MenuDivider()
                EditorMenuRow(glyph: .trash2, label: "Delete workflow", danger: true) {
                    editor.overflowOpen = false
                    editor.deleteWorkflow()
                }
            }
        }
    }
}

/// The overflow menu's row: 12pt lucide icon at stroke 1.5, 12px label.
struct EditorMenuRow: View {
    let glyph: LucideGlyph
    let label: String
    var danger = false
    let action: () -> Void

    var body: some View {
        Hovering { hovered in
            Button(action: action) {
                HStack(spacing: 8) {
                    LucideIcon(glyph, size: 12, strokeWidth: 1.5)
                    Text(label)
                    Spacer(minLength: 0)
                }
                .frame(minHeight: 18)
                .font(.system(size: 12))
                .foregroundStyle(danger ? Theme.danger.opacity(hovered ? 1 : 0.8) : hovered ? Color.white : Theme.gray300)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(hovered ? Theme.white(0.06) : .clear)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }
}

/// The workflow's icon: its lucide name or brand mark, the workflow glyph when unknown.
struct WorkflowIconGlyph: View {
    let icon: String
    let color: String
    var size: CGFloat = 16

    var body: some View {
        if ["github", "linear", "mcp"].contains(icon) || LucideGlyph(componentName: icon) != nil {
            ProjectIcon(icon: icon, color: color, size: size)
        } else {
            LucideIcon(.workflow, size: size, strokeWidth: 1.5).foregroundStyle(Color(css: color) ?? Theme.gray500)
        }
    }
}

/// The editor's icon picker: 8 icons a row, then the colour swatches.
struct WorkflowIconPicker: View {
    @Binding var icon: String
    @Binding var color: String

    static let options: [(String, String)] = [
        ("Folder", "Folder"), ("FolderGit2", "Git Repo"), ("Code", "Code"), ("Globe", "Web"), ("Database", "Database"),
        ("Server", "Server"), ("Smartphone", "Mobile"), ("Package", "Package"), ("FileCode", "Script"),
        ("Terminal", "Terminal"), ("Cpu", "Hardware"), ("Cloud", "Cloud"), ("Shield", "Security"), ("Zap", "Fast"),
        ("Workflow", "Workflow"), ("Gamepad2", "Game"), ("Music", "Audio"), ("Image", "Image"), ("BookOpen", "Docs"),
        ("FlaskConical", "Lab"), ("Rocket", "Launch"), ("github", "GitHub"), ("linear", "Linear"), ("mcp", "MCP"),
    ]
    static let palette = ["#6b7280", "#3b82f6", "#8b5cf6", "#ec4899", "#ef4444", "#f97316", "#eab308", "#22c55e"]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            LazyVGrid(columns: Array(repeating: GridItem(.fixed(22), spacing: 4), count: 8), spacing: 4) {
                ForEach(Self.options, id: \.0) { name, label in
                    Hovering { hovered in
                        Button { icon = name } label: {
                            WorkflowIconGlyph(icon: name, color: icon == name ? color : "#9ca3af", size: 12)
                                .frame(width: 22, height: 24)
                                .background(icon == name ? Theme.white(0.1) : hovered ? Theme.white(0.06) : .clear,
                                            in: RoundedRectangle(cornerRadius: Theme.radius))
                                .overlay {
                                    if icon == name { RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.2), lineWidth: 1) }
                                }
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .help(label)
                    }
                }
            }
            HStack(spacing: 6) {
                ForEach(Self.palette, id: \.self) { c in
                    Hovering { hovered in
                        Button { color = c } label: {
                            Circle().fill(Color(css: c) ?? .gray)
                                .frame(width: 20, height: 20)
                                .overlay(Circle().strokeBorder(color == c ? Color.white : hovered ? Theme.white(0.3) : .clear, lineWidth: 1))
                                .scaleEffect(color == c ? 1.1 : 1)
                                .contentShape(Circle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
        .padding(8)
        .frame(width: 220, alignment: .leading)
        .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusLg))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.08), lineWidth: 1))
        .shadow(color: .black.opacity(0.5), radius: 16, y: 8)
    }
}

/// Closes a popover on a press anywhere outside it, without eating the press.
struct OutsideClick: NSViewRepresentable {
    let onOutside: () -> Void

    final class Watcher: NSView {
        var onOutside: () -> Void = {}
        var monitor: Any?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if let monitor { NSEvent.removeMonitor(monitor) }
            monitor = nil
            guard window != nil else { return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] e in
                guard let self, e.window === self.window else { return e }
                if !self.bounds.contains(self.convert(e.locationInWindow, from: nil)) {
                    let close = self.onOutside
                    DispatchQueue.main.async { close() }
                }
                return e
            }
        }

        deinit { if let monitor { NSEvent.removeMonitor(monitor) } }
    }

    func makeNSView(context: Context) -> Watcher {
        let w = Watcher()
        w.onOutside = onOutside
        return w
    }

    func updateNSView(_ w: Watcher, context: Context) { w.onOutside = onOutside }
}

/// The "Still needs" panel: what the canvas cannot run yet, worded like the template rows.
struct StillNeedsPanel: View {
    let requirements: [TemplateRequirement]
    let dismissable: Bool
    let onDismiss: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Still needs").font(.system(size: 13, weight: .medium)).foregroundStyle(.white)
                Spacer()
                if dismissable { PanelCloseButton(help: "Dismiss", action: onDismiss) }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .overlay(alignment: .bottom) { Rectangle().fill(Theme.white(0.08)).frame(height: 1) }
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(requirements, id: \.self) { entry in
                        Text("Needs \(WorkflowFiles.describeRequirement(entry.requirement))")
                            .font(.system(size: 11))
                            .foregroundStyle(Theme.bronzo)
                            .lineLimit(1)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.leading, 38)
                            .padding(.trailing, 10)
                            .padding(.bottom, 6)
                    }
                }
                .padding(.vertical, 8)
            }
        }
        .frame(width: 280)
        .frame(maxHeight: .infinity, alignment: .top)
        .background(Theme.surfaceNode)
        .overlay(alignment: .leading) { Rectangle().fill(Theme.white(0.08)).frame(width: 1) }
    }
}

/// `p-1.5 rounded-md` bar button: gray-400, hover white on white .06, pressed white on white .08.
struct BarButton: View {
    let glyph: LucideGlyph
    var size: CGFloat = 15
    var help: String
    var disabled = false
    var disabledOpacity: Double = 0.4
    var pressed = false
    var hoverTint: Color = .white
    let action: () -> Void

    var body: some View {
        Hovering { hovered in
            Button(action: action) {
                LucideIcon(glyph, size: size)
                    .foregroundStyle(pressed ? .white : hovered && !disabled ? hoverTint : Theme.gray400)
                    .padding(6)
                    .background(
                        pressed ? Theme.white(0.08) : hovered && !disabled ? Theme.white(0.06) : .clear,
                        in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(disabled)
            .opacity(disabled ? disabledOpacity : 1)
            .help(help)
            .accessibilityLabel(help)
        }
    }
}
