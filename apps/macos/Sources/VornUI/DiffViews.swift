import SwiftUI
import VornCore

private extension Color {
    static let green300 = Color(hex: 0x7BF1A8)
    static let green600 = Color(hex: 0x00A63E)
    static let red600 = Color(hex: 0xE7000B)
}

/// DiffSidebar.tsx's review state: the file scrolled to, the comments, and the line whose comment box is open.
@MainActor @Observable
public final class DiffReviewState {
    public var selectedFile: String?
    public private(set) var comments: [DiffComment] = []
    /// The line whose comment box is open; its `comment` is unused.
    public var commentingLine: DiffComment?

    public init() {}

    public func reset() {
        selectedFile = nil
        comments = []
        commentingLine = nil
    }

    public func clearComments() { comments = [] }

    /// handleClickLine: a second click on the open line closes it.
    public func clickLine(filePath: String, lineIndex: Int, lineContent: String) {
        if commentingLine?.filePath == filePath, commentingLine?.lineIndex == lineIndex {
            commentingLine = nil
        } else {
            commentingLine = DiffComment(filePath: filePath, lineIndex: lineIndex, lineContent: lineContent, comment: "")
        }
    }

    public func addComment(_ text: String) {
        guard var line = commentingLine else { return }
        line.comment = text
        comments.append(line)
        commentingLine = nil
    }

    public func comment(filePath: String, lineIndex: Int, lineContent: String, _ text: String) {
        clickLine(filePath: filePath, lineIndex: lineIndex, lineContent: lineContent)
        addComment(text)
    }

    public func removeComment(at index: Int) {
        guard comments.indices.contains(index) else { return }
        comments.remove(at: index)
    }
}

/// DiffSidebar.tsx DiffFileList: one row per changed file, at most 200pt tall before it scrolls.
public struct DiffFileList: View {
    let files: [GitFileDiff]
    let selectedFile: String?
    let onSelect: (String) -> Void

    public init(files: [GitFileDiff], selectedFile: String?, onSelect: @escaping (String) -> Void) {
        self.files = files
        self.selectedFile = selectedFile
        self.onSelect = onSelect
    }

    private static let rowHeight: CGFloat = 28

    public var body: some View {
        ScrollView(.vertical) {
            VStack(spacing: 0) {
                ForEach(files) { file in row(file) }
            }
        }
        .frame(height: min(200, CGFloat(files.count) * Self.rowHeight))
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.white(0.06)).frame(height: 1) }
    }

    private func row(_ file: GitFileDiff) -> some View {
        Hovering { hovered in
            Button { onSelect(file.filePath) } label: {
                HStack(spacing: 8) {
                    FileTypeIcon(name: (file.filePath as NSString).lastPathComponent, size: 15)
                    Text(file.filePath).font(.system(size: 12, design: .monospaced)).foregroundStyle(Theme.gray300)
                        .lineLimit(1).truncationMode(.tail)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    HStack(spacing: 6) {
                        if file.insertions > 0 { Text("+\(file.insertions)").foregroundStyle(Theme.diffAdd) }
                        if file.deletions > 0 { Text("-\(file.deletions)").foregroundStyle(Theme.diffRemove) }
                    }
                    .font(.system(size: 11, design: .monospaced))
                    Text(file.status.letter).font(.system(size: 10, weight: .bold)).foregroundStyle(Theme.inkSecondary)
                }
                .padding(.horizontal, 12)
                .frame(height: Self.rowHeight)
                .background(selectedFile == file.filePath ? Theme.white(0.08) : hovered ? Theme.white(0.04) : .clear)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }
}

/// DiffSidebar.tsx DiffContent: every file's diff under a pinned header, a click on a changed line opening a comment.
/// The caller puts it in a vertical scroll view.
public struct DiffContent: View {
    let model: DiffReviewState
    let files: [GitFileDiff]

    public init(review: DiffReviewState, files: [GitFileDiff]) {
        self.model = review
        self.files = files
    }

    public var body: some View {
        ScrollViewReader { proxy in
            LazyVStack(alignment: .leading, spacing: 0, pinnedViews: .sectionHeaders) {
                ForEach(files) { file in
                    Section {
                        DiffFileRows(model: model, file: file)
                    } header: {
                        header(file)
                    }
                    .id(file.filePath)
                }
            }
            .onChange(of: model.selectedFile) { _, file in
                guard let file else { return }
                withAnimation(.easeOut(duration: 0.2)) { proxy.scrollTo(file, anchor: .top) }
            }
        }
    }

    private func header(_ file: GitFileDiff) -> some View {
        let count = model.comments.filter { $0.filePath == file.filePath }.count
        return HStack(spacing: 8) {
            FileTypeIcon(name: (file.filePath as NSString).lastPathComponent, size: 14)
            Text(file.filePath).foregroundStyle(Theme.gray300).lineLimit(1).truncationMode(.tail)
                .frame(maxWidth: .infinity, alignment: .leading)
            Text(file.status.letter).font(.system(size: 10, weight: .bold)).foregroundStyle(Theme.inkSecondary)
            if count > 0 {
                Text("\(count) comment\(count == 1 ? "" : "s")")
                    .font(.system(size: 10)).foregroundStyle(Theme.inkSecondary)
                    .padding(.horizontal, 6).padding(.vertical, 2)
                    .background(Theme.white(0.06), in: Capsule())
            }
        }
        .font(.system(size: 12, design: .monospaced))
        .padding(.horizontal, 12).padding(.vertical, 6)
        .background(Theme.surfaceOverlay)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.white(0.06)).frame(height: 1) }
    }
}

