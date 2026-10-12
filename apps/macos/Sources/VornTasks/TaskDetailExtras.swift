import AppKit
import SwiftUI
import UniformTypeIdentifiers
import VornCore
import VornUI
import VornWorkflows

/// TaskDetailPanel.tsx images: 64pt tiles that drop their × on hover, and the dashed "Add images" tile.
struct TaskImagesStrip: View {
    let model: TaskDetailModel

    var body: some View {
        WrapLayout(spacing: 8) {
            ForEach(model.images, id: \.self) { name in
                ImageTile(image: model.imagePaths[name].flatMap(Thumbnail.file)) {
                    Task { await model.removeImage(name) }
                }
            }
            ForEach(model.pending) { image in
                ImageTile(image: NSImage(data: image.data)) { model.removePending(image) }
            }
            Hovering { hovered in
                Button(action: pick) {
                    LucideIcon(.imagePlus, size: 18, strokeWidth: 1.5)
                        .foregroundStyle(hovered ? Theme.inkSecondary : Theme.inkFaint)
                        .frame(width: 64, height: 64)
                        .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg)
                            .strokeBorder(Theme.white(hovered ? 0.2 : 0.1), style: StrokeStyle(lineWidth: 1, dash: [3, 3])))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("Add images")
            }
        }
        .padding(.horizontal, 16).padding(.bottom, 12)
    }

    /// openImageDialog: any number of images.
    private func pick() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.allowedContentTypes = TaskDetailModel.imageExtensions.compactMap { UTType(filenameExtension: $0) }
        panel.begin { response in
            guard response == .OK else { return }
            let urls = panel.urls
            Task { @MainActor in await model.addImages(urls) }
        }
    }
}

private struct ImageTile: View {
    let image: NSImage?
    let onRemove: () -> Void
    @State private var hovered = false

    var body: some View {
        ZStack(alignment: .topTrailing) {
            Group {
                if let image {
                    Image(nsImage: image).resizable().aspectRatio(contentMode: .fill)
                } else {
                    Color.clear
                }
            }
            .frame(width: 64, height: 64)
            .clipped()
            Hovering { over in
                Button(action: onRemove) {
                    LucideIcon(.x, size: 10, strokeWidth: 3)
                        .foregroundStyle(over ? Theme.danger : .white)
                        .frame(width: 16, height: 16)
                        .background(Color.black.opacity(0.7), in: Circle())
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
            }
            .padding(2)
            .opacity(hovered ? 1 : 0)
        }
        .frame(width: 64, height: 64)
        .background(Theme.white(0.03))
        .clipShape(RoundedRectangle(cornerRadius: Theme.radiusLg))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.08)))
        .onHover { hovered = $0 }
        .animation(.easeOut(duration: 0.15), value: hovered)
    }
}

/// Thumbnails read once per file.
@MainActor
enum Thumbnail {
    private static var cache: [String: NSImage] = [:]

    static func file(_ path: String) -> NSImage? {
        guard !path.isEmpty else { return nil }
        if let hit = cache[path] { return hit }
        let image = NSImage(contentsOfFile: path)
        cache[path] = image
        return image
    }
}

/// The compact Focus Session / Resume Session buttons; both show when a live session can also be resumed.
struct TaskSessionActions: View {
    let store: TasksStore
    let task: VornTask

    var body: some View {
        let live = store.isSessionLive(task)
        let resume = store.canResume(task)
        if live || resume {
            WrapLayout(spacing: 8) {
                if live, let sid = task.assignedSessionId {
                    chip(.terminal, "Focus Session") { store.focusSession?(sid) }
                }
                if resume {
                    chip(.play, "Resume Session") { Task { await store.resumeSession(task) } }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16).padding(.vertical, 8)
            .overlay(alignment: .top) { Rectangle().fill(Theme.white(0.06)).frame(height: 1) }
        }
    }

    private func chip(_ glyph: LucideGlyph, _ label: String, action: @escaping () -> Void) -> some View {
        Hovering { hovered in
            Button(action: action) {
                HStack(spacing: 6) {
                    LucideIcon(glyph, size: 12)
                    Text(label)
                }
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(hovered ? Theme.ink : Theme.inkSecondary)
                .padding(.horizontal, 10).padding(.vertical, 6)
                .background(Theme.white(hovered ? 0.08 : 0.04), in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.08)))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }
}

/// The uppercase toggle row that opens and closes a section of the panel.
struct DetailSectionHeader<Trailing: View>: View {
    let open: Bool
    let glyph: LucideGlyph
    let title: String
    let toggle: () -> Void
    @ViewBuilder var trailing: Trailing

