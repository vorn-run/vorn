import SwiftUI
import VornCore
import VornTerminals
import VornUI
import VornWorkflows

/// Where the top bar's controls sit, so their panels can hang from them.
struct ControlAnchors: PreferenceKey {
    static let defaultValue: [String: Anchor<CGRect>] = [:]
    static func reduce(value: inout [String: Anchor<CGRect>], nextValue: () -> [String: Anchor<CGRect>]) {
        value.merge(nextValue()) { _, b in b }
    }
}

extension View {
    func controlAnchor(_ key: String) -> some View {
        anchorPreference(key: ControlAnchors.self, value: .bounds) { [key: $0] }
    }
}

/// Every floating panel and menu, over the whole window.
struct OverlayHost: View {
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions
    let features: FeatureStores
    let anchors: [String: Anchor<CGRect>]

    var body: some View {
        GeometryReader { geo in
            let size = geo.size
            if model.overlay != nil || model.contextMenu != nil {
                backdrop
                    .contentShape(Rectangle())
                    .onTapGesture { close() }
                    .onRightClick { _ in close() }
            }
            if let overlay = model.overlay {
                panel(overlay, size: size, geo: geo)
            }
            if let menu = model.contextMenu {
                SessionContextMenu(model: model, store: store, actions: actions, features: features,
                                   state: menu, window: size)
            }
        }
        .ignoresSafeArea()
    }

    private func close() {
        model.contextMenu = nil
        model.overlay = nil
    }

    private var backdrop: Color {
        switch model.overlay {
        case .addProject?: Color.black.opacity(0.5)
        case .shortcuts?, .removeWorktree?, .commandPalette?: Color.black.opacity(0.4)
        default: Color.white.opacity(0.001)
        }
    }

    private func rect(_ key: String, _ geo: GeometryProxy) -> CGRect? { anchors[key].map { geo[$0] } }

    @ViewBuilder private func panel(_ overlay: Overlay, size: CGSize, geo: GeometryProxy) -> some View {
        switch overlay {
        case .viewOptions:
            if let r = rect("viewOptions", geo) {
                ViewOptionsPanel(model: model, store: store, actions: actions)
                    .offset(x: r.maxX - 200, y: r.maxY + 4)
            }
        case .recent:
            if let r = rect("recent", geo) {
                RecentSessionsPanel(model: model, store: store, actions: actions)
                    .offset(x: r.maxX - 380, y: r.maxY + 8)
            }
        case .headless:
            if let r = rect("headless", geo) {
                HeadlessPanel(model: model, store: store, actions: actions)
                    .fixedSize()
                    .offset(x: model.layoutMode(store) == .tabs ? r.maxX - 280 : r.minX, y: r.maxY + 6)
            }
        case .shortcuts:
            ShortcutsPanel(model: model)
                .frame(width: 520)
                .offset(x: (size.width - 520) / 2, y: size.height * 0.15)
        case .commandPalette:
            CommandPaletteView(model: model, store: store, actions: actions, features: features)
                .offset(x: (size.width - CommandPaletteView.width) / 2, y: size.height * 0.2)
        case .sidebarOptions:
            if let r = rect("sidebarOptions", geo) {
                SidebarOptionsPanel(model: model).offset(x: r.minX, y: r.maxY + 4)
            }
        case .projectMenu(let name):
            if let r = rect("project:\(name)", geo) {
                ProjectMenuPanel(project: name, model: model, actions: actions)
                    .frame(width: r.maxX, alignment: .trailing)
                    .offset(y: r.maxY + 4)
            }
        case .addProject(let editing):
            AddProjectDialog(model: model, store: store, actions: actions, editing: editing)
                .id(editing)
                .frame(width: size.width, height: size.height)
        case .removeWorktree(let projectPath, let worktreePath):
            WorktreeCleanupDialog(projectPath: projectPath, worktreePath: worktreePath, model: model, store: store,
                                  actions: actions)
                .frame(width: size.width, height: size.height)
        case .newMenu:
            if let r = rect("newMenu", geo) {
                SessionContextMenu(model: model, store: store, actions: actions, features: features,
                                   state: ContextMenuState(target: .grid, point: CGPoint(x: r.maxX - 220, y: r.maxY + 4)),
                                   window: size)
            }
        }
    }
}

