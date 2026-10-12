import SwiftUI
import VornCore
import VornUI

private extension Color {
    static let green300 = Color(hex: 0x7BF1A8)
    static let green600 = Color(hex: 0x00A63E)
    static let red600 = Color(hex: 0xE7000B)
}

/// DiffSidebar.tsx DiffFileList: one row per changed file, at most 200pt tall before it scrolls.
struct DiffFileList: View {
    let files: [GitFileDiff]
    let selectedFile: String?
    let onSelect: (String) -> Void

    private static let rowHeight: CGFloat = 28

    var body: some View {
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
                    LucideIcon(.fileCode, size: 15, strokeWidth: 1.5).foregroundStyle(Theme.inkSecondary)
                    Text(file.filePath).font(.system(size: 12, design: .monospaced)).foregroundStyle(Theme.gray300)
                        .lineLimit(1).truncationMode(.tail)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    HStack(spacing: 6) {
                        if file.insertions > 0 { Text("+\(file.insertions)").foregroundStyle(Theme.diffAdd) }
                        if file.deletions > 0 { Text("-\(file.deletions)").foregroundStyle(Theme.diffRemove) }
                    }
                    .font(.system(size: 11, design: .monospaced))
                    Text(DiffReview.letter(file.status)).font(.system(size: 10, weight: .bold)).foregroundStyle(Theme.inkSecondary)
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
struct DiffContent: View {
    let model: TaskDetailModel
    let files: [GitFileDiff]

    var body: some View {
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
            LucideIcon(.fileCode, size: 14, strokeWidth: 1.5).foregroundStyle(Theme.inkSecondary)
            Text(file.filePath).foregroundStyle(Theme.gray300).lineLimit(1).truncationMode(.tail)
                .frame(maxWidth: .infinity, alignment: .leading)
            Text(DiffReview.letter(file.status)).font(.system(size: 10, weight: .bold)).foregroundStyle(Theme.inkSecondary)
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
    let model: TaskDetailModel
    let file: GitFileDiff

    var body: some View {
        let rows = DiffReview.rows(file.diff)
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
