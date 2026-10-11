import AppKit
import SwiftUI
import VornCore
import VornTerminals
import VornUI

/// worktree-names.ts: an adjective and a noun.
enum WorktreeNames {
    static let adjectives = [
        "gilded", "marble", "ornate", "sacred", "divine", "golden", "silver", "crimson", "ivory", "velvet", "noble",
        "royal", "regal", "ancient", "baroque", "classical", "tuscan", "florentine", "venetian", "emerald", "amber",
        "obsidian", "bronze", "sienna", "scarlet",
    ]
    static let nouns = [
        "fresco", "madrigal", "etching", "sketch", "triptych", "inkwell", "study", "canvas", "palette", "tableau",
        "vellum", "relic", "mosaic", "statue", "chapel", "garden", "fountain", "frieze", "archive", "folio",
        "portrait", "scroll", "chronicle", "stanza", "muse",
    ]

    static func generate() -> String { "\(adjectives.randomElement()!)-\(nouns.randomElement()!)" }
}

/// A `py-1.5` block of a dropdown: an uppercase label over its rows, ruled above when not first.
struct OptionSection<Content: View>: View {
    let label: String
    var rule = false
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            MenuSectionLabel(label)
            content
        }
        .padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .top) { if rule { Rectangle().fill(Theme.hairline).frame(height: 1) } }
    }
}

// MARK: Filter & sort

/// ProjectsSectionToolbar.tsx's dropdown.
struct SidebarOptionsPanel: View {
    @Bindable var model: AppModel

    var body: some View {
        let mode = model.sidebarViewMode
        MenuPanel(width: 200, border: 0.08, verticalPadding: 0) {
            OptionSection(label: "Show") {
                ForEach(SidebarViewMode.allCases, id: \.self) { m in
                    MenuOptionRow(m.label, selected: mode == m) { model.sidebarViewMode = m }
                }
            }
            if mode != .sessionsFlat {
                OptionSection(label: "Filter", rule: true) {
                    ForEach(WorktreeFilter.allCases, id: \.self) { f in
                        MenuOptionRow(f.label, selected: model.worktreeFilter == f) { model.worktreeFilter = f }
                    }
                }
                OptionSection(label: "Sort projects", rule: true) {
                    ForEach(ProjectSort.allCases, id: \.self) { s in
                        MenuOptionRow(s.label, selected: model.projectSort == s) { model.projectSort = s }
                    }
                }
            }
            if mode != .sessions && mode != .sessionsFlat {
                OptionSection(label: "Sort worktrees", rule: true) {
                    ForEach(WorktreeSort.allCases, id: \.self) { s in
                        MenuOptionRow(s.label, selected: model.worktreeSort == s) { model.worktreeSort = s }
                    }
                }
            }
        }
    }
}

/// The section header's list-filter button: white while open or off its defaults, with a dot for the latter.
struct SidebarOptionsButton: View {
    @Bindable var model: AppModel

    var body: some View {
        let open = model.overlay == .sidebarOptions
        let active = model.hasSidebarOptions
        Hovering { hover in
            Button { model.toggle(.sidebarOptions) } label: {
                LucideIcon(.listFilter, size: 13, strokeWidth: 1.5)
                    .foregroundStyle(open || active || hover ? Color.white : Theme.gray600)
                    .padding(2)
                    .background(RoundedRectangle(cornerRadius: Theme.radius)
                        .fill(open || (hover && !active) ? Theme.white(0.08) : .clear))
                    .overlay(alignment: .topTrailing) {
                        if active && !open {
                            Circle().fill(Theme.inkSecondary).frame(width: 6, height: 6).offset(x: 2, y: -2)
                        }
                    }
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .tooltip("Filter & sort")
            .accessibilityLabel("Filter & sort")
        }
        .controlAnchor("sidebarOptions")
    }
}

// MARK: Project menu

/// ProjectContextMenu.tsx: Edit Project, and Delete Project behind a second click.
struct ProjectMenuPanel: View {
    let project: String
    @Bindable var model: AppModel
    let actions: ShellActions
    @State private var confirmDelete = false

    var body: some View {
        MenuPanel(minWidth: 140, border: 0.08) {
            item(.pencil, "Edit Project", color: Theme.gray300) {
                model.overlay = .addProject(editing: project)
            }
            if confirmDelete {
                item(.trash2, "Confirm delete?", color: Theme.danger, fill: Theme.danger.opacity(0.1)) {
                    model.overlay = nil
                    actions.removeProject(project)
                }
            } else {
                item(.trash2, "Delete Project", color: Theme.danger.opacity(0.8)) { confirmDelete = true }
            }
        }
        .fixedSize()
    }