// MARK: View options

/// The sliders' panel (GridToolbar.tsx): status filter, sort, layout, columns.
struct ViewOptionsPanel: View {
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions

    static let columns: [(Int, String, String?)] = [
        (0, "Auto", "Adapts columns and rows to viewport and card count"),
        (1, "1 Column", nil), (2, "2 Columns", nil), (3, "3 Columns", nil), (4, "4 Columns", nil),
        (-1, "Flexible", nil),
    ]

    var body: some View {
        let layout = model.layoutMode(store)
        MenuPanel(width: 200, border: 0.08, verticalPadding: 0) {
            OptionSection(label: "Status") {
                ForEach(StatusFilter.allCases, id: \.self) { f in
                    MenuOptionRow(f.label, selected: model.statusFilter == f, dot: f.dot) { model.statusFilter = f }
                }
            }
            OptionSection(label: "Sort", rule: true) {
                ForEach(SortMode.allCases, id: \.self) { m in
                    MenuOptionRow(m.label, selected: model.sortMode == m) { model.sortMode = m }
                }
            }
            OptionSection(label: "Layout", rule: true) {
                HStack(spacing: 4) {
                    layoutButton("Grid", .layoutGrid, .grid, layout)
                    layoutButton("Tabs", .layoutTabs, .tabs, layout)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 4)
            }
            if layout == .grid {
                OptionSection(label: "Columns", rule: true) {
                    ForEach(Self.columns, id: \.0) { value, label, help in
                        MenuOptionRow(label, selected: model.gridColumns == value, help: help) { model.gridColumns = value }
                    }
                }
            }
        }
    }

    private func layoutButton(_ label: String, _ icon: LucideGlyph, _ mode: LayoutMode, _ current: LayoutMode) -> some View {
        let on = mode == current
        return Button { actions.setLayout(mode) } label: {
            HStack(spacing: 6) {
                LucideIcon(icon, size: 12)
                Text(label)
            }
            .font(.ui(11))
            .foregroundStyle(on ? Color.white : Theme.gray500)
            .padding(.horizontal, 10)
            .padding(.vertical, 4)
            .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(on ? Theme.white(0.1) : Theme.white(0.04)))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// The top bar's sliders: tinted while open, a dot while any option differs from its default.
struct ViewOptionsButton: View {
    @Bindable var model: AppModel
    let store: VornStore

    var body: some View {
        let open = model.overlay == .viewOptions
        let active = model.hasActiveViewOptions(store)
        Hovering { hover in
            Button { model.toggle(.viewOptions) } label: {
                LucideIcon(.slidersHorizontal, size: 16, strokeWidth: 1.5)
                    .foregroundStyle(open || active || hover ? Color.white : Theme.gray400)
                    .padding(4)
                    .background(RoundedRectangle(cornerRadius: Theme.radiusMd)
                        .fill(open ? Theme.white(0.1) : active ? Theme.white(0.08) : hover ? Theme.white(0.06) : .clear))
                    .overlay(alignment: .topTrailing) {
                        if active && !open {
                            Circle().fill(Theme.inkSecondary).frame(width: 6, height: 6).offset(x: -2, y: 2)
                        }
                    }
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .tooltip("View options", shortcut: "⌘J")
        .controlAnchor("viewOptions")
    }
}

// MARK: Recent sessions

/// RecentSessionsPopover.tsx: conversations on disk, newest first, resumable in a new pane.
struct RecentSessionsPanel: View {
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions
    @State private var sessions: [RecentSession]?

    var body: some View {
        let project = model.activeProject.flatMap(store.project(named:))
        let list = sessions ?? model.recentFixture ?? []
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                Text("Recent Sessions").font(.ui(13, weight: .medium)).foregroundStyle(.white)
                Text("· \(model.activeProject ?? "All Projects")").font(.ui(11)).foregroundStyle(Theme.gray500).lineLimit(1)
                Spacer(minLength: 8)
                if !list.isEmpty { Text("\(list.count)").font(.ui(12)).foregroundStyle(Theme.gray600) }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }
            if list.isEmpty {
                Text(sessions == nil && model.recentFixture == nil ? "" : "No recent sessions")
                    .font(.ui(12))
                    .foregroundStyle(Theme.gray600)
                    .frame(maxWidth: .infinity)
                    .padding(32)
            } else {
                ScrollView {
                    VStack(spacing: 0) {
                        ForEach(list) { r in
                            RecentRow(session: r, showProject: project == nil) { actions.resumeRecent(r) }
                        }
                    }
                    .padding(4)
                }
                .frame(maxHeight: 400 - 45)
                .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(width: 380)
        .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusXl))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusXl).strokeBorder(Theme.white(0.08), lineWidth: 1))
        .clipShape(RoundedRectangle(cornerRadius: Theme.radiusXl))
        .shadow(color: .black.opacity(0.5), radius: 20, y: 10)
        .task {
            guard model.recentFixture == nil else { return }
            sessions = (try? await store.recentSessions(projectPath: project?.path)) ?? []
        }
    }
}