/// One file's rows, with the comments on them and the open comment box.
private struct DiffFileRows: View {
    let model: DiffReviewState
    let file: GitFileDiff

    var body: some View {
        let rows = DiffRow.parse(file.diff)
        let commenting = model.commentingLine?.filePath == file.filePath ? model.commentingLine?.lineIndex : nil
        ForEach(rows) { row in
            DiffRowView(row: row) { model.clickLine(filePath: file.filePath, lineIndex: row.index, lineContent: row.text) }
            ForEach(Array(model.comments.enumerated()).filter { $0.element.filePath == file.filePath && $0.element.lineIndex == row.index },
                    id: \.offset) { index, comment in
                CommentBadge(comment: comment) { model.removeComment(at: index) }
            }
            if commenting == row.index {
                InlineCommentInput(onSubmit: model.addComment) { model.commentingLine = nil }
            }
        }
    }
}

/// DiffRowView: a hunk header, metadata, or a numbered line; added and removed lines take a comment.
private struct DiffRowView: View {
    let row: DiffRow
    let onClick: () -> Void
    @State private var hovered = false

    var body: some View {
        switch row.kind {
        case .hunk:
            line(row.text, Theme.inkSecondary).padding(.horizontal, 12).padding(.vertical, 2)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Theme.white(0.05))
        case .meta:
            line(row.text, Theme.gray500).padding(.horizontal, 12)
                .frame(maxWidth: .infinity, alignment: .leading)
        case .add:
            numbered(old: nil, new: row.newLine, oldColor: Theme.gray600, newColor: .green600, text: .green300,
                     fill: Theme.green500.opacity(hovered ? 0.15 : 0.1))
        case .del:
            numbered(old: row.oldLine, new: nil, oldColor: .red600, newColor: Theme.gray600, text: Theme.red300,
                     fill: Theme.red500.opacity(hovered ? 0.15 : 0.1))
        case .ctx:
            numbered(old: row.oldLine, new: row.newLine, oldColor: Theme.gray600, newColor: Theme.gray600,
                     text: Theme.gray400, fill: .clear)
        }
    }

    private func line(_ text: String, _ color: Color) -> some View {
        Text(text).font(.system(size: 12, design: .monospaced)).foregroundStyle(color)
            .lineLimit(1).frame(minHeight: 19.2)
    }

    private func number(_ n: Int?, _ color: Color) -> some View {
        Text(n.map(String.init) ?? " ").font(.system(size: 11, design: .monospaced)).foregroundStyle(color)
            .frame(width: 27, alignment: .trailing).padding(.trailing, 8)
    }

    @ViewBuilder
    private func numbered(old: Int?, new: Int?, oldColor: Color, newColor: Color, text: Color, fill: Color) -> some View {
        let changed = row.kind != .ctx
        let content = HStack(spacing: 0) {
            number(old, oldColor)
            number(new, newColor)
            line(String(row.text.dropFirst()).isEmpty ? " " : String(row.text.dropFirst()), text)
                .padding(.horizontal, 4)
                .frame(maxWidth: .infinity, alignment: .leading)
            if changed {
                LucideIcon(.messageSquare, size: 11).foregroundStyle(Theme.inkSecondary)
                    .padding(.trailing, 8).opacity(hovered ? 1 : 0)
            }
        }
        .background(fill)
        if changed {
            content
                .contentShape(Rectangle())
                .onHover { hovered = $0 }
                .onTapGesture(perform: onClick)
        } else {
            content
        }
    }
}

