import SwiftUI
import VornCore
import VornTerminals
import VornUI

/// RightPanel.tsx: a session's uncommitted changes, with review comments and the commit dialog.
public struct ChangesPanel: View {
    @Bindable var panes: PanesModel
    let session: TerminalSession
    let stat: DiffStat?
    let project: Project?
    let services: PaneServices
    let onCommit: () -> Void
    @State private var result: GitDiffResult?
    @State private var loading = false
    @State private var selected: String?
    @State private var comments: [DiffComment] = []
    @State private var commenting: (file: String, line: Int, content: String)?
    @State private var dragStart: CGFloat?

    public init(panes: PanesModel, session: TerminalSession, stat: DiffStat?, project: Project?, services: PaneServices,
                onCommit: @escaping () -> Void) {
        self.panes = panes
        self.session = session
        self.stat = stat
        self.project = project
        self.services = services
        self.onCommit = onCommit
    }

    private var cwd: String { session.worktreePath ?? session.projectPath }

    public var body: some View {
        VStack(spacing: 0) {
            contextHeader
            tabBar
            changesHeader
            if !comments.isEmpty { feedbackBar }
            content
        }
        .frame(maxHeight: .infinity)
        .background(Theme.surfacePanel)
        .overlay(alignment: .leading) {
            Rectangle().fill(Theme.hairline).frame(width: 1)
            Color.clear.frame(width: 4).contentShape(Rectangle())
                .onHover { inside in if inside { NSCursor.resizeLeftRight.push() } else { NSCursor.pop() } }
                .gesture(DragGesture(minimumDistance: 1, coordinateSpace: .global)
                    .onChanged { g in
                        if dragStart == nil {
                            dragStart = panes.diffWidth
                            panes.diffMaximized = false
                        }
                        panes.diffWidth = max(320, min(800, (dragStart ?? 480) - g.translation.width))
                    }
                    .onEnded { _ in dragStart = nil })
        }
        .task(id: cwd) { await fetch() }
    }

    func fetch() async {
        loading = true
        defer { loading = false }
        result = try? await services.client.diffFull(cwd)
        selected = nil
        comments = []
        commenting = nil
    }