private struct RecentRow: View {
    let session: RecentSession
    let showProject: Bool
    let resume: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: resume) {
                HStack(spacing: 10) {
                    AgentIcon(session.agentType, size: 14)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(session.display.isEmpty ? "Untitled session" : session.display)
                            .font(.ui(12))
                            .foregroundStyle(Theme.gray300)
                            .lineLimit(1)
                        Text(subline).font(.ui(10)).foregroundStyle(Theme.gray600).lineLimit(1)
                    }
                    Spacer(minLength: 8)
                    if session.canResumeExact {
                        Text("resume").font(.ui(10)).foregroundStyle(Theme.gray700).opacity(hover ? 1 : 0)
                    } else {
                        Text("history only").font(.ui(10)).foregroundStyle(Theme.gray600)
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(hover ? Theme.white(0.04) : .clear))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .opacity(session.canResumeExact ? 1 : 0.75)
        }
    }

    private var subline: String {
        var parts: [String] = []
        if showProject { parts.append(URL(fileURLWithPath: session.projectPath).lastPathComponent) }
        parts.append(timeAgo(session.timestamp))
        if !session.activity.isEmpty { parts.append(session.activity) }
        return parts.joined(separator: " · ")
    }
}

/// "just now", "5m ago", "3h ago", "2d ago", then the date.
func timeAgo(_ ms: Double, now: Date = Date()) -> String {
    let diff = now.timeIntervalSince1970 - ms / 1000
    let minutes = Int(diff / 60)
    if minutes < 1 { return "just now" }
    if minutes < 60 { return "\(minutes)m ago" }
    let hours = minutes / 60
    if hours < 24 { return "\(hours)h ago" }
    let days = hours / 24
    if days < 7 { return "\(days)d ago" }
    return Date(timeIntervalSince1970: ms / 1000).formatted(date: .numeric, time: .omitted)
}

// MARK: Shortcuts

/// KeyboardShortcutsPanel.tsx (⌘/).
struct ShortcutsPanel: View {
    @Bindable var model: AppModel