    var body: some View {
        Hovering { hovered in
            HStack(spacing: 8) {
                LucideIcon(open ? .chevronDown : .chevronRight, size: 11)
                LucideIcon(glyph, size: 12)
                Text(title.uppercased()).kerning(0.55)
                trailing
            }
            .font(.system(size: 11, weight: .medium))
            .foregroundStyle(hovered ? Theme.inkSecondary : Theme.inkFaint)
            .padding(.horizontal, 16).padding(.vertical, 10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
            .onTapGesture(perform: toggle)
        }
    }
}

/// "Workflow Runs (N)": the runs this task started or took part in, each as the workflows view draws it.
struct TaskWorkflowRuns: View {
    let store: TasksStore
    @Bindable var model: TaskDetailModel

    var body: some View {
        if !model.runs.isEmpty {
            VStack(spacing: 0) {
                DetailSectionHeader(open: model.showRuns, glyph: .workflow, title: "Workflow Runs (\(model.runs.count))") {
                    model.showRuns.toggle()
                } trailing: { EmptyView() }
                if model.showRuns, let workflows = store.workflows {
                    VStack(spacing: 8) {
                        ForEach(Array(model.runs.enumerated()), id: \.offset) { _, run in
                            let wf = model.workflow(for: run)
                            RunEntryCard(store: workflows, run: run, nodes: wf?.nodes ?? [],
                                         workflowName: run.workflowName ?? wf?.name, taskTitles: model.taskTitles,
                                         onViewFullOutput: { model.fullOutput = $0 },
                                         onResumeSession: { store.resumeStep($0) },
                                         taskContext: { store.runTaskContext($0) }, offersRerun: false)
                        }
                    }
                    .padding(.horizontal, 12).padding(.bottom, 12)
                }
            }
            .overlay(alignment: .top) { Rectangle().fill(Theme.white(0.06)).frame(height: 1) }
        }
    }
}

/// "Changes": the worktree's diff with its totals, the commit chip, the review bar, and the files.
struct TaskChangesSection: View {
    @Bindable var model: TaskDetailModel

