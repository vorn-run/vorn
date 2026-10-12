import SwiftUI
import VornCore
import VornUI

private let keySep = "\u{0}"

private let verdictStyles: [String: (label: String, fg: Color, bg: Color)] = [
    "keep": ("Keep", Theme.gray400, Theme.white(0.05)),
    "review": ("Review", Theme.amber300, Theme.amber500.opacity(0.12)),
    "reclaim": ("Build output only", Color(hex: 0x8EC5FF), Theme.blue500.opacity(0.12)),
    "remove": ("Removable", Color(hex: 0x8EC5FF), Theme.blue500.opacity(0.12)),
    "orphan": ("Orphan directory", Theme.red300, Theme.red500.opacity(0.12)),
]

extension SettingsModel {
    var wtEntries: [WorktreeInventoryEntry] { inventory?.projects.flatMap(\.entries) ?? [] }
    var wtSelectedEntries: [WorktreeInventoryEntry] { wtEntries.filter { wtSelected.contains($0.path) } }
    var wtReclaimTargets: [WorktreeInventoryEntry] { wtEntries.filter { $0.isSelectable && $0.artifactBytes > 0 } }

    func wtRescan() {
        inventoryLoading = true
        Task { await wtLoad(refresh: true) }
    }

    public func wtLoad(refresh: Bool) async {
        await loadInventory(refresh: refresh)
        let live = Set(wtEntries.map(\.path))
        wtSelected = wtSelected.filter { live.contains($0) }
        let liveBranches = Set(inventory?.projects.flatMap { p in p.staleBranches.map { p.projectPath + keySep + $0.name } } ?? [])
        wtBranches = wtBranches.filter { liveBranches.contains($0) }
    }

    /// Runs a batch, reports what it did and rescans (`run` in WorktreeSettings.tsx).
    func wtRun(loading: String, success: String, _ fn: @escaping () async throws -> WorktreeActionResult) {
        guard !wtBusy else { return }
        wtBusy = true
        wtConfirming = nil
        onToast(loading, .info)
        Task {
            do {
                let r = try await fn()
                if r.succeeded.isEmpty, let first = r.failed.first { throw VornStoreError(first.error) }
                let freed = r.freedBytes > 0 ? " · freed \(formatBytes(r.freedBytes))" : ""
                if !r.succeeded.isEmpty { onToast("\(success) \(r.succeeded.count)\(freed)", .success) }
                for f in r.failed { onToast("\(f.path.split(separator: "/").last.map(String.init) ?? f.path): \(f.error)", .error) }
            } catch {
                onToast(error.localizedDescription, .error)
            }
            wtBusy = false
            wtRescan()
        }
    }

    func wtRemoveSelected() {
        let sel = wtSelectedEntries
        wtRun(loading: "Removing worktrees…", success: "Removed") { [self] in
            var combined = WorktreeActionResult()
            let registered = sel.filter { $0.kind == "registered" }
            if !registered.isEmpty {
                let r = try await removeWorktrees(registered)
                combined.succeeded += r.succeeded
                combined.failed += r.failed
                combined.freedBytes += r.freedBytes
                combined.deletedBranches += r.deletedBranches
            }
            let orphans = sel.filter { $0.kind == "orphan-dir" }
            if !orphans.isEmpty {
                let r = try await pruneOrphans(orphans.map(\.path))
                combined.succeeded += r.succeeded
                combined.failed += r.failed
                combined.freedBytes += r.freedBytes
            }
            return combined
        }
    }
}

/// WorktreeSettings.tsx: what every worktree costs on disk, and what can safely go.
struct WorktreeSettings: View {
    @Bindable var model: SettingsModel