    static let groups: [(String, [(String, String)])] = [
        ("Navigation", [("Next card", "⌘]"), ("Previous card", "⌘["), ("Close / deselect", "Esc"),
                        ("Jump to card by position", "⌘1-9")]),
        ("Sessions", [("New session", "⌘N"), ("Rename session", "F2")]),
        ("View", [("Command palette", "⌘K"), ("Keyboard shortcuts", "⌘/"), ("Settings", "⌘,"),
                  ("Toggle sidebar", "⌘B"), ("New terminal session", "Ctrl+`"), ("Sessions view", "⌘S"),
                  ("Tasks view", "⌘T"), ("Workflows view", "⌘⇧W"), ("View options", "⌘J"),
                  ("Fit the whole workflow on the canvas", "1")]),
        ("Filters", [("Show all", "⌥1"), ("Show running", "⌥2"), ("Show waiting", "⌥3"), ("Show idle", "⌥4"),
                     ("Show errors", "⌥5")]),
    ]

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Keyboard Shortcuts").font(.ui(14, weight: .medium)).foregroundStyle(.white)
                Spacer()
                Text("ESC")
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(Theme.gray600)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(RoundedRectangle(cornerRadius: Theme.radius).fill(Theme.white(0.04)))
                    .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.hairline, lineWidth: 1))
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 16)
            .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    ForEach(Self.groups, id: \.0) { title, rows in
                        VStack(alignment: .leading, spacing: 0) {
                            groupLabel(title)
                            ForEach(rows, id: \.0) { label, key in
                                row(label) { kbd(key) }
                            }
                        }
                    }
                    VStack(alignment: .leading, spacing: 0) {
                        groupLabel("Tips")
                        row("Rename inline") {
                            Text("Double-click card title")
                                .font(.system(size: 11, design: .monospaced))
                                .foregroundStyle(Theme.gray500)
                        }
                    }
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 16)
            }
            .frame(maxHeight: 400)
        }
        .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusXl))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusXl).strokeBorder(Theme.white(0.08), lineWidth: 1))
        .clipShape(RoundedRectangle(cornerRadius: Theme.radiusXl))
        .shadow(color: .black.opacity(0.5), radius: 24, y: 12)
    }

    private func groupLabel(_ text: String) -> some View {
        Text(text.uppercased())
            .font(.ui(11, weight: .medium))
            .tracking(0.5)
            .foregroundStyle(Theme.gray500)
            .padding(.bottom, 8)
    }

    private func row<K: View>(_ label: String, @ViewBuilder key: () -> K) -> some View {
        HStack {
            Text(label).font(.ui(13)).foregroundStyle(Theme.gray300)
            Spacer()
            key()
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
    }

    private func kbd(_ key: String) -> some View {
        Text(key)
            .font(.system(size: 11, design: .monospaced))
            .foregroundStyle(Theme.gray400)
            .padding(.horizontal, 8)
            .padding(.vertical, 2)
            .background(RoundedRectangle(cornerRadius: Theme.radius).fill(Theme.white(0.04)))
            .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.hairline, lineWidth: 1))
    }
}

// MARK: Restored banner

/// SessionRestoredBanner.tsx: sessions from the last run that were not reopened.
struct RestoredBanner: View {
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions

    var body: some View {
        let cold = store.coldRestored
        if !model.bannerDismissed, store.defaults?.reopenSessions == false, !cold.isEmpty {
            let projects = Set(cold.map(\.session.projectName))
            HStack(spacing: 12) {
                LucideIcon(.rotateCcw, size: 16).foregroundStyle(Theme.gray400)
                (Text("\(cold.count) previous session\(cold.count == 1 ? "" : "s") can be restored")
                    .foregroundColor(Theme.gray300)
                    + Text(projects.count == 1 ? " · \(projects.first ?? "")" : " · \(projects.count) projects")
                    .foregroundColor(Theme.gray500))
                    .font(.ui(14))
                    .lineLimit(1)
                Spacer(minLength: 8)
                Button { actions.restoreAll() } label: {
                    Text("Restore")
                        .font(.ui(12, weight: .medium))
                        .foregroundStyle(.black)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 4)
                        .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.bronzo))
                }
                .buttonStyle(.plain)
                IconButton(.x, size: 14, padding: 4, color: Theme.gray500, hoverColor: Theme.gray300, help: "Dismiss") {
                    model.bannerDismissed = true
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .background(Theme.white(0.03), in: RoundedRectangle(cornerRadius: Theme.radiusLg))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.08), lineWidth: 1))
            .padding(.horizontal, 16)
            .padding(.top, 16)
        }
    }
}

// MARK: Headless

