import SwiftUI
import VornCore
import VornUI

/// The git and editor calls a card's status bar makes, bound by the shell.
public struct GitActions {
    public var listBranches: (String) async -> [String] = { _ in [] }
    public var listRemoteBranches: (String) async -> [String] = { _ in [] }
    /// Checks out `branch` in the session's tree; nil on success, else why not.
    public var checkout: (TerminalSession, String) async -> String? = { _, _ in nil }
    public var detectIDEs: () async -> [DetectedIDE] = { [] }
    public var openIn: (String, String) -> Void = { _, _ in }
    public var toast: (String, Bool) -> Void = { _, _ in }

    public init() {}
}

/// BranchChip.tsx: worktree and branch, opening the branch switcher.
public struct BranchChip: View {
    public enum Size { case sm, md }
    let session: TerminalSession
    let git: GitActions
    var size: Size = .sm
    @State private var open = false
    @State private var switching = false

    public init(session: TerminalSession, git: GitActions, size: Size = .sm) {
        self.session = session
        self.git = git
        self.size = size
    }

    public var body: some View {
        if let branch = session.branch, !branch.isEmpty {
            let icon: CGFloat = size == .md ? 11 : 10
            let text: CGFloat = size == .md ? 11 : 10
            let maxW: CGFloat = size == .md ? 140 : 110
            Hovering { hover in
                Button { open.toggle() } label: {
                    HStack(spacing: 4) {
                        if session.isWorktree == true, let wt = session.worktreeName {
                            LucideIcon(.folderGit2, size: icon, strokeWidth: 1.5).foregroundStyle(Theme.inkSecondary)
                            Text(wt).font(.system(size: text, design: .monospaced)).foregroundStyle(Theme.inkSecondary)
                                .lineLimit(1).frame(maxWidth: maxW, alignment: .leading)
                            Text("·").foregroundStyle(Theme.gray600)
                        }
                        LucideIcon(.gitBranch, size: icon, strokeWidth: 1.5).foregroundStyle(Theme.gray500)
                        Text(branch).font(.system(size: text, design: .monospaced)).lineLimit(1).truncationMode(.tail)
                            .frame(maxWidth: maxW, alignment: .leading)
                        LucideIcon(.chevronDown, size: icon - 1).foregroundStyle(Theme.gray500)
                    }
                    .foregroundStyle(open || hover ? Theme.gray200 : Theme.gray400)
                    .padding(.horizontal, 4)
                    .padding(.vertical, 2)
                    .background(RoundedRectangle(cornerRadius: Theme.radius).fill(open ? Theme.white(0.08) : hover ? Theme.white(0.06) : .clear))
                    .opacity(switching ? 0.5 : 1)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(switching)
                .accessibilityLabel("Switch branch (current: \(branch))")
            }
            .popover(isPresented: $open, arrowEdge: .bottom) {
                BranchPicker(projectPath: session.projectPath, currentBranch: branch, git: git) { pick in
                    open = false
                    guard pick != branch, !switching else { return }
                    switching = true
                    Task {
                        if let err = await git.checkout(session, pick) {
                            git.toast(err, true)
                        } else {
                            git.toast("Switched to '\(pick)'", false)
                        }
                        switching = false
                    }
                }
            }
        }
    }
}

/// BranchPicker.tsx: filter, local branches, remote ones on request.
public struct BranchPicker: View {
    let projectPath: String
    let currentBranch: String?
    var selectedBranch: String?
    let git: GitActions
    let onSelect: (String) -> Void
    @State private var local: [String] = []
    @State private var remote: [String] = []
    @State private var filter = ""
    @State private var loadingLocal = true
    @State private var loadingRemote = false
    @FocusState private var focused: Bool

    public init(projectPath: String, currentBranch: String?, selectedBranch: String? = nil, git: GitActions,
                onSelect: @escaping (String) -> Void) {
        self.projectPath = projectPath
        self.currentBranch = currentBranch
        self.selectedBranch = selectedBranch
        self.git = git
        self.onSelect = onSelect
    }

    public var body: some View {
        let all = local.map { ($0, false) } + remote.map { ($0, true) }
        let shown = filter.isEmpty ? all : all.filter { $0.0.lowercased().contains(filter.lowercased()) }
        let active = selectedBranch ?? currentBranch
        VStack(spacing: 0) {
            HStack(spacing: 4) {
                TextField("", text: $filter, prompt: Text("Filter branches...").foregroundColor(Theme.gray600))
                    .textFieldStyle(.plain).font(.ui(12)).foregroundStyle(Theme.gray200)
                    .padding(.horizontal, 8).padding(.vertical, 4)
                    .focused($focused)
                Button {
                    guard !loadingRemote else { return }
                    loadingRemote = true
                    Task {
                        let r = await git.listRemoteBranches(projectPath)
                        remote = r.filter { !local.contains($0) }
                        loadingRemote = false
                    }
                } label: {
                    Group {
                        if loadingRemote { SpinningGlyph(size: 10) } else { LucideIcon(.refreshCw, size: 10) }
                    }
                    .foregroundStyle(Theme.gray500).padding(4)
                }
                .buttonStyle(.plain)
                .help("Fetch remotes")
            }
            .padding(8)
            .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }
            ScrollView {
                VStack(spacing: 0) {
                    if loadingLocal {
                        SpinningGlyph(size: 14).foregroundStyle(Theme.gray500).padding(.vertical, 12)
                    } else if shown.isEmpty {
                        Text("No branches found").font(.ui(12)).foregroundStyle(Theme.gray500)
                            .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 12).padding(.vertical, 8)
                    } else {
                        ForEach(shown, id: \.0) { name, isRemote in
                            Hovering { hover in
                                Button { onSelect(name) } label: {
                                    HStack(spacing: 8) {
                                        LucideIcon(.gitBranch, size: 10).foregroundStyle(isRemote ? Theme.blue400 : Theme.gray500)
                                        Text(name).font(.ui(12)).lineHeight(16, size: 12).lineLimit(1)
                                        Spacer(minLength: 0)
                                        if isRemote { Text("remote").font(.ui(9)).foregroundStyle(Theme.blue400.opacity(0.6)) }
                                        if name == currentBranch { Text("current").font(.ui(9)).foregroundStyle(Theme.green400.opacity(0.6)) }
                                    }
                                    .foregroundStyle(active == name ? .white : Theme.gray400)
                                    .padding(.horizontal, 12).padding(.vertical, 6)
                                    .background(hover ? Theme.white(0.06) : active == name ? Theme.white(0.04) : .clear)
                                    .contentShape(Rectangle())
                                }
                                .buttonStyle(.plain)
                            }
                        }
                    }
                }
                .padding(.vertical, 4)
            }
            .frame(maxHeight: 230)
        }
        .frame(minWidth: 220)
        .fixedSize(horizontal: false, vertical: true)
        .background(Theme.surfaceOverlay)
        .task {
            local = await git.listBranches(projectPath)
            loadingLocal = false
            focused = true
        }
    }
}