    var body: some View {
        let entries = model.wtEntries
        var onDisk = 0.0, artifacts = 0.0, removable = 0.0
        for e in entries {
            onDisk += e.sizeBytes
            if e.isSelectable {
                artifacts += e.artifactBytes
                if e.verdict.level == "remove" || e.verdict.level == "orphan" { removable += e.sizeBytes }
            }
        }
        let projects = model.inventory?.projects ?? []
        let hasAnything = projects.contains { !$0.entries.isEmpty || !$0.staleBranches.isEmpty }
        return VStack(alignment: .leading, spacing: 0) {
            SettingsPageHeader(title: "Worktrees", description: "What every worktree costs on disk, and what can safely go") {
                QuietButton(label: "Rescan", icon: model.inventoryLoading ? .loaderCircle : .refreshCw, iconSize: 13, bordered: false,
                            fontSize: 11, radius: Theme.radiusLg, color: Theme.gray300, padding: (12, 6),
                            disabled: model.inventoryLoading || model.wtBusy) { model.wtRescan() }
            }
            summary(onDisk: onDisk, artifacts: artifacts, removable: removable)
            if model.inventoryLoading && model.inventory == nil {
                HStack(spacing: 8) { Spinner(size: 15); Text("Measuring worktrees…") }
                    .font(.ui(13)).foregroundStyle(Theme.gray500).frame(maxWidth: .infinity).padding(.vertical, 40)
            }
            if !model.inventoryLoading && !hasAnything {
                Text("No worktrees yet. They appear here as soon as you create one.").font(.ui(13)).foregroundStyle(Theme.gray500)
                    .frame(maxWidth: .infinity).padding(.vertical, 40)
            }
            ForEach(projects, id: \.projectPath) { ProjectGroup(model: model, project: $0) }
            if entries.contains(where: { $0.isSelectable && $0.verdict.autoSelect }) {
                Button("Select everything merged and idle") {
                    model.wtConfirming = nil
                    model.wtSelected = Set(entries.filter { $0.isSelectable && $0.verdict.autoSelect }.map(\.path))
                }
                .buttonStyle(.plain).font(.ui(11)).foregroundStyle(Theme.gray500).padding(.top, 16)
            }
        }
        .padding(.bottom, 96)
        .task { if model.inventory == nil { await model.wtLoad(refresh: false) } }
    }

    private func summary(onDisk: Double, artifacts: Double, removable: Double) -> some View {
        let aPct = onDisk > 0 ? min(100, artifacts / onDisk * 100) : 0
        let rPct = onDisk > 0 ? min(100 - aPct, removable / onDisk * 100) : 0
        let confirming = model.wtConfirming == "reclaim"
        let targets = model.wtReclaimTargets
        return VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .bottom) {
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 6) { LucideIcon(.hardDrive, size: 12); Text("ON DISK").tracking(0.5) }
                        .font(.ui(11)).foregroundStyle(Theme.gray500)
                    Text(formatBytes(onDisk)).font(.system(size: 30, weight: .semibold).monospacedDigit()).foregroundStyle(.white).lineHeight(38, size: 30)
                }
                Spacer(minLength: 24)
                VStack(alignment: .trailing, spacing: 4) {
                    Text("BUILD OUTPUT — REBUILT BY A REINSTALL").font(.ui(11)).tracking(0.5).foregroundStyle(Theme.gray500)
                    Text(formatBytes(artifacts)).font(.system(size: 20, weight: .semibold).monospacedDigit()).foregroundStyle(Color(hex: 0x8EC5FF)).lineHeight(28, size: 20)
                }
            }
            GeometryReader { g in
                HStack(spacing: 0) {
                    Rectangle().fill(Theme.blue400.opacity(0.7)).frame(width: g.size.width * aPct / 100)
                    Rectangle().fill(Theme.blue400.opacity(0.25)).frame(width: g.size.width * rPct / 100)
                    Spacer(minLength: 0)
                }
                .background(Theme.white(0.06))
                .clipShape(Capsule())
            }
            .frame(height: 8)
            .padding(.top, 16)
            HStack(spacing: 16) {
                Text("Build output")
                Text("Removable worktrees")
                Spacer()
                Text("\(formatBytes(removable)) in merged worktrees").monospacedDigit()
            }
            .font(.ui(11)).foregroundStyle(Theme.gray500).padding(.top, 8)
            if artifacts > 0 {
                HStack(spacing: 8) {
                    let label = confirming
                        ? "Delete build output in \(targets.count) worktree\(targets.count == 1 ? "" : "s") — frees \(formatBytes(artifacts))"
                        : "Reclaim \(formatBytes(artifacts)) of build output"
                    QuietButton(label: label, icon: .trash2, bordered: confirming, fontSize: 11,
                                fill: confirming ? Theme.blue500.opacity(0.25) : Theme.blue500.opacity(0.12),
                                hoverFill: confirming ? Theme.blue500.opacity(0.35) : Theme.blue500.opacity(0.2),
                                color: confirming ? .white : Color(hex: 0x8EC5FF), padding: (12, 8),
                                disabled: model.inventoryLoading || model.wtBusy) {
                        if confirming {
                            let paths = targets.map(\.path)
                            model.wtRun(loading: "Deleting build output…", success: "Reclaimed") { try await model.reclaim(paths) }
                        } else {
                            model.wtConfirming = "reclaim"
                        }
                    }
                    if confirming {
                        Button("Cancel") { model.wtConfirming = nil }.buttonStyle(.plain).font(.ui(11)).foregroundStyle(Theme.gray400).padding(.horizontal, 12)
                    }
                }
                .padding(.top, 16)
            }
        }
        .padding(20)
        .background(RoundedRectangle(cornerRadius: Theme.radiusXl).fill(Theme.white(0.02)))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusXl).strokeBorder(Theme.hairline, lineWidth: 1))
        .padding(.bottom, 24)
    }
}