    private func item(_ icon: LucideGlyph, _ label: String, color: Color, fill: Color = .clear,
                      action: @escaping () -> Void) -> some View {
        Hovering { hover in
            Button(action: action) {
                HStack(spacing: 8) {
                    LucideIcon(icon, size: 12, strokeWidth: 1.5)
                    Text(label)
                    Spacer(minLength: 0)
                }
                .font(.ui(13))
                .foregroundStyle(hover && fill == .clear ? (color == Theme.gray300 ? .white : Theme.danger) : color)
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .background(fill == .clear ? (hover ? Theme.white(0.06) : .clear) : fill)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }
}

// MARK: Add project

/// AddProjectDialog.tsx: a folder, a name, an icon and its colour.
struct AddProjectDialog: View {
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions
    let editing: String?

    @Environment(\.terminalSnapshot) private var snapshot
    @State private var path = ""
    @State private var name = ""
    @State private var icon = "Folder"
    @State private var color = "#6b7280"

    static let icons: [(String, String)] = [
        ("Folder", "Folder"), ("FolderGit2", "Git Repo"), ("Code", "Code"), ("Globe", "Web"), ("Database", "Database"),
        ("Server", "Server"), ("Smartphone", "Mobile"), ("Package", "Package"), ("FileCode", "Script"),
        ("Terminal", "Terminal"), ("Cpu", "Hardware"), ("Cloud", "Cloud"), ("Shield", "Security"), ("Zap", "Fast"),
        ("Workflow", "Workflow"), ("Gamepad2", "Game"), ("Music", "Audio"), ("Image", "Image"), ("BookOpen", "Docs"),
        ("FlaskConical", "Lab"), ("Rocket", "Launch"), ("github", "GitHub"), ("linear", "Linear"), ("mcp", "MCP"),
    ]
    static let palette = ["#6b7280", "#3b82f6", "#8b5cf6", "#ec4899", "#ef4444", "#f97316", "#eab308", "#22c55e"]

    private var canSubmit: Bool {
        !name.trimmingCharacters(in: .whitespaces).isEmpty && !path.trimmingCharacters(in: .whitespaces).isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 2) {
                Text(editing == nil ? "Add Project" : "Edit Project")
                    .font(.ui(18, weight: .medium))
                    .foregroundStyle(.white)
                Text(editing == nil ? "Select a folder and customize your project" : "Update your project settings")
                    .font(.ui(14))
                    .foregroundStyle(Theme.gray500)
            }
            .padding(.horizontal, 24)
            .padding(.vertical, 16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }

            VStack(alignment: .leading, spacing: 20) {
                field("Project Folder") {
                    HStack(spacing: 8) {
                        Hovering { hover in
                            Button(action: browse) {
                                HStack(spacing: 8) {
                                    LucideIcon(.folderOpen, size: 16, strokeWidth: 1.5)
                                    Text("Browse")
                                }
                                .font(.ui(14))
                                .foregroundStyle(Theme.gray200)
                                .padding(.horizontal, 16)
                                .padding(.vertical, 10)
                                .background(RoundedRectangle(cornerRadius: Theme.radiusLg)
                                    .fill(Theme.white(hover ? 0.1 : 0.06)))
                                .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg)
                                    .strokeBorder(Theme.white(0.08), lineWidth: 1))
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                        }
                        Text(path.isEmpty ? "No folder selected" : path)
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .font(.ui(14))
                            .foregroundStyle(Theme.gray400)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .inputBox()
                    }
                }
                field("Project Name") {
                    Group {
                        if snapshot {
                            Text(name.isEmpty ? "my-project" : name)
                                .foregroundStyle(name.isEmpty ? Theme.gray600 : Theme.gray200)
                        } else {
                            TextField("", text: $name, prompt: Text("my-project").foregroundStyle(Theme.gray600))
                                .textFieldStyle(.plain)
                                .foregroundStyle(Theme.gray200)
                                .onSubmit(submit)
                        }
                    }
                    .font(.ui(14))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .inputBox()
                }
                field("Icon") {
                    LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 6), count: 8), spacing: 6) {
                        ForEach(Self.icons, id: \.0) { name, label in
                            iconButton(name, label)
                        }
                    }
                }
                field("Color") {
                    HStack(spacing: 8) {
                        ForEach(Self.palette, id: \.self) { swatch in
                            Button { color = swatch } label: {
                                Circle()
                                    .fill(Color(css: swatch) ?? .gray)
                                    .frame(width: 24, height: 24)
                                    .padding(2)
                                    .overlay(Circle().strokeBorder(color == swatch ? Color.white : .clear, lineWidth: 2))
                                    .scaleEffect(color == swatch ? 1.1 : 1)
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel("Color \(swatch)")
                        }
                        HStack(spacing: 8) {
                            ProjectIcon(icon: icon, color: color, size: 20)
                            Text("Preview").font(.ui(12)).foregroundStyle(Theme.gray500)
                        }
                        .padding(.leading, 12)
                    }
                }
            }
            .padding(24)