/// GitChangesIndicator.tsx: `+N -M` when the tree has uncommitted changes.
struct GitChangesIndicator: View {
    let stat: DiffStat
    let action: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                HStack(spacing: 6) {
                    Text("+\(stat.insertions)").foregroundStyle(Theme.diffAdd)
                    Text("-\(stat.deletions)").foregroundStyle(Theme.diffRemove)
                }
                .font(.system(size: 11, design: .monospaced))
                .fixedSize()
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(RoundedRectangle(cornerRadius: Theme.radius).fill(hover ? Theme.white(0.06) : .clear))
            }
            .buttonStyle(.plain)
            .help("\(stat.filesChanged) file\(stat.filesChanged != 1 ? "s" : "") changed")
        }
    }
}

/// OpenInButton.tsx: "Open" in the first detected editor, or pick one from the menu.
public struct OpenInButton: View {
    let projectPath: String
    let git: GitActions
    @State private var ides: [DetectedIDE] = OpenInButton.cache ?? []
    @State private var open = false
    @MainActor public static var cache: [DetectedIDE]?

    public init(projectPath: String, git: GitActions) {
        self.projectPath = projectPath
        self.git = git
    }

    public var body: some View {
        Group {
            if let first = ides.first {
                HStack(spacing: 0) {
                    Hovering { hover in
                        Button { git.openIn(first.id, projectPath) } label: {
                            HStack(spacing: 4) {
                                IDEIcon(first.id, size: 11)
                                Text("Open").font(.ui(10))
                            }
                            .foregroundStyle(hover ? Theme.gray300 : Theme.gray500)
                            .padding(.horizontal, 4).padding(.vertical, 2)
                            .background(UnevenRoundedRectangle(topLeadingRadius: Theme.radius, bottomLeadingRadius: Theme.radius)
                                .fill(hover ? Theme.white(0.06) : .clear))
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .help("Open in \(first.name)")
                    }
                    Hovering { hover in
                        Button { open.toggle() } label: {
                            LucideIcon(.chevronDown, size: 9).foregroundStyle(hover ? Theme.gray300 : Theme.gray500)
                                .padding(.horizontal, 2).frame(maxHeight: .infinity)
                                .background(UnevenRoundedRectangle(bottomTrailingRadius: Theme.radius, topTrailingRadius: Theme.radius)
                                    .fill(hover ? Theme.white(0.06) : .clear))
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Choose IDE")
                    }
                }
                .fixedSize()
                .popover(isPresented: $open, arrowEdge: .top) {
                    Menu(ides: ides, projectPath: projectPath, git: git) { open = false }
                }
            }
        }
        .task {
            if OpenInButton.cache == nil {
                let found = await git.detectIDEs()
                if !found.isEmpty { OpenInButton.cache = found }
                ides = found
            }
        }
    }

    /// The "Open in" list.
    public struct Menu: View {
        let ides: [DetectedIDE]
        let projectPath: String
        let git: GitActions
        let onPick: () -> Void

        public init(ides: [DetectedIDE], projectPath: String, git: GitActions, onPick: @escaping () -> Void) {
            self.ides = ides
            self.projectPath = projectPath
            self.git = git
            self.onPick = onPick
        }

        public var body: some View {
            VStack(alignment: .leading, spacing: 0) {
                Text("Open in").font(.ui(11, weight: .medium)).foregroundStyle(Theme.gray500).padding(.horizontal, 12).padding(.vertical, 6)
                ForEach(ides) { ide in
                    Hovering { hover in
                        Button {
                            git.openIn(ide.id, projectPath)
                            onPick()
                        } label: {
                            HStack(spacing: 10) {
                                IDEIcon(ide.id, size: 14)
                                Text(ide.name).font(.ui(13))
                                Spacer(minLength: 0)
                            }
                            .foregroundStyle(hover ? .white : Theme.gray300)
                            .padding(.horizontal, 12).padding(.vertical, 6)
                            .background(hover ? Theme.white(0.06) : .clear)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            .padding(.vertical, 4)
            .frame(width: 180)
            .background(Theme.surfaceOverlay)
        }
    }
}
