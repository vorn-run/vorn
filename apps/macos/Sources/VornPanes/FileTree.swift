import SwiftUI
import VornCore
import VornUI

/// The tree's directory listings, loaded as folders open (FileTreePane).
@MainActor
@Observable
public final class FileTreeModel {
    let client: VornClient
    let cwd: String
    let remoteHostId: String?
    var root: [FileEntry]?
    var loading = true
    var cache: [String: [FileEntry]] = [:]
    var expanded: Set<String> = []
    var loadingDirs: Set<String> = []
    var filter = ""

    init(client: VornClient, cwd: String, remoteHostId: String?) {
        self.client = client
        self.cwd = cwd
        self.remoteHostId = remoteHostId
    }

    /// Folder names an offscreen render opens straight away.
    @MainActor public static var renderExpand: Set<String> = []

    func load() async {
        loading = true
        root = try? await client.listDir(cwd, remoteHostId: remoteHostId)
        loading = false
        guard !Self.renderExpand.isEmpty else { return }
        var queue = root ?? []
        while let e = queue.first {
            queue.removeFirst()
            if e.isDirectory, Self.renderExpand.contains(e.name) {
                await toggle(e)
                queue += cache[e.path] ?? []
            }
        }
    }

    func toggle(_ e: FileEntry) async {
        guard e.isDirectory else { return }
        if !expanded.contains(e.path) && cache[e.path] == nil {
            loadingDirs.insert(e.path)
            if let entries = try? await client.listDir(e.path, remoteHostId: remoteHostId) { cache[e.path] = entries }
            loadingDirs.remove(e.path)
        }
        if expanded.contains(e.path) { expanded.remove(e.path) } else { expanded.insert(e.path) }
    }

    /// computeFilterSets: what matches the filter, and the folders opened to show it.
    func filterSets() -> (matched: Set<String>, expand: Set<String>) {
        var matched = Set<String>(), expand = Set<String>()
        guard !filter.isEmpty else { return (matched, expand) }
        let lc = filter.lowercased()
        func visit(_ e: FileEntry) -> Bool {
            let selfMatch = e.name.lowercased().contains(lc)
            if e.isDirectory {
                var below = false
                for c in cache[e.path] ?? [] where visit(c) { below = true }
                if below { expand.insert(e.path) }
                if selfMatch || below { matched.insert(e.path); return true }
                return false
            }
            if selfMatch { matched.insert(e.path) }
            return selfMatch
        }
        for e in root ?? [] { _ = visit(e) }
        return (matched, expand)
    }

    /// The rows drawn, depth first, as the nested `TreeNode`s lay them out.
    func rows() -> [(entry: FileEntry?, depth: Int, emptyOf: String?)] {
        let (matched, expand) = filterSets()
        let filtering = !filter.isEmpty
        var out: [(FileEntry?, Int, String?)] = []
        func walk(_ entries: [FileEntry], _ depth: Int) {
            for e in entries {
                if filtering && !matched.contains(e.path) { continue }
                out.append((e, depth, nil))
                let open = expanded.contains(e.path) || (filtering && expand.contains(e.path))
                if e.isDirectory, open, let kids = cache[e.path] {
                    walk(kids, depth + 1)
                    if kids.isEmpty { out.append((nil, depth + 1, e.path)) }
                }
            }
        }
        walk(root ?? [], 0)
        return out
    }
}

/// FileTreePane + FilesPanel: the filter field and the tree rows.
struct FileTree: View {
    @State var model: FileTreeModel
    let selectedFile: String?
    let onSelect: (String) -> Void
    let onPin: (String) -> Void
    let onPopOut: ((String) -> Void)?

    static let rowHeight: CGFloat = 22
    static let indent: CGFloat = 16
    static let baseLeft: CGFloat = 8