private struct ProjectGroup: View {
    @Bindable var model: SettingsModel
    let project: WorktreeProjectInventory
    @State private var expanded = false

    var body: some View {
        let entries = project.entries.filter { !$0.isMain }.sorted { $0.sizeBytes > $1.sizeBytes }
        let total = project.entries.reduce(0) { $0 + $1.sizeBytes }
        if !entries.isEmpty || !project.staleBranches.isEmpty || project.error != nil {
            VStack(alignment: .leading, spacing: 0) {
                HStack(alignment: .firstTextBaseline) {
                    Text(project.projectName).font(.ui(13, weight: .medium)).foregroundStyle(Theme.gray200).lineLimit(1)
                    if let b = project.defaultBranch { Text("vs \(b)").font(.system(size: 11, design: .monospaced)).foregroundStyle(Theme.gray600) }
                    if project.remoteHostId != nil { Text("remote").font(.ui(11)).foregroundStyle(Theme.gray600) }
                    Spacer()
                    Text(formatBytes(total)).font(.ui(11).monospacedDigit()).foregroundStyle(Theme.gray500)
                }
                .padding(.bottom, 8)
                .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }
                if let err = project.error {
                    Text("Could not scan — \(err)").font(.ui(11)).foregroundStyle(Theme.gray500).padding(.vertical, 12)
                }
                ForEach(Array(entries.enumerated()), id: \.element.path) { i, e in row(e, last: i == entries.count - 1) }
                if !project.staleBranches.isEmpty { staleStrip.padding(.top, 12) }
            }
            .padding(.bottom, 24)
        }
    }

    private func row(_ e: WorktreeInventoryEntry, last: Bool) -> some View {
        let style = verdictStyles[e.verdict.level] ?? ("", Theme.gray400, .clear)
        let checked = model.wtSelected.contains(e.path)
        return HStack(spacing: 12) {
            Toggle("", isOn: Binding(get: { checked }, set: { _ in
                model.wtConfirming = nil
                if checked { model.wtSelected.remove(e.path) } else { model.wtSelected.insert(e.path) }
            }))
            .toggleStyle(.checkbox).labelsHidden().disabled(!e.isSelectable)
            .accessibilityLabel("Select \(e.name)")
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 8) {
                    LucideIcon(.folderGit2, size: 13).foregroundStyle(Theme.gray600)
                    Text(e.name).font(.ui(13)).foregroundStyle(Theme.gray200).lineLimit(1)
                    Text(style.label).font(.ui(10)).foregroundStyle(style.fg).padding(.horizontal, 6).padding(.vertical, 2)
                        .background(RoundedRectangle(cornerRadius: Theme.radius).fill(style.bg))
                }
                HStack(spacing: 8) {
                    Text(e.branch ?? "no branch").font(.system(size: 11, design: .monospaced)).lineLimit(1)
                    Text("·")
                    Text(e.idleDays.map { $0 == 0 ? "today" : "\($0)d idle" } ?? "—")
                    if !e.verdict.reasons.isEmpty {
                        Text("·")
                        Text(e.verdict.reasons.joined(separator: ", ")).lineLimit(1)
                    }
                }
                .font(.ui(11)).foregroundStyle(Theme.gray600)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            VStack(alignment: .trailing, spacing: 4) {
                Text(e.sizeMeasured ? formatBytes(e.sizeBytes) : "—").font(.ui(11).monospacedDigit()).foregroundStyle(Theme.gray300)
                if e.artifactBytes > 0 {
                    GeometryReader { g in
                        ZStack(alignment: .leading) {
                            Capsule().fill(Theme.white(0.06))
                            Capsule().fill(Theme.blue400.opacity(0.6)).frame(width: g.size.width * (e.sizeBytes > 0 ? e.artifactBytes / e.sizeBytes : 0))
                        }
                    }
                    .frame(height: 4)
                }
            }
            .frame(width: 86)
        }
        .padding(.vertical, 10)
        .overlay(alignment: .bottom) { if !last { Rectangle().fill(Theme.white(0.04)).frame(height: 1) } }
        .opacity(e.isSelectable ? 1 : 0.5)
    }

    private var staleStrip: some View {
        let branches = project.staleBranches
        let merged = branches.filter(\.isMerged)
        let key = { (name: String) in project.projectPath + keySep + name }
        let chosen = branches.filter { model.wtBranches.contains(key($0.name)) }
        return VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                LucideIcon(.gitBranch, size: 13).foregroundStyle(Theme.gray600)
                Text("\(branches.count) branch\(branches.count == 1 ? "" : "es") left behind by removed worktrees\(merged.isEmpty ? "" : " — \(merged.count) already merged")")
                    .font(.ui(11)).foregroundStyle(Theme.gray400)
                Spacer()
                Button(expanded ? "Hide" : "Show") { expanded.toggle() }.buttonStyle(.plain).font(.ui(11)).foregroundStyle(Theme.gray500)
            }
            if expanded {
                FlowLayout(spacing: 6) {
                    ForEach(branches, id: \.name) { b in
                        let sel = model.wtBranches.contains(key(b.name))
                        Button {
                            if sel { model.wtBranches.remove(key(b.name)) } else { model.wtBranches.insert(key(b.name)) }
                        } label: {
                            Text(b.name).font(.system(size: 11, design: .monospaced))
                                .foregroundStyle(sel ? Color(hex: 0xFFC9C9) : b.isMerged ? Theme.gray400 : Theme.amber300.opacity(0.8))
                                .padding(.horizontal, 8).padding(.vertical, 4)
                                .background(RoundedRectangle(cornerRadius: Theme.radius).fill(sel ? Theme.red500.opacity(0.15) : b.isMerged ? Theme.white(0.03) : Theme.amber500.opacity(0.06)))
                                .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(sel ? Theme.red400.opacity(0.4) : b.isMerged ? Theme.white(0.08) : Theme.amber500.opacity(0.25), lineWidth: 1))
                        }
                        .buttonStyle(.plain)
                        .help(b.isMerged ? "Merged — deleting loses nothing" : "Not merged — git will refuse to delete it")
                    }
                }
                .padding(.top, 10)
                HStack(spacing: 8) {
                    Button("Select \(merged.count) merged") {
                        for b in merged { model.wtBranches.insert(key(b.name)) }
                    }
                    .buttonStyle(.plain).font(.ui(11)).foregroundStyle(Theme.gray500).disabled(merged.isEmpty).opacity(merged.isEmpty ? 0.4 : 1)
                    Spacer()
                    QuietButton(label: "Delete \(chosen.isEmpty ? "" : "\(chosen.count) ")branch\(chosen.count == 1 ? "" : "es")", icon: .trash2,
                                iconSize: 11, bordered: false, fontSize: 11, fill: Theme.red500.opacity(0.1), hoverFill: Theme.red500.opacity(0.2),
                                color: Theme.red300, padding: (10, 4), disabled: chosen.isEmpty || model.wtBusy) {
                        let names = chosen.map(\.name)
                        model.wtRun(loading: "Deleting branches…", success: "Deleted") {
                            try await model.deleteBranches(projectPath: project.projectPath, branches: names)
                        }
                    }
                }
                .padding(.top, 12)
                (Text("Deletion uses ") + Text("git branch -d").font(.system(size: 10, design: .monospaced)) + Text(", so git refuses anything that isn't merged."))
                    .font(.ui(10)).foregroundStyle(Theme.gray600).padding(.top, 8)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.white(0.02)))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.hairline, lineWidth: 1))
    }
}

