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
    @State private var review = DiffReviewState()
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
            if !review.comments.isEmpty { feedbackBar }
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
        review.reset()
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
            Text("\(review.comments.count) review comment\(review.comments.count != 1 ? "s" : "")").font(.ui(12)).foregroundStyle(Color(hex: 0x8EC5FF))
                .frame(maxWidth: .infinity, alignment: .leading)
            Button("Clear") { review.clearComments() }.buttonStyle(.plain).font(.ui(11)).foregroundStyle(Theme.gray500)
            Hovering { hover in
                Button {
                    services.write(session.id, DiffComment.feedback(review.comments) + "\n")
                    review.clearComments()
                    review.commentingLine = nil
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
            DiffFileList(files: r.files, selectedFile: review.selectedFile) { review.selectedFile = $0 }
            ScrollView { DiffContent(review: review, files: r.files) }
        } else {
            Text("No changes").font(.ui(14)).foregroundStyle(Theme.gray500).frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}