    var body: some View {
        VStack(spacing: 0) {
            DetailSectionHeader(open: model.showDiff, glyph: .fileCode, title: "Changes") {
                model.showDiff.toggle()
            } trailing: {
                if let stat = model.stat {
                    HStack(spacing: 6) {
                        Text("+\(stat.insertions)").foregroundStyle(Theme.inkSecondary)
                        Text("-\(stat.deletions)").foregroundStyle(Theme.inkFaint)
                    }
                    .font(.system(size: 11, weight: .medium, design: .monospaced))
                }
                Spacer(minLength: 0)
                if model.hasChanges {
                    Hovering { hovered in
                        Button { model.commitDialogOpen = true } label: {
                            HStack(spacing: 4) {
                                LucideIcon(.gitCommitHorizontal, size: 11, strokeWidth: 1.5)
                                Text("Commit")
                            }
                            .font(.system(size: 10, weight: .medium))
                            .foregroundStyle(hovered ? Theme.ink : Theme.inkSecondary)
                            .padding(.horizontal, 8).padding(.vertical, 2)
                            .background(Theme.white(hovered ? 0.08 : 0.04), in: RoundedRectangle(cornerRadius: Theme.radius))
                            .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.06)))
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
                Hovering { hovered in
                    Button { Task { await model.loadDiff() } } label: {
                        SpinningGlyph(glyph: .refreshCw, size: 12, strokeWidth: 1.5, active: model.diffLoading)
                            .foregroundStyle(hovered ? .white : Theme.inkFaint)
                            .padding(2)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
            if model.showDiff {
                if !model.comments.isEmpty { feedbackBar }
                if model.diffLoading && model.diff == nil {
                    SpinningGlyph(glyph: .loaderCircle, size: 18, strokeWidth: 2, active: true)
                        .foregroundStyle(Theme.inkFaint)
                        .frame(maxWidth: .infinity).padding(.vertical, 32)
                } else if let diff = model.diff, !diff.files.isEmpty {
                    DiffFileList(files: diff.files, selectedFile: model.selectedFile) { model.selectedFile = $0 }
                    DiffContent(model: model, files: diff.files)
                } else {
                    Text(model.cwd.isEmpty ? "No project path" : "No uncommitted changes")
                        .font(.system(size: 12)).foregroundStyle(Theme.inkFaint)
                        .frame(maxWidth: .infinity).padding(.vertical, 24)
                }
            }
        }
        .overlay(alignment: .top) { Rectangle().fill(Theme.white(0.06)).frame(height: 1) }
    }

    private var feedbackBar: some View {
        HStack(spacing: 8) {
            LucideIcon(.messageSquare, size: 13).foregroundStyle(Theme.inkSecondary)
            Text("\(model.comments.count) comment\(model.comments.count == 1 ? "" : "s")")
                .font(.system(size: 12)).foregroundStyle(Theme.inkSecondary)
                .frame(maxWidth: .infinity, alignment: .leading)
            Hovering { hovered in
                Button { model.clearComments() } label: {
                    Text("Clear").font(.system(size: 11)).foregroundStyle(hovered ? Theme.inkSecondary : Theme.inkFaint)
                }
                .buttonStyle(.plain)
            }
            Hovering { hovered in
                Button { Task { await model.sendFeedback() } } label: {
                    HStack(spacing: 6) {
                        LucideIcon(.send, size: 11)
                        Text(model.sendsToAgent ? "Send to Agent" : "Copy Feedback")
                    }
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(Theme.surfaceBase)
                    .padding(.horizontal, 10).padding(.vertical, 4)
                    .background(hovered ? Color.white : Theme.ink, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 12).padding(.vertical, 8)
        .background(Theme.white(0.02))
        .overlay(alignment: .top) { Rectangle().fill(Theme.white(0.08)).frame(height: 1) }
    }
}

/// A glyph that turns while `active`, for Loader2 and a refreshing RefreshCw.
struct SpinningGlyph: View {
    let glyph: LucideGlyph
    var size: CGFloat
    var strokeWidth: CGFloat
    var active: Bool
    @State private var angle = 0.0

    var body: some View {
        LucideIcon(glyph, size: size, strokeWidth: strokeWidth)
            .rotationEffect(.degrees(active ? angle : 0))
            .onAppear(perform: spin)
            .onChange(of: active) { spin() }
    }

    private func spin() {
        guard active else { return }
        angle = 0
        withAnimation(.linear(duration: 1).repeatForever(autoreverses: false)) { angle = 360 }
    }
}

/// `flex flex-wrap`: children left to right, onto a new line when the row is full.
struct WrapLayout: Layout {
    var spacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? .infinity
        let rows = arrange(width: width, subviews: subviews)
        let height = rows.last.map { $0.y + $0.height } ?? 0
        let used = rows.map(\.width).max() ?? 0
        return CGSize(width: proposal.width ?? used, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        for row in arrange(width: bounds.width, subviews: subviews) {
            var x = bounds.minX
            for i in row.items {
                let size = subviews[i].sizeThatFits(.unspecified)
                subviews[i].place(at: CGPoint(x: x, y: bounds.minY + row.y), proposal: ProposedViewSize(size))
                x += size.width + spacing
            }
        }
    }

    private struct Row { var items: [Int] = []; var width: CGFloat = 0; var height: CGFloat = 0; var y: CGFloat = 0 }

    private func arrange(width: CGFloat, subviews: Subviews) -> [Row] {
        var rows: [Row] = []
        var row = Row()
        for i in subviews.indices {
            let size = subviews[i].sizeThatFits(.unspecified)
            if !row.items.isEmpty, row.width + spacing + size.width > width {
                rows.append(row)
                row = Row(y: row.y + row.height + spacing)
            }
            row.width += (row.items.isEmpty ? 0 : spacing) + size.width
            row.height = max(row.height, size.height)
            row.items.append(i)
        }
        if !row.items.isEmpty { rows.append(row) }
        return rows
    }
}