    private var contextHeader: some View {
        HStack(spacing: 8) {
            if let project {
                HStack(spacing: 4) {
                    ProjectIcon(icon: project.icon, color: project.iconColor, size: 11)
                    Text(project.name).lineLimit(1).frame(maxWidth: 100, alignment: .leading).fixedSize()
                }
            }
            if let wt = session.worktreeName {
                if project != nil { Text("/").foregroundStyle(Theme.gray700) }
                HStack(spacing: 4) {
                    LucideIcon(.folderGit2, size: 11, strokeWidth: 1.5).foregroundStyle(Theme.amber500)
                    Text(wt).lineLimit(1).frame(maxWidth: 100, alignment: .leading).fixedSize()
                }
            }
            if let b = session.branch {
                if project != nil || session.worktreeName != nil { Text("/").foregroundStyle(Theme.gray700) }
                HStack(spacing: 4) {
                    LucideIcon(.gitBranch, size: 11, strokeWidth: 1.5).foregroundStyle(Theme.gray600)
                    Text(b).lineLimit(1)
                }
            }
            Spacer(minLength: 0)
        }
        .font(.ui(11))
        .foregroundStyle(Theme.gray500)
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }
    }

    private var tabBar: some View {
        HStack(spacing: 2) {
            Text("Changes").font(.ui(11, weight: .medium)).foregroundStyle(Theme.gray200)
                .padding(.horizontal, 10).padding(.vertical, 4)
                .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.white(0.1)))
            Spacer()
            IconButton(panes.diffMaximized ? .minimize2 : .maximize2, size: 13, strokeWidth: 1.5, padding: 4, radius: Theme.radius,
                       color: Theme.gray500, help: panes.diffMaximized ? "Restore size" : "Maximize") { panes.diffMaximized.toggle() }
            IconButton(.x, size: 13, strokeWidth: 1.5, padding: 4, radius: Theme.radius, color: Theme.gray500, help: "Close panel") {
                panes.diffSessionID = nil
                panes.diffMaximized = false
            }
        }
        .padding(.horizontal, 8)
        .frame(height: 36)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }
    }

    private var changesHeader: some View {
        HStack(spacing: 12) {
            if let stat {
                HStack(spacing: 6) {
                    Text("+\(stat.insertions)").foregroundStyle(Theme.green400)
                    Text("-\(stat.deletions)").foregroundStyle(Theme.red400)
                    Text("\(stat.filesChanged) file\(stat.filesChanged != 1 ? "s" : "")").foregroundStyle(Theme.gray500)
                }
                .font(.system(size: 11, design: .monospaced))
            }
            Spacer()
            if let stat, stat.insertions > 0 || stat.deletions > 0 {
                Hovering { hover in
                    Button(action: onCommit) {
                        HStack(spacing: 6) {
                            LucideIcon(.gitCommitHorizontal, size: 13, strokeWidth: 1.5)
                            Text("Commit").font(.ui(11, weight: .medium))
                        }
                        .foregroundStyle(hover ? Color(hex: 0xF3F4F6) : Theme.gray300)
                        .padding(.horizontal, 8).padding(.vertical, 4)
                        .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.white(hover ? 0.08 : 0.04)))
                        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.hairline, lineWidth: 1))
                    }
                    .buttonStyle(.plain)
                    .help("Commit changes")
                }
            }
            Hovering { hover in
                Button { Task { await fetch() } } label: {
                    Group {
                        if loading { SpinningGlyph(glyph: .refreshCw, size: 13, strokeWidth: 1.5) } else { LucideIcon(.refreshCw, size: 13, strokeWidth: 1.5) }
                    }
                    .foregroundStyle(hover ? .white : Theme.gray400).padding(4)
                }
                .buttonStyle(.plain)
                .disabled(loading)
                .help("Refresh")
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }
    }

    private var feedbackBar: some View {
        HStack(spacing: 8) {
            LucideIcon(.messageSquare, size: 13).foregroundStyle(Theme.blue400)
            Text("\(comments.count) review comment\(comments.count != 1 ? "s" : "")").font(.ui(12)).foregroundStyle(Color(hex: 0x8EC5FF))
                .frame(maxWidth: .infinity, alignment: .leading)
            Button("Clear") { comments = [] }.buttonStyle(.plain).font(.ui(11)).foregroundStyle(Theme.gray500)
            Hovering { hover in
                Button {
                    services.write(session.id, DiffComment.feedback(comments) + "\n")
                    comments = []
                    commenting = nil
                } label: {
                    HStack(spacing: 6) {
                        LucideIcon(.send, size: 11)
                        Text("Send to Agent").font(.ui(11, weight: .medium))
                    }
                    .foregroundStyle(hover ? Color(hex: 0xBEDBFF) : Color(hex: 0x8EC5FF))
                    .padding(.horizontal, 10).padding(.vertical, 4)
                    .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.blue500.opacity(hover ? 0.3 : 0.2)))
                    .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.blue500.opacity(0.2), lineWidth: 1))
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(Theme.blue500.opacity(0.05))
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.blue500.opacity(0.15)).frame(height: 1) }
    }

    @ViewBuilder private var content: some View {
        if loading && result == nil {
            SpinningGlyph(size: 20).foregroundStyle(Theme.gray500).frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let r = result, !r.files.isEmpty {
            fileList(r.files)
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0, pinnedViews: [.sectionHeaders]) {
                        ForEach(r.files) { f in
                            Section {
                                DiffLines(file: f, comments: comments, commenting: commenting.flatMap { $0.file == f.filePath ? $0.line : nil },
                                          onClick: { row in
                                              if commenting?.file == f.filePath && commenting?.line == row.index { commenting = nil }
                                              else { commenting = (f.filePath, row.index, row.text) }
                                          },
                                          onAdd: { text in
                                              guard let c = commenting else { return }
                                              comments.append(DiffComment(filePath: c.file, lineIndex: c.line, lineContent: c.content, comment: text))
                                              commenting = nil
                                          },
                                          onCancel: { commenting = nil },
                                          onRemove: { i in comments.remove(at: i) })
                            } header: {
                                fileHeader(f).id(f.filePath)
                            }
                        }
                    }
                }
                .onChange(of: selected) { _, s in if let s { withAnimation { proxy.scrollTo(s, anchor: .top) } } }
            }
        } else {
            Text("No changes").font(.ui(14)).foregroundStyle(Theme.gray500).frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func fileList(_ files: [GitFileDiff]) -> some View {
        ScrollView {
            VStack(spacing: 0) {
                ForEach(files) { f in
                    Hovering { hover in
                        Button { selected = f.filePath } label: {
                            HStack(spacing: 8) {
                                FileTypeIcon(name: (f.filePath as NSString).lastPathComponent, size: 15)
                                Text(f.filePath).font(.system(size: 12, design: .monospaced)).foregroundStyle(Theme.gray300).lineLimit(1)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                HStack(spacing: 6) {
                                    if f.insertions > 0 { Text("+\(f.insertions)").foregroundStyle(Theme.diffAdd) }
                                    if f.deletions > 0 { Text("-\(f.deletions)").foregroundStyle(Theme.diffRemove) }
                                }
                                .font(.system(size: 11, design: .monospaced))
                                Text(f.letter).font(.ui(10, weight: .bold)).foregroundStyle(Theme.inkSecondary)
                            }
                            .padding(.horizontal, 12).padding(.vertical, 6)
                            .background(selected == f.filePath ? Theme.white(0.08) : hover ? Theme.white(0.04) : .clear)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
        .frame(maxHeight: min(200, CGFloat(files.count) * 30))
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }
    }

    private func fileHeader(_ f: GitFileDiff) -> some View {
        let n = comments.filter { $0.filePath == f.filePath }.count
        return HStack(spacing: 8) {
            FileTypeIcon(name: (f.filePath as NSString).lastPathComponent, size: 14)
            Text(f.filePath).foregroundStyle(Theme.gray300).lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
            Text(f.letter).font(.ui(10, weight: .bold)).foregroundStyle(Theme.inkSecondary)
            if n > 0 {
                Text("\(n) comment\(n != 1 ? "s" : "")").font(.ui(10)).foregroundStyle(Theme.inkSecondary)
                    .padding(.horizontal, 6).padding(.vertical, 2).background(Capsule().fill(Theme.white(0.06)))
            }
        }
        .font(.system(size: 12, design: .monospaced))
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(Theme.surfaceOverlay)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }
    }
}

/// DiffContent's rows for one file: numbered, coloured, with comments under the lines they are on.
struct DiffLines: View {
    let file: GitFileDiff
    let comments: [DiffComment]
    let commenting: Int?
    let onClick: (DiffRow) -> Void
    let onAdd: (String) -> Void
    let onCancel: () -> Void
    let onRemove: (Int) -> Void

    static let rowHeight: CGFloat = 19.2

    var body: some View {
        let rows = DiffRow.parse(file.diff)
        VStack(alignment: .leading, spacing: 0) {
            ForEach(rows, id: \.index) { row in
                line(row)
                ForEach(Array(comments.enumerated()).filter { $0.element.filePath == file.filePath && $0.element.lineIndex == row.index }, id: \.offset) { i, c in
                    HStack(alignment: .top, spacing: 8) {
                        LucideIcon(.messageSquare, size: 11).foregroundStyle(Theme.inkSecondary).padding(.top, 2)
                        Text(c.comment).font(.ui(12)).foregroundStyle(Theme.inkSecondary).frame(maxWidth: .infinity, alignment: .leading)
                        IconButton(.x, size: 10, padding: 2, color: Theme.gray600, hoverColor: Theme.danger) { onRemove(i) }
                    }
                    .padding(.horizontal, 12).padding(.vertical, 6)
                    .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.white(0.03)))
                    .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.08), lineWidth: 1))
                    .padding(.horizontal, 8).padding(.vertical, 2)
                }
                if commenting == row.index { CommentInput(onSubmit: onAdd, onCancel: onCancel) }
            }
        }
        .font(.system(size: 12, design: .monospaced))
    }

    @ViewBuilder private func line(_ row: DiffRow) -> some View {
        switch row.kind {
        case .hunk:
            Text(row.text).foregroundStyle(Theme.inkSecondary).lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 12).padding(.vertical, 2)
                .frame(height: Self.rowHeight + 4)
                .background(Theme.white(0.05))
                .textSelection(.enabled)
        case .meta:
            Text(row.text).foregroundStyle(Theme.gray500).padding(.horizontal, 12).frame(height: Self.rowHeight)
        case .add, .del, .ctx:
            Hovering { hover in
                HStack(spacing: 0) {
                    number(row.kind == .add ? nil : row.oldLine, color: row.kind == .del ? Color(hex: 0xE7000B) : Theme.gray600)
                    number(row.kind == .del ? nil : row.newLine, color: row.kind == .add ? Color(hex: 0x00A63E) : Theme.gray600)
                    Text(row.text.count > 1 ? String(row.text.dropFirst()) : " ")
                        .foregroundStyle(row.kind == .add ? Color(hex: 0x7BF1A8) : row.kind == .del ? Theme.red300 : Theme.gray400)
                        .lineLimit(1).fixedSize()
                        .padding(.horizontal, 4)
                    Spacer(minLength: 0)
                    if row.kind != .ctx {
                        LucideIcon(.messageSquare, size: 11).foregroundStyle(Theme.inkSecondary).padding(.trailing, 8).opacity(hover ? 1 : 0)
                    }
                }
                .frame(height: Self.rowHeight)
                .background(bg(row.kind, hover))
                .contentShape(Rectangle())
                .onTapGesture { if row.kind != .ctx { onClick(row) } }
            }
        }
    }

    private func bg(_ k: DiffRow.Kind, _ hover: Bool) -> Color {
        switch k {
        case .add: Theme.green500.opacity(hover ? 0.15 : 0.1)
        case .del: Theme.red500.opacity(hover ? 0.15 : 0.1)
        default: .clear
        }
    }

    private func number(_ n: Int?, color: Color) -> some View {
        Text(n.map(String.init) ?? " ").font(.system(size: 11, design: .monospaced)).foregroundStyle(color)
            .frame(width: 27, alignment: .trailing).padding(.trailing, 8)
    }
}