    var body: some View {
        Group {
            if model.loading {
                SpinningGlyph(size: 20).foregroundStyle(Theme.gray500).frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if (model.root ?? []).isEmpty {
                Text("Empty directory").font(.ui(14)).foregroundStyle(Theme.gray500).frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                VStack(spacing: 0) {
                    filterField.padding(6)
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 0) {
                            ForEach(Array(model.rows().enumerated()), id: \.offset) { _, row in
                                if let e = row.entry {
                                    rowView(e, depth: row.depth)
                                } else {
                                    emptyRow(depth: row.depth)
                                }
                            }
                            if !model.filter.isEmpty && model.filterSets().matched.isEmpty {
                                Text("No matching files loaded").font(.ui(11)).italic().foregroundStyle(Theme.gray600)
                                    .padding(.horizontal, 12).padding(.vertical, 8)
                            }
                        }
                        .padding(.vertical, 2)
                    }
                    .scrollIndicators(.automatic)
                }
            }
        }
        .task(id: model.cwd) { await model.load() }
    }

    private var filterField: some View {
        HStack(spacing: 6) {
            LucideIcon(.search, size: 13).foregroundStyle(Theme.gray600)
            TextField("", text: $model.filter, prompt: Text("Filter files…").foregroundColor(Theme.gray600))
                .textFieldStyle(.plain).font(.ui(13)).foregroundStyle(Theme.gray200)
                .onExitCommand { model.filter = "" }
            if !model.filter.isEmpty {
                IconButton(.x, size: 11, padding: 2, color: Theme.gray600, help: "Clear filter") { model.filter = "" }
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(RoundedRectangle(cornerRadius: Theme.radius).fill(Theme.white(0.09)))
    }

    private func guides(_ depth: Int) -> some View {
        ZStack(alignment: .topLeading) {
            ForEach(0..<depth, id: \.self) { i in
                Rectangle().fill(Theme.hairline).frame(width: 1).offset(x: Self.baseLeft + 7 + CGFloat(i) * Self.indent)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .allowsHitTesting(false)
    }

    @ViewBuilder private func rowView(_ e: FileEntry, depth: Int) -> some View {
        let filtering = !model.filter.isEmpty
        if e.isDirectory {
            let open = model.expanded.contains(e.path) || (filtering && model.filterSets().expand.contains(e.path))
            Hovering { hover in
                HStack(spacing: 5) {
                    if model.loadingDirs.contains(e.path) {
                        SpinningGlyph(size: 14).foregroundStyle(Theme.gray600)
                    } else {
                        LucideIcon(.chevronRight, size: 14).foregroundStyle(Theme.gray500).rotationEffect(.degrees(open ? 90 : 0))
                    }
                    Text(e.name).font(.ui(13.5)).foregroundStyle(Theme.gray300).lineLimit(1)
                    Spacer(minLength: 0)
                }
                .padding(.leading, Self.baseLeft + CGFloat(depth) * Self.indent)
                .padding(.trailing, 8)
                .frame(height: Self.rowHeight)
                .background(hover ? Theme.white(0.05) : .clear)
                .background(guides(depth))
                .contentShape(Rectangle())
                .onTapGesture { Task { await model.toggle(e) } }
            }
        } else {
            let selected = selectedFile == e.path
            Hovering { hover in
                HStack(spacing: 5) {
                    FileTypeIcon(name: e.name, size: 16)
                    Text(e.name).font(.ui(13.5)).lineLimit(1)
                        .foregroundStyle(selected ? Theme.gray200 : hover ? Theme.gray300 : Theme.gray400)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    if let onPopOut {
                        Button { onPopOut(e.path) } label: {
                            LucideIcon(.squareArrowOutUpRight, size: 11).foregroundStyle(Theme.inkGhost).padding(2)
                        }
                        .buttonStyle(.plain)
                        .opacity(hover ? 1 : 0)
                        .help("Open as its own card")
                        .accessibilityLabel("Open \(e.name) as its own card")
                    }
                }
                .padding(.leading, Self.baseLeft + CGFloat(depth) * Self.indent + 16)
                .padding(.trailing, 4)
                .frame(height: Self.rowHeight)
                .background(selected ? Theme.white(0.10) : hover ? Theme.white(0.05) : .clear)
                .background(guides(depth))
                .contentShape(Rectangle())
                .onTapGesture(count: 2) { onPin(e.path) }
                .simultaneousGesture(TapGesture().onEnded { onSelect(e.path) })
            }
        }
    }

    private func emptyRow(depth: Int) -> some View {
        Text("empty").font(.ui(11)).italic().foregroundStyle(Theme.gray600)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.leading, Self.baseLeft + CGFloat(depth) * Self.indent + 16)
            .frame(height: Self.rowHeight)
            .background(guides(depth))
    }
}