/// InlineCommentInput: a two-line box under the line; ⌘↩ comments, Esc cancels.
private struct InlineCommentInput: View {
    let onSubmit: (String) -> Void
    let onCancel: () -> Void
    @State private var text = ""
    @FocusState private var focused: Bool

    private var trimmed: String { text.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            TextEditor(text: $text)
                .font(.system(size: 12, design: .monospaced))
                .foregroundStyle(Theme.gray200)
                .scrollContentBackground(.hidden)
                .focused($focused)
                .frame(height: 34)
                .padding(.horizontal, 3).padding(.vertical, 6)
                .overlay(alignment: .topLeading) {
                    if text.isEmpty {
                        Text("Add review comment...").font(.system(size: 12, design: .monospaced))
                            .foregroundStyle(Theme.gray600).padding(.horizontal, 8).padding(.vertical, 6)
                            .allowsHitTesting(false)
                    }
                }
                .background(Theme.white(0.04), in: RoundedRectangle(cornerRadius: Theme.radius))
                .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(focused ? 0.2 : 0.08)))
            HStack {
                Text("Cmd+Enter to submit").font(.system(size: 10)).foregroundStyle(Theme.gray600)
                Spacer()
                Hovering { hovered in
                    Button(action: onCancel) {
                        Text("Cancel").font(.system(size: 10)).foregroundStyle(hovered ? Theme.gray300 : Theme.gray500)
                            .padding(.horizontal, 8).padding(.vertical, 4)
                    }
                    .buttonStyle(.plain)
                    .keyboardShortcut(.cancelAction)
                }
                Hovering { hovered in
                    Button { if !trimmed.isEmpty { onSubmit(trimmed) } } label: {
                        Text("Comment").font(.system(size: 10, weight: .medium)).foregroundStyle(hovered ? .white : Theme.ink)
                            .padding(.horizontal, 8).padding(.vertical, 4)
                    }
                    .buttonStyle(.plain)
                    .keyboardShortcut(.return, modifiers: .command)
                    .disabled(trimmed.isEmpty)
                    .opacity(trimmed.isEmpty ? 0.3 : 1)
                }
            }
        }
        .padding(8)
        .background(Theme.white(0.04), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.1)))
        .padding(.horizontal, 8).padding(.vertical, 4)
        .onAppear { focused = true }
    }
}

/// CommentBadge: a comment under its line, with its own remove button.
private struct CommentBadge: View {
    let comment: DiffComment
    let onRemove: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            LucideIcon(.messageSquare, size: 11).foregroundStyle(Theme.inkSecondary).padding(.top, 2)
            Text(comment.comment).font(.system(size: 12)).foregroundStyle(Theme.inkSecondary)
                .frame(maxWidth: .infinity, alignment: .leading)
            Hovering { hovered in
                Button(action: onRemove) {
                    LucideIcon(.x, size: 10).foregroundStyle(hovered ? Theme.danger : Theme.gray600).padding(2)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 12).padding(.vertical, 6)
        .background(Theme.white(0.03), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.08)))
        .padding(.horizontal, 8).padding(.vertical, 2)
    }
}

/// CommitDialog.tsx: the branch, the totals, a message, and whether to push after committing.
public struct CommitDialog: View {
    public typealias Commit = (_ message: String, _ includeUnstaged: Bool, _ push: Bool) async -> String?

    let branch: String?
    let stat: DiffStat?
    let onClose: () -> Void
    let commit: Commit

    @State private var message = ""
    @State private var includeUnstaged = true
    @State private var push = false
    @State private var loading = false
    @State private var error: String?
    @FocusState private var focused: Bool

    private static let gray100 = Color(hex: 0xF3F4F6)

    /// `commit` answers the error to show, or nil once it committed (and pushed), which closes the dialog.
    public init(branch: String?, stat: DiffStat?, onClose: @escaping () -> Void, commit: @escaping Commit) {
        self.branch = branch
        self.stat = stat
        self.onClose = onClose
        self.commit = commit
    }

    private var canCommit: Bool { !message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !loading }

    public var body: some View {
        ZStack {
            Color.black.opacity(0.5).contentShape(Rectangle()).onTapGesture(perform: onClose)
            dialog
        }
    }