/// The floating selection bar at the window's foot (SelectionBar in WorktreeSettings.tsx).
struct WorktreeSelectionHost: View {
    @Bindable var model: SettingsModel

    var body: some View {
        let sel = model.wtSelectedEntries
        if model.category == .worktrees && !sel.isEmpty {
            let bytes = sel.reduce(0) { $0 + $1.sizeBytes }
            let dirty = sel.filter(\.isDirty).count
            let unmerged = sel.filter { $0.kind == "registered" && !$0.isMerged }.count
            let orphans = sel.filter { $0.kind == "orphan-dir" }.count
            let confirming = model.wtConfirming == "remove"
            let warnings = [
                dirty > 0 ? "\(dirty) \(dirty == 1 ? "has" : "have") uncommitted changes that will be lost" : nil,
                unmerged > 0 ? "\(unmerged) not merged — the branch is kept" : nil,
                orphans > 0 ? "\(orphans) deleted from disk directly" : nil,
            ].compactMap { $0 }
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 12) {
                    VStack(alignment: .leading, spacing: 4) {
                        (Text("\(sel.count) selected · ").foregroundColor(.white) + Text("frees \(formatBytes(bytes))").foregroundColor(Theme.gray300))
                            .font(.ui(13))
                        if !warnings.isEmpty {
                            HStack(alignment: .top, spacing: 6) {
                                LucideIcon(.triangleAlert, size: 11).foregroundStyle(Theme.amber400).padding(.top, 2)
                                Text(warnings.joined(separator: " · ")).font(.ui(11)).foregroundStyle(Theme.amber300.opacity(0.85))
                            }
                        }
                    }
                    Spacer(minLength: 0)
                    Button("Clear") { model.wtSelected = []; model.wtConfirming = nil }
                        .buttonStyle(.plain).font(.ui(11)).foregroundStyle(Theme.gray400).padding(.horizontal, 10)
                    QuietButton(label: confirming ? "Yes — remove \(sel.count) and free \(formatBytes(bytes))" : "Remove", icon: .trash2,
                                iconSize: 12, bordered: confirming, fontSize: 11,
                                fill: Theme.red500.opacity(confirming ? 0.3 : 0.12), hoverFill: Theme.red500.opacity(confirming ? 0.45 : 0.2),
                                color: confirming ? .white : Theme.red300, padding: (12, 6), disabled: model.wtBusy) {
                        if confirming { model.wtRemoveSelected() } else { model.wtConfirming = "remove" }
                    }
                }
                if confirming { Text("This cannot be undone.").font(.ui(11)).foregroundStyle(Theme.gray500).padding(.top, 8) }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .frame(width: 560)
            .background(RoundedRectangle(cornerRadius: Theme.radiusXl).fill(Theme.surfaceOverlay))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusXl).strokeBorder(Theme.white(0.1), lineWidth: 1))
            .floatingShadow()
            .padding(.bottom, 24)
            .padding(.leading, 224)
        }
    }
}