/// InlineCommentInput: ⌘↩ to submit, Esc to cancel.
struct CommentInput: View {
    let onSubmit: (String) -> Void
    let onCancel: () -> Void
    @State private var text = ""
    @FocusState private var focused: Bool

    var body: some View {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        VStack(alignment: .leading, spacing: 6) {
            TextField("", text: $text, prompt: Text("Add review comment...").foregroundColor(Theme.gray600), axis: .vertical)
                .lineLimit(2, reservesSpace: true)
                .textFieldStyle(.plain).font(.system(size: 12, design: .monospaced)).foregroundStyle(Theme.gray200)
                .focused($focused)
                .padding(.horizontal, 8).padding(.vertical, 6)
                .background(RoundedRectangle(cornerRadius: Theme.radius).fill(Theme.white(0.04)))
                .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(focused ? Theme.white(0.2) : Theme.white(0.08), lineWidth: 1))
                .onExitCommand(perform: onCancel)
            HStack {
                Text("Cmd+Enter to submit").font(.ui(10)).foregroundStyle(Theme.gray600)
                Spacer()
                Button("Cancel", action: onCancel).buttonStyle(.plain).font(.ui(10)).foregroundStyle(Theme.gray500)
                Button("Comment") { if !trimmed.isEmpty { onSubmit(trimmed) } }
                    .buttonStyle(.plain).font(.ui(10, weight: .medium)).foregroundStyle(Theme.ink)
                    .disabled(trimmed.isEmpty).opacity(trimmed.isEmpty ? 0.3 : 1)
                    .keyboardShortcut(.return, modifiers: .command)
            }
        }
        .padding(8)
        .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.white(0.04)))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.10), lineWidth: 1))
        .padding(.horizontal, 8).padding(.vertical, 4)
        .onAppear { focused = true }
    }
}