/// The sessions with no pane: workflow steps and other background agents (useFilteredHeadless).
@MainActor
func filteredHeadless(_ model: AppModel, _ store: VornStore) -> [HeadlessSession] {
    guard store.defaults?.showHeadlessAgents != false else { return [] }
    let workspaceProjects = Set(store.projects.filter { $0.workspace == model.activeWorkspace }.map(\.name))
    return store.headless.filter { s in
        if store.dismissedHeadless.contains(s.id) { return false }
        if let p = model.activeProject { if s.projectName != p { return false } }
        else if !workspaceProjects.contains(s.projectName) { return false }
        return model.statusFilter == .all || model.statusFilter.matches(s.filterStatus)
    }
}

/// HeadlessBadge.tsx: ⚡ and a count, a dot while any run.
struct HeadlessBadge: View {
    @Bindable var model: AppModel
    let store: VornStore

    var body: some View {
        let list = filteredHeadless(model, store)
        if !list.isEmpty {
            let running = list.filter(\.isRunning).count
            let errors = list.filter(\.isError).count
            Hovering { hover in
                Button { model.toggle(.headless) } label: {
                    HStack(spacing: 6) {
                        LucideIcon(.zap, size: 12, strokeWidth: 1.5)
                        Text("\(list.count)").font(.system(size: 11, weight: .medium, design: .monospaced))
                    }
                    .font(.ui(11, weight: .medium))
                    .foregroundStyle(hover ? Color.white : Theme.gray300)
                    .padding(.horizontal, 8)
                    .frame(height: 26)
                    .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                    .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd)
                        .strokeBorder(hover ? Theme.white(0.12) : Theme.hairline, lineWidth: 1))
                    .overlay(alignment: .topTrailing) {
                        if running > 0 || errors > 0 {
                            Circle().fill(running > 0 ? Theme.green500 : Theme.red500)
                                .frame(width: 6, height: 6).offset(x: 2, y: -2)
                        }
                    }
                }
                .buttonStyle(.plain)
            }
            .help(tooltip(list.count, running, errors))
            .controlAnchor("headless")
        }
    }

    private func tooltip(_ total: Int, _ running: Int, _ errors: Int) -> String {
        var parts = ["\(total) headless"]
        if running > 0 { parts.append("\(running) running") } else if errors > 0 { parts.append("\(errors) errored") }
        return parts.joined(separator: " · ")
    }
}

struct HeadlessPanel: View {
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(filteredHeadless(model, store)) { s in
                HeadlessPill(session: s, kill: { actions.killHeadless(s.id) },
                             dismiss: { store.dismissedHeadless.insert(s.id) })
            }
        }
        .padding(6)
        .frame(minWidth: 280, alignment: .leading)
        .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.08), lineWidth: 1))
        .shadow(color: .black.opacity(0.4), radius: 8, y: 4)
    }
}

/// HeadlessPill.tsx, collapsed: status, agent, name, branch, duration, exit code; kill and hide on hover.
struct HeadlessPill: View {
    let session: HeadlessSession
    let kill: () -> Void
    let dismiss: () -> Void
    @State private var hovered = false