            HStack(spacing: 12) {
                Spacer(minLength: 0)
                footerButton("Cancel", primary: false, action: close)
                footerButton(editing == nil ? "Add Project" : "Save", primary: true, action: submit)
                    .disabled(!canSubmit)
                    .opacity(canSubmit ? 1 : 0.3)
            }
            .padding(.horizontal, 24)
            .padding(.vertical, 16)
            .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
        }
        .frame(width: 520)
        .background(Theme.surfaceOverlay, in: RoundedRectangle(cornerRadius: Theme.radiusXl))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusXl).strokeBorder(Theme.white(0.08), lineWidth: 1))
        .clipShape(RoundedRectangle(cornerRadius: Theme.radiusXl))
        .shadow(color: .black.opacity(0.5), radius: 25, y: 12)
        .onAppear(perform: load)
    }

    private func field<C: View>(_ label: String, @ViewBuilder content: () -> C) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(label.uppercased())
                .font(.ui(12, weight: .medium))
                .tracking(0.6)
                .foregroundStyle(Theme.gray500)
            content()
        }
    }

    private func iconButton(_ name: String, _ label: String) -> some View {
        let on = icon == name
        return Hovering { hover in
            Button { icon = name } label: {
                ProjectIcon(icon: name, color: on ? color : "#9ca3af", size: 16)
                    .frame(maxWidth: .infinity)
                    .padding(8)
                    .background(RoundedRectangle(cornerRadius: Theme.radiusLg)
                        .fill(on ? Theme.white(0.08) : hover ? Theme.white(0.04) : .clear))
                    .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg)
                        .strokeBorder(on ? Theme.white(0.2) : .clear, lineWidth: 1))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(label)
            .accessibilityLabel(label)
        }
    }

    private func footerButton(_ label: String, primary: Bool, action: @escaping () -> Void) -> some View {
        Hovering { hover in
            Button(action: action) {
                Text(label)
                    .font(.ui(14, weight: primary ? .medium : .regular))
                    .foregroundStyle(primary ? Color.white : hover ? Theme.gray200 : Theme.gray400)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 8)
                    .background(RoundedRectangle(cornerRadius: Theme.radiusLg)
                        .fill(primary ? Theme.white(hover ? 0.15 : 0.1) : Theme.white(hover ? 0.08 : 0.04)))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }

    private func load() {
        guard let editing, let p = store.project(named: editing) else { return }
        name = p.name
        path = p.path
        icon = p.icon ?? "Folder"
        color = p.iconColor ?? "#6b7280"
    }

    private func browse() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        path = url.path
        if name.isEmpty { name = url.lastPathComponent }
    }

    private func submit() {
        guard canSubmit else { return }
        let original = editing.flatMap { store.project(named: $0) }
        var project = Project(name: name.trimmingCharacters(in: .whitespaces),
                              path: path.trimmingCharacters(in: .whitespaces),
                              preferredAgents: original?.preferredAgents ?? ["claude"], icon: icon, iconColor: color,
                              workspaceId: original?.workspaceId ?? model.activeWorkspace)
        project.hostIds = original?.hostIds ?? ["local"]
        actions.saveProject(project, replacing: editing)
        close()
    }

    private func close() { model.overlay = nil }
}

private extension View {
    /// `px-4 py-2.5 bg-white/[0.03] border border-white/[0.06] rounded-lg`.
    func inputBox() -> some View {
        padding(.horizontal, 16)
            .padding(.vertical, 10)
            .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.white(0.03)))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.06), lineWidth: 1))
    }
}

// MARK: Remove worktree

/// WorktreeCleanupDialog.tsx: confirms removing a worktree that has sessions or changes.
struct WorktreeCleanupDialog: View {
    let projectPath: String
    let worktreePath: String
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions

    private enum Dirty { case checking, dirty, clean, unknown }
    @State private var dirty = Dirty.checking