/// CommitDialog.tsx.
public struct CommitDialog: View {
    let cwd: String
    let branch: String?
    let stat: DiffStat?
    let client: VornClient
    let toasts: ToastCenter
    let onClose: () -> Void
    let onCommitted: () -> Void
    @State private var message = ""
    @State private var includeUnstaged = true
    @State private var push = false
    @State private var loading = false
    @State private var error: String?

    public init(cwd: String, branch: String?, stat: DiffStat?, client: VornClient, toasts: ToastCenter,
                onClose: @escaping () -> Void, onCommitted: @escaping () -> Void) {
        self.cwd = cwd
        self.branch = branch
        self.stat = stat
        self.client = client
        self.toasts = toasts
        self.onClose = onClose
        self.onCommitted = onCommitted
    }

    public var body: some View {
        let can = !message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !loading
        ZStack {
            Color.black.opacity(0.5).contentShape(Rectangle()).onTapGesture(perform: onClose)
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 12) {
                    LucideIcon(.gitCommitHorizontal, size: 18, strokeWidth: 1.5).foregroundStyle(Theme.gray300)
                        .padding(8).background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.white(0.06)))
                    Text("Commit your changes").font(.ui(18, weight: .semibold)).lineHeight(28, size: 18).foregroundStyle(Color(hex: 0xF3F4F6))
                        .frame(maxWidth: .infinity, alignment: .leading)
                    IconButton(.x, size: 18, strokeWidth: 1.5, padding: 4, radius: Theme.radius, color: Theme.gray400, action: onClose)
                }
                .padding(.horizontal, 20).padding(.top, 20).padding(.bottom, 12)
                VStack(alignment: .leading, spacing: 16) {
                    HStack {
                        Text("Branch").font(.ui(14, weight: .medium)).foregroundStyle(Theme.gray300).lineHeight(20, size: 14)
                        Spacer()
                        HStack(spacing: 6) {
                            LucideIcon(.gitBranch, size: 14, strokeWidth: 1.5).foregroundStyle(Theme.gray500)
                            Text(branch ?? "unknown").font(.system(size: 14, design: .monospaced)).foregroundStyle(Theme.gray200)
                        }
                    }
                    HStack {
                        Text("Changes").font(.ui(14, weight: .medium)).foregroundStyle(Theme.gray300).lineHeight(20, size: 14)
                        Spacer()
                        if let stat {
                            HStack(spacing: 8) {
                                Text("\(stat.filesChanged) file\(stat.filesChanged != 1 ? "s" : "")").foregroundStyle(Theme.gray300)
                                Text("+\(stat.insertions)").font(.system(size: 14, design: .monospaced)).foregroundStyle(Theme.green400)
                                Text("-\(stat.deletions)").font(.system(size: 14, design: .monospaced)).foregroundStyle(Theme.red400)
                            }
                            .font(.ui(14))
                        } else {
                            Text("No changes").font(.ui(14)).foregroundStyle(Theme.gray500)
                        }
                    }
                    HStack(spacing: 12) {
                        Button { includeUnstaged.toggle() } label: {
                            ZStack(alignment: .topLeading) {
                                Capsule().fill(includeUnstaged ? Theme.blue500 : Theme.white(0.12))
                                Circle().fill(.white).frame(width: 16, height: 16).offset(x: includeUnstaged ? 22 : 3, y: 3)
                            }
                            .frame(width: 40, height: 22)
                            .animation(.easeOut(duration: 0.15), value: includeUnstaged)
                        }
                        .buttonStyle(.plain)
                        Text("Include unstaged").font(.ui(14)).foregroundStyle(Theme.gray300).lineHeight(20, size: 14)
                    }
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Commit message").font(.ui(14, weight: .medium)).foregroundStyle(Theme.gray300).lineHeight(20, size: 14)
                        TextField("", text: $message, prompt: Text("Enter a commit message...").foregroundColor(Theme.gray600), axis: .vertical)
                            .lineLimit(3, reservesSpace: true)
                            .textFieldStyle(.plain).font(.ui(14)).foregroundStyle(Theme.gray200).lineSpacing(3)
                            .padding(.horizontal, 12).padding(.vertical, 8)
                            .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.white(0.04)))
                            .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.08), lineWidth: 1))
                    }
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Next steps").font(.ui(14, weight: .medium)).foregroundStyle(Theme.gray300).lineHeight(20, size: 14)
                        VStack(spacing: 4) {
                            step(.gitCommitHorizontal, "Commit", selected: !push) { push = false }
                            step(.arrowUp, "Commit and push", selected: push) { push = true }
                        }
                    }
                    if let error {
                        Text(error).font(.ui(13)).foregroundStyle(Theme.red400)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 12).padding(.vertical, 8)
                            .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.red500.opacity(0.1)))
                    }
                    Button { Task { await go() } } label: {
                        Group {
                            if loading { SpinningGlyph(size: 16) } else { Text("Continue").font(.ui(14, weight: .medium)).lineHeight(20, size: 14) }
                        }
                        .foregroundStyle(can ? Color(hex: 0x101828) : Theme.gray500)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 10)
                        .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(can ? .white : Theme.white(0.06)))
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .disabled(!can)
                }
                .padding(.horizontal, 20).padding(.bottom, 20)
            }
            .frame(width: 440)
            .background(RoundedRectangle(cornerRadius: Theme.radiusXl).fill(Theme.surfaceOverlay))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusXl).strokeBorder(Theme.white(0.08), lineWidth: 1))
            .floatingShadow()
        }
    }

    private func step(_ icon: LucideGlyph, _ label: String, selected: Bool, action: @escaping () -> Void) -> some View {
        Hovering { hover in
            Button(action: action) {
                HStack(spacing: 12) {
                    LucideIcon(icon, size: 16, strokeWidth: 1.5)
                    Text(label).font(.ui(14)).lineHeight(20, size: 14).frame(maxWidth: .infinity, alignment: .leading)
                    if selected { LucideIcon(.check, size: 16).foregroundStyle(Theme.gray300) }
                }
                .foregroundStyle(selected ? Color(hex: 0xF3F4F6) : Theme.gray400)
                .padding(.horizontal, 12).padding(.vertical, 10)
                .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(selected ? Theme.white(0.08) : hover ? Theme.white(0.04) : .clear))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }

    private func go() async {
        let msg = message.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !msg.isEmpty, !loading else { return }
        loading = true
        error = nil
        defer { loading = false }
        do {
            if let err = try await client.commit(cwd: cwd, message: msg, includeUnstaged: includeUnstaged) {
                error = err
                return
            }
            if push, let err = try await client.push(cwd: cwd) {
                error = "Committed, but push failed: \(err)"
                return
            }
            toasts.show(push ? "Committed and pushed" : "Changes committed", .success)
            onCommitted()
            onClose()
        } catch {
            self.error = String(describing: error)
        }
    }
}