    static let square = LucideGlyph([.rect(x: 3, y: 3, width: 18, height: 18, rx: 2)])

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { ctx in
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Circle()
                        .fill(session.isRunning ? Theme.green500 : session.isError ? Theme.red500 : Theme.gray500)
                        .frame(width: 6, height: 6)
                    AgentIcon(session.agentType, size: 14)
                    Text(session.displayName ?? session.projectName)
                        .font(.ui(11, weight: .medium))
                        .foregroundStyle(Theme.gray200)
                        .lineLimit(1)
                        .frame(maxWidth: 100, alignment: .leading)
                        .fixedSize(horizontal: true, vertical: false)
                    Text("·").font(.ui(10)).foregroundStyle(Theme.gray600)
                    if let b = session.branch {
                        HStack(spacing: 2) {
                            LucideIcon(.gitBranch, size: 9, strokeWidth: 1.5)
                            Text(b).lineLimit(1)
                        }
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundStyle(Theme.gray500)
                        .frame(maxWidth: 90, alignment: .leading)
                        .fixedSize(horizontal: true, vertical: false)
                    }
                    Spacer(minLength: 4)
                    if hovered {
                        HStack(spacing: 4) {
                            if session.isRunning {
                                IconButton(Self.square, size: 10, padding: 2, radius: Theme.radius, color: Theme.red400,
                                           hoverColor: Theme.red400, hoverFill: Theme.red500.opacity(0.1),
                                           help: "Kill process", action: kill)
                            }
                            IconButton(.x, size: 10, padding: 2, radius: Theme.radius, color: Theme.gray500,
                                       hoverColor: Theme.gray300, hoverFill: Theme.white(0.06), help: "Hide", action: dismiss)
                        }
                    } else {
                        Text(duration(now: ctx.date))
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundStyle(Theme.gray500)
                        if let code = session.exitCode {
                            Text("\(code == 0 ? "✓" : "✗") \(code)")
                                .font(.system(size: 9, design: .monospaced))
                                .foregroundStyle(code == 0 ? Theme.green400 : Theme.red400)
                                .padding(.horizontal, 4)
                                .padding(.vertical, 1)
                                .background(RoundedRectangle(cornerRadius: Theme.radius)
                                    .fill((code == 0 ? Theme.green500 : Theme.red500).opacity(0.1)))
                        }
                    }
                }
                if let wf = session.workflowName {
                    HStack(spacing: 4) {
                        LucideIcon(.workflow, size: 9, strokeWidth: 1.5)
                        Text(wf).lineLimit(1)
                    }
                    .font(.ui(10))
                    .foregroundStyle(Theme.gray500)
                    .padding(.leading, 20)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 4)
            .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(border, lineWidth: 1))
            .opacity(session.isRunning ? 1 : 0.65)
            .onHover { hovered = $0 }
        }
    }

    private var border: Color {
        if session.isRunning { return Theme.green500.opacity(0.3) }
        if session.isError { return Theme.red500.opacity(0.25) }
        return hovered ? Theme.white(0.12) : Theme.hairline
    }

    private func duration(now: Date) -> String {
        let end = session.isRunning ? now.timeIntervalSince1970 * 1000 : (session.endedAt ?? now.timeIntervalSince1970 * 1000)
        let s = max(0, Int((end - session.startedAt) / 1000))
        if s < 60 { return "\(s)s" }
        let m = s / 60, rs = s % 60
        if m < 60 { return rs > 0 ? "\(m)m\(rs)s" : "\(m)m" }
        return "\(m / 60)h\(m % 60)m"
    }
}

// MARK: Context menus

/// One row of a context menu or its submenu.
private struct MenuItem: Identifiable {
    enum Kind { case action, header, submenu }
    var id: String
    var label: String
    var kind: Kind = .action
    var detail: String?
    var shortcut: String?
    var separator = false
    var mono = false
    var icon: AnyView
    var action: () -> Void = {}
    var children: () -> [MenuItem] = { [] }
}

/// CardContextMenu.tsx and GridContextMenu.tsx: new sessions and terminals where you clicked, workflows.
struct SessionContextMenu: View {
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions
    let features: FeatureStores
    let state: ContextMenuState
    let window: CGSize
    @State private var openSub: String?

    static let width: CGFloat = 220
    static let rowHeight: CGFloat = 30