    var body: some View {
        let sessions = store.sessions.filter { $0.worktreePath == worktreePath }.count
        let warn = dirty == .dirty || dirty == .unknown
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 12) {
                LucideIcon(.folderGit2, size: 18).foregroundStyle(Theme.amber400)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Remove Worktree").font(.ui(13, weight: .medium)).foregroundStyle(.white)
                    Text(sessions > 0 ? "This worktree has active sessions" : "Remove this worktree from disk?")
                        .font(.ui(11))
                        .foregroundStyle(Theme.gray500)
                }
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }

            Text(worktreePath)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(Theme.gray500)
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.white(0.03)))
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.06), lineWidth: 1))
                .padding(.horizontal, 20)
                .padding(.vertical, 12)

            if sessions > 0 {
                warning("\(sessions) session\(sessions > 1 ? "s are" : " is") running in this worktree. "
                    + "\(sessions > 1 ? "They" : "It") will be terminated.")
            }
            if warn {
                warning(dirty == .dirty
                    ? "This worktree has uncommitted changes that will be permanently lost."
                    : "Unable to check for uncommitted changes. Removal will use --force.")
            }

            HStack(spacing: 8) {
                Spacer(minLength: 0)
                Hovering { hover in
                    Button { model.overlay = nil } label: {
                        Text("Cancel")
                            .font(.ui(11))
                            .foregroundStyle(Theme.gray300)
                            .padding(.horizontal, 12)
                            .padding(.vertical, 6)
                            .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.white(hover ? 0.08 : 0.04)))
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
                Hovering { hover in
                    let strong = warn || sessions > 0
                    Button {
                        model.overlay = nil
                        actions.removeWorktree(projectPath: projectPath, worktreePath: worktreePath, force: warn)
                    } label: {
                        HStack(spacing: 6) {
                            LucideIcon(.trash2, size: 12)
                            Text(dirty == .checking ? "Checking…" : sessions > 0 ? "Close sessions & remove"
                                : warn ? "Remove anyway" : "Remove")
                        }
                        .font(.ui(11))
                        .foregroundStyle(strong ? Theme.red300 : Theme.red400)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(RoundedRectangle(cornerRadius: Theme.radiusMd)
                            .fill(Theme.red500.opacity(strong ? (hover ? 0.25 : 0.15) : (hover ? 0.15 : 0.08))))
                        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd)
                            .strokeBorder(strong ? Theme.red500.opacity(0.3) : .clear, lineWidth: 1))
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .disabled(dirty == .checking)
                    .opacity(dirty == .checking ? 0.5 : 1)
                }
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 12)
            .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
        }
        .frame(width: 420)
        .background(Theme.surfaceRaised, in: RoundedRectangle(cornerRadius: Theme.radiusXl))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusXl).strokeBorder(Theme.white(0.08), lineWidth: 1))
        .clipShape(RoundedRectangle(cornerRadius: Theme.radiusXl))
        .shadow(color: .black.opacity(0.5), radius: 25, y: 12)
        .task(id: worktreePath) {
            dirty = .checking
            switch await store.worktreeDirty(worktreePath) {
            case true?: dirty = .dirty
            case false?: dirty = .clean
            case nil: dirty = .unknown
            }
        }
    }

    private func warning(_ text: String) -> some View {
        HStack(alignment: .top, spacing: 8) {
            LucideIcon(.triangleAlert, size: 14).foregroundStyle(Theme.amber400).padding(.top, 2)
            Text(text).font(.ui(11)).foregroundStyle(Theme.amber300.opacity(0.9))
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.amber500.opacity(0.08)))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.amber500.opacity(0.2), lineWidth: 1))
        .padding(.horizontal, 20)
        .padding(.bottom, 8)
    }
}

// MARK: Flat sessions

/// FlatSessionsSection.tsx: every session in the workspace, most recently active first.
struct FlatSessionsSection: View {
    @Bindable var model: AppModel
    let store: VornStore
    let actions: ShellActions
    @State private var open = true

    var body: some View {
        let names = Set(store.projects.filter { $0.workspace == model.activeWorkspace }.map(\.name))
        let inWorkspace = store.sessions.filter { names.contains($0.projectName) }
        let scope = model.activeProject.flatMap { names.contains($0) ? $0 : nil }
        let sessions = inWorkspace
            .filter { scope == nil || $0.projectName == scope }
            .sorted { (store.lastActivity[$0.id] ?? 0) > (store.lastActivity[$1.id] ?? 0) }
        SectionHeader(title: "Sessions", open: $open) { SidebarOptionsButton(model: model) }
        if open {
            NavRow(icon: .layers, label: "All Projects", badge: inWorkspace.count, active: model.activeProject == nil) {
                model.select(project: nil)
            }
            VStack(alignment: .leading, spacing: 2) {
                if sessions.isEmpty {
                    Text("No active sessions")
                        .font(.ui(11))
                        .foregroundStyle(Theme.gray600)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                }
                ForEach(sessions) { s in
                    SessionItem(session: s, model: model, store: store, actions: actions, showBranch: true)
                }
            }
            .padding(.top, 4)
        }
    }
}