/// Wraps children onto new lines (`flex flex-wrap`).
struct FlowLayout: Layout {
    var spacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(width: proposal.width ?? .infinity, subviews)
        return CGSize(width: proposal.width ?? rows.map(\.width).max() ?? 0, height: rows.last.map { $0.y + $0.height } ?? 0)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var i = 0
        for row in arrange(width: bounds.width, subviews) {
            var x = bounds.minX
            for _ in 0..<row.count {
                let s = subviews[i].sizeThatFits(.unspecified)
                subviews[i].place(at: CGPoint(x: x, y: bounds.minY + row.y), proposal: .unspecified)
                x += s.width + spacing
                i += 1
            }
        }
    }

    private func arrange(width: CGFloat, _ subviews: Subviews) -> [(y: CGFloat, height: CGFloat, width: CGFloat, count: Int)] {
        var rows: [(y: CGFloat, height: CGFloat, width: CGFloat, count: Int)] = []
        var x: CGFloat = 0, y: CGFloat = 0, h: CGFloat = 0, n = 0
        for v in subviews {
            let s = v.sizeThatFits(.unspecified)
            if n > 0 && x + s.width > width {
                rows.append((y, h, x - spacing, n))
                y += h + spacing
                x = 0; h = 0; n = 0
            }
            x += s.width + spacing
            h = max(h, s.height)
            n += 1
        }
        if n > 0 { rows.append((y, h, x - spacing, n)) }
        return rows
    }
}