    private var dialog: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 12) {
                LucideIcon(.gitCommitHorizontal, size: 18, strokeWidth: 1.5).foregroundStyle(Theme.gray300)
                    .padding(8).background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.white(0.06)))
                Text("Commit your changes").font(.ui(18, weight: .semibold)).lineHeight(28, size: 18).foregroundStyle(Self.gray100)
                    .frame(maxWidth: .infinity, alignment: .leading)
                IconButton(.x, size: 18, strokeWidth: 1.5, padding: 4, radius: Theme.radius, color: Theme.gray400, action: onClose)
            }
            .padding(.horizontal, 20).padding(.top, 20).padding(.bottom, 12)

            VStack(alignment: .leading, spacing: 16) {
                row("Branch") {
                    HStack(spacing: 6) {
                        LucideIcon(.gitBranch, size: 14, strokeWidth: 1.5).foregroundStyle(Theme.gray500)
                        Text(branch ?? "unknown").font(.system(size: 14, design: .monospaced)).foregroundStyle(Theme.gray200)
                    }
                }
                row("Changes") {
                    if let stat {
                        HStack(spacing: 8) {
                            Text("\(stat.filesChanged) file\(stat.filesChanged == 1 ? "" : "s")").foregroundStyle(Theme.gray300)
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
                    Text("Include unstaged").font(.ui(14)).lineHeight(20, size: 14).foregroundStyle(Theme.gray300)
                }
                VStack(alignment: .leading, spacing: 8) {
                    label("Commit message")
                    TextField("", text: $message, prompt: Text("Enter a commit message...").foregroundColor(Theme.gray600), axis: .vertical)
                        .lineLimit(3, reservesSpace: true)
                        .textFieldStyle(.plain).font(.ui(14)).foregroundStyle(Theme.gray200).lineSpacing(3)
                        .focused($focused)
                        .padding(.horizontal, 12).padding(.vertical, 8)
                        .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.white(0.04)))
                        .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(focused ? 0.16 : 0.08), lineWidth: 1))
                }
                VStack(alignment: .leading, spacing: 8) {
                    label("Next steps")
                    VStack(spacing: 4) {
                        option(.gitCommitHorizontal, "Commit", selected: !push) { push = false }
                        option(.arrowUp, "Commit and push", selected: push) { push = true }
                    }
                }
                if let error {
                    Text(error).font(.ui(13)).foregroundStyle(Theme.red400)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 12).padding(.vertical, 8)
                        .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.red500.opacity(0.1)))
                }
                Button(action: submit) {
                    Group {
                        if loading {
                            SpinningGlyph(glyph: .loaderCircle, size: 16, strokeWidth: 2, active: true)
                        } else {
                            Text("Continue").font(.ui(14, weight: .medium)).lineHeight(20, size: 14)
                        }
                    }
                    .foregroundStyle(canCommit ? Color(hex: 0x101828) : Theme.gray500)
                    .frame(maxWidth: .infinity).padding(.vertical, 10)
                    .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(canCommit ? .white : Theme.white(0.06)))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(!canCommit)
                .keyboardShortcut(.return, modifiers: .command)
            }
            .padding(.horizontal, 20).padding(.bottom, 20)
        }
        .frame(width: 440)
        .background(RoundedRectangle(cornerRadius: Theme.radiusXl).fill(Theme.surfaceOverlay))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusXl).strokeBorder(Theme.white(0.08), lineWidth: 1))
        .floatingShadow()
        .onExitCommand(perform: onClose)
    }

    private func row(_ title: String, @ViewBuilder _ value: () -> some View) -> some View {
        HStack {
            label(title)
            Spacer()
            value()
        }
    }

    private func label(_ text: String) -> some View {
        Text(text).font(.ui(14, weight: .medium)).lineHeight(20, size: 14).foregroundStyle(Theme.gray300)
    }

    private func option(_ glyph: LucideGlyph, _ title: String, selected: Bool, action: @escaping () -> Void) -> some View {
        Hovering { hovered in
            Button(action: action) {
                HStack(spacing: 12) {
                    LucideIcon(glyph, size: 16, strokeWidth: 1.5)
                    Text(title).font(.ui(14)).lineHeight(20, size: 14).frame(maxWidth: .infinity, alignment: .leading)
                    if selected { LucideIcon(.check, size: 16, strokeWidth: 2).foregroundStyle(Theme.gray300) }
                }
                .foregroundStyle(selected ? Self.gray100 : Theme.gray400)
                .padding(.horizontal, 12).padding(.vertical, 10)
                .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(selected ? Theme.white(0.08) : hovered ? Theme.white(0.04) : .clear))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }

    /// handleContinue: a failure stays on screen; success closes the dialog.
    private func submit() {
        guard canCommit else { return }
        loading = true
        error = nil
        let text = message.trimmingCharacters(in: .whitespacesAndNewlines)
        Task {
            let failure = await commit(text, includeUnstaged, push)
            loading = false
            if let failure { error = failure } else { onClose() }
        }
    }
}