    var body: some View {
        let items = self.items
        let height = Self.height(items) + 8
        let x = max(8, min(state.point.x, window.width - Self.width - 8))
        let y = max(8, min(state.point.y, window.height - height - 8))
        let sub = openSub ?? state.submenu
        ZStack(alignment: .topLeading) {
            MenuPanel(minWidth: Self.width) {
                ForEach(items) { item in
                    if item.separator { MenuRule() }
                    entry(item, highlighted: item.id == sub)
                }
            }
            .fixedSize()
            .offset(x: x, y: y)
            if let sub, let i = items.firstIndex(where: { $0.id == sub }) {
                let children = items[i].children()
                let subW: CGFloat = state.target == .grid ? 240 : 220
                let subH = Self.height(children) + 8
                let left = x + Self.width + 4 + subW > window.width - 8 ? x - subW - 4 : x + Self.width + 4
                let top = max(8, min(y + 4 + Self.height(Array(items[..<i])) + (items[i].separator ? 9 : 0) - 4,
                                     window.height - subH - 8))
                MenuPanel(minWidth: subW) {
                    ForEach(children) { child in
                        if child.separator { MenuRule() }
                        if child.kind == .header {
                            MenuGroupHeader(child.label) { child.icon }
                        } else {
                            subEntry(child)
                        }
                    }
                }
                .fixedSize()
                .offset(x: left, y: top)
                .onHover { if $0 { openSub = sub } }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .task { if state.target == .grid, features.workflows.workflows.isEmpty { await features.workflows.reload() } }
    }

    private static func height(_ items: [MenuItem]) -> CGFloat {
        items.reduce(0) { h, item in
            h + (item.separator ? 9 : 0) + (item.kind == .header ? 26 : rowHeight)
        }
    }

    private func entry(_ item: MenuItem, highlighted: Bool) -> some View {
        MenuEntry(item.label, shortcut: item.shortcut, submenu: item.kind == .submenu, highlighted: highlighted,
                  onHover: { inside in if inside { openSub = item.kind == .submenu ? item.id : nil } },
                  action: {
                      if item.kind == .submenu {
                          openSub = openSub == item.id ? nil : item.id
                      } else {
                          close()
                          item.action()
                      }
                  }) { item.icon }
    }

    private func subEntry(_ item: MenuItem) -> some View {
        Hovering { hover in
            Button {
                close()
                item.action()
            } label: {
                HStack(spacing: 10) {
                    item.icon.frame(width: 12, height: 12)
                    Text(item.label)
                        .font(item.mono ? .system(size: 12, design: .monospaced) : .ui(12))
                        .lineLimit(1)
                    Spacer(minLength: 8)
                    if let d = item.detail, !d.isEmpty {
                        Text(d).font(.ui(10)).foregroundStyle(Theme.green400.opacity(0.7))
                    }
                }
                .foregroundStyle(Theme.gray300)
                .padding(.leading, state.target == .grid ? 28 : 12)
                .padding(.trailing, 12)
                .frame(height: Self.rowHeight)
                .background(hover ? Theme.white(0.06) : .clear)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }

    private func close() {
        model.contextMenu = nil
        model.overlay = nil
    }

    private var items: [MenuItem] {
        switch state.target {
        case .card(let id): store.session(id).map(cardItems) ?? []
        case .grid: gridItems
        }
    }

    private var defaultAgent: AgentType { actions.defaultAgent }

    private func cardItems(_ s: TerminalSession) -> [MenuItem] {
        let project = store.project(named: s.projectName) ?? Project(name: s.projectName, path: s.projectPath)
        let worktree = s.isWorktree == true ? s.worktreePath.map {
            Worktree(path: $0, branch: s.branch ?? "", isMain: false, name: s.worktreeName ?? "")
        } : nil
        var items = [
            MenuItem(id: "new-session", label: "New session", icon: AnyView(AgentIcon(defaultAgent, size: 14)),
                     action: { actions.newSession(project, worktree) }),
            MenuItem(id: "new-terminal", label: "New terminal",
                     icon: AnyView(LucideIcon(.terminal, size: 14).foregroundStyle(Theme.gray400)),
                     action: { actions.newTerminal(project, worktree) }),
        ]
        if store.installedAgents.count > 1 {
            items.append(MenuItem(id: "new-with", label: "New session with…", kind: .submenu,
                                  icon: AnyView(AgentIcon(defaultAgent, size: 14)), children: {
                store.installedAgents.map { a in
                    MenuItem(id: a.rawValue, label: a.displayName, icon: AnyView(AgentIcon(a, size: 12)),
                             action: { actions.newSession(project, worktree, agent: a) })
                }
            }))
        }
        return items
    }

    private var workspaceProjects: [Project] { store.projects.filter { $0.workspace == model.activeWorkspace } }

    private var gridItems: [MenuItem] {
        var items: [MenuItem] = []
        if !workspaceProjects.isEmpty {
            items.append(MenuItem(id: "session-in", label: "New session in…", kind: .submenu,
                                  icon: AnyView(AgentIcon(defaultAgent, size: 14)), children: { places(session: true) }))
            items.append(MenuItem(id: "terminal-in", label: "New terminal in…", kind: .submenu,
                                  icon: AnyView(LucideIcon(.terminal, size: 14).foregroundStyle(Theme.gray400)),
                                  children: { places(session: false) }))
        }
        items.append(MenuItem(id: "new", label: "New session...", shortcut: "⌘N", separator: !workspaceProjects.isEmpty,
                              icon: AnyView(LucideIcon(.plus, size: 14).foregroundStyle(Theme.gray500)),
                              action: { actions.newSession(actions.activeProject ?? workspaceProjects.first, actions.activeWorktree) }))
        let workflows = features.workflows.workspaceWorkflows.filter { !isContextual($0) }
        if !workflows.isEmpty {
            items.append(MenuItem(id: "run-workflow", label: "Run workflow", kind: .submenu,
                                  icon: AnyView(LucideIcon(.workflow, size: 14).foregroundStyle(Theme.gray500)), children: {
                workflows.map { wf in
                    MenuItem(id: wf.id, label: wf.name,
                             icon: AnyView(ProjectIcon(icon: wf.icon, color: wf.iconColor, size: 12)),
                             action: { features.workflows.runNow(wf) })
                }
            }))
        }
        return items
    }

    private func isContextual(_ wf: WorkflowDefinition) -> Bool {
        guard let config = wf.triggerNode?.config else { return false }
        return config["triggerType"]?.stringValue == "manual" && config["contextual"]?.boolValue == true
    }

    /// Every project, its main branch and worktrees with their session counts.
    private func places(session: Bool) -> [MenuItem] {
        var subs: [MenuItem] = []
        for p in workspaceProjects {
            let wts = store.worktrees[p.path] ?? []
            let main = wts.first { $0.isMain }
            let others = wts.filter { !$0.isMain }
            let icon = AnyView(ProjectIcon(icon: p.icon, color: p.iconColor, size: 12))
            let open: (Worktree?) -> () -> Void = { wt in
                { session ? actions.newSession(p, wt) : actions.newTerminal(p, wt) }
            }
            guard main != nil || !others.isEmpty else {
                subs.append(MenuItem(id: p.path, label: p.name, separator: !subs.isEmpty, icon: icon, action: open(nil)))
                continue
            }
            subs.append(MenuItem(id: "h:" + p.path, label: p.name, kind: .header, separator: !subs.isEmpty, icon: icon))
            if let main {
                subs.append(MenuItem(id: main.path, label: main.branch, detail: count(main, p), mono: true,
                                     icon: AnyView(LucideIcon(.gitBranch, size: 12).foregroundStyle(Theme.gray400)),
                                     action: open(main)))
            }
            for wt in others {
                subs.append(MenuItem(id: wt.path, label: wt.name, detail: count(wt, p), mono: true,
                                     icon: AnyView(LucideIcon(.folderGit2, size: 12).foregroundStyle(Theme.amber400.opacity(0.7))),
                                     action: open(wt)))
            }
            if session {
                subs.append(MenuItem(id: "wt:" + p.path, label: "New worktree", mono: true,
                                     icon: AnyView(LucideIcon(.plus, size: 12).foregroundStyle(Theme.gray500)),
                                     action: { actions.newWorktreeSession(p) }))
            }
        }
        return subs
    }

    /// formatSessionCount: nothing for none, "1 session", "N sessions".
    private func count(_ wt: Worktree, _ p: Project) -> String {
        let n = store.sessions.filter { s in
            wt.isMain ? s.projectPath == p.path && s.isWorktree != true : s.worktreePath == wt.path
        }.count
        return n == 0 ? "" : n == 1 ? "1 session" : "\(n) sessions"
    }
}
