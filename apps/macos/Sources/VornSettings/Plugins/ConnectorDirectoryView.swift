import AppKit
import SwiftUI
import VornCore
import VornUI

/// ConnectorDirectory.tsx: search, filters, the listings by category, and the catalog's age.
struct ConnectorDirectoryView: View {
    @Bindable var model: PluginsModel
    @State private var search = ""
    @State private var category = ""
    @State private var kind = ""
    @State private var rung = ""

    private var all: [ConnectorListing] { model.listings }

    private var visible: [ConnectorListing] {
        var list = ConnectorBrowse.filter(all, query: search)
        list = ConnectorBrowse.filter(list, category: category)
        list = ConnectorBrowse.filter(list, kind: ConnectorKind(rawValue: kind))
        return ConnectorBrowse.filter(list, rung: ConnectorAuthRung(rawValue: rung))
    }

    var body: some View {
        let visible = visible
        let sections = ConnectorBrowse.sections(visible)
        let pendingKey = model.pending?.rowKey.flatMap { key in visible.contains { $0.key == key } ? key : nil }
        VStack(alignment: .leading, spacing: 0) {
            controls.padding(.bottom, 4)
            if let error = model.installError, model.pending == nil {
                ToneLine(tone: .broken, text: error).padding(.top, 8)
            }
            if model.pending != nil && pendingKey == nil {
                PendingSheet(model: model).padding(.top, 8)
            }
            if model.catalogLoading && model.catalog == nil {
                HStack(spacing: 6) {
                    Spinner(size: 12)
                    Text("Loading the directory…").font(.ui(12))
                }
                .foregroundStyle(Theme.gray500).padding(.vertical, 16)
            } else if visible.isEmpty {
                Text(search.isEmpty && category.isEmpty && kind.isEmpty && rung.isEmpty
                     ? "No connectors available. Check your connection and try again." : "Nothing matches that.")
                    .font(.ui(13)).foregroundStyle(Theme.gray500).padding(.vertical, 16)
            } else {
                ForEach(sections, id: \.category) { section in
                    if sections.count > 1 {
                        Text(section.category.uppercased()).font(.ui(10)).tracking(0.8).foregroundStyle(Theme.gray600)
                            .padding(.top, 16).padding(.bottom, 4)
                    }
                    ForEach(section.listings) { listing in
                        ConnectorRow(model: model, listing: listing)
                        if listing.key == pendingKey {
                            PendingSheet(model: model).padding(.top, 8).padding(.bottom, 12)
                        }
                    }
                }
            }
            footer
        }
    }

    private var controls: some View {
        let categories = ConnectorBrowse.categories(all)
        let kinds = ConnectorBrowse.kinds(all)
        let rungs = ConnectorBrowse.authRungs(all)
        return VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                LucideIcon(.search, size: 12).foregroundStyle(Theme.gray600)
                ZStack(alignment: .leading) {
                    if search.isEmpty {
                        Text("Search connectors").font(.ui(12)).foregroundStyle(Theme.gray600).allowsHitTesting(false)
                    }
                    TextField("", text: $search).textFieldStyle(.plain).font(.ui(12)).foregroundStyle(Theme.gray200)
                }
            }
            .padding(.horizontal, 10).padding(.vertical, 6)
            .background(RoundedRectangle(cornerRadius: Theme.radiusSm).fill(Theme.white(0.05)))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusSm).strokeBorder(Theme.white(0.1), lineWidth: 1))

            HStack(spacing: 8) {
                QuietButton(label: "Install from file", icon: .folderOpen, iconSize: 11, fill: .clear, hoverFill: Theme.white(0.06),
                            color: Theme.gray300, hoverColor: .white, padding: (8, 6), action: pickFile)
                    .help("Install a .vorn.tgz you already have")
                if categories.count > 1 {
                    PluginSelect(options: [("", "All categories")] + categories.map { ($0, $0) }, value: category) { category = $0 }
                }
                if kinds.count > 1 {
                    PluginSelect(options: [("", "Everything")] + kinds.map { ($0.rawValue, ConnectorBrowse.kindLabel($0)) },
                                 value: kind) { kind = $0 }
                }
                if rungs.count > 1 {
                    PluginSelect(options: [("", "Any sign-in")] + rungs.map { ($0.rawValue, ConnectorBrowse.rung($0).label) },
                                 value: rung) { rung = $0 }
                }
            }
        }
    }

    private var footer: some View {
        HStack(spacing: 6) {
            Text(ConnectorBrowse.catalogAge(fetchedAt: model.catalog?.fetchedAt))
            Text("·")
            Hovering { hover in
                Button {
                    Task { await model.refreshCatalog() }
                } label: {
                    HStack(spacing: 4) {
                        if model.catalogRefreshing { Spinner(size: 9) } else { LucideIcon(.refreshCw, size: 9) }
                        Text(model.catalogRefreshing ? "Checking" : "Check now")
                    }
                    .foregroundStyle(hover ? Theme.gray300 : Theme.gray500)
                }
                .buttonStyle(.plain)
                .disabled(model.catalogRefreshing)
            }
        }
        .font(.ui(11)).foregroundStyle(Theme.gray600)
        .padding(.top, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
        .padding(.top, 8)
    }

    private func pickFile() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.allowedContentTypes = [.init(filenameExtension: "tgz")].compactMap { $0 }
        panel.message = "Choose a .vorn.tgz pack to install"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        Task { await model.inspectFile(url.path) }
    }
}

/// ConnectorRow: one listing in the directory.
struct ConnectorRow: View {
    @Bindable var model: PluginsModel
    let listing: ConnectorListing

    var body: some View {
        let state = model.state(listing)
        let status = PackStatusView(state)
        HStack(alignment: .top, spacing: 12) {
            Hovering { hover in
                Button {
                    model.selectedKey = listing.key
                } label: {
                    HStack(alignment: .top, spacing: 12) {
                        ListingIcon(id: listing.id, icon: listing.icon, packaged: listing.packaged, size: 17, color: Theme.gray200)
                            .frame(width: 32, height: 32)
                            .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.white(0.05)))
                            .padding(.top, 2)
                        info(state: state, status: status, hover: hover)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            HStack(spacing: 6) {
                if listing.source != .builtin && listing.source != .mcp, let action = status.action {
                    PluginButton(label: status.label, icon: action == .install ? .download : .refreshCw, busy: status.busy) {
                        Task { await model.inspect(listing, direct: true) }
                    }
                }
                if !listing.implicitlyConnected && model.canAdd(listing) {
                    PluginButton(label: listing.connectedCount > 0 ? "Add another" : listing.source == .mcp ? "Add server" : "Add",
                                 icon: .plus) {
                        model.adding = listing
                    }
                }
            }
            .frame(maxHeight: .infinity, alignment: .center)
        }
        .padding(.vertical, 12)
        .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
    }

    private func info(state: PackState, status: PackStatusView, hover: Bool) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Text(listing.name).font(.ui(13.5, weight: .medium)).foregroundStyle(Theme.gray200).lineLimit(1)
                    .underline(hover, color: Theme.white(0.25))
                if let badge = ConnectorBrowse.kindBadge(listing.kind) { PluginBadge(text: badge) }
                if let rung = listing.authRung { PluginBadge(text: ConnectorBrowse.rung(rung).badge) }
                if let verified = listing.verified {
                    PluginBadge(text: "✓ verified")
                        .help("Checked \(verified.checks.joined(separator: ", ")) against v\(verified.version)")
                }
            }
            if let description = listing.description, !description.isEmpty {
                Text(description).font(.ui(12.5)).foregroundStyle(Theme.gray500).lineSpacing(1)
                    .fixedSize(horizontal: false, vertical: true).padding(.top, 2)
            }
            Text(PluginFacts.directory(listing, details: ConnectorBrowse.details(listing, builtIns: model.builtIns),
                                       activeCards: model.activeCards.map { $0[listing.id] ?? 0 }))
                .font(.ui(11)).foregroundStyle(Theme.gray600).padding(.top, 6)
            if let percent = status.percent {
                GeometryReader { geo in
                    Rectangle().fill(Theme.ink).frame(width: geo.size.width * min(max(percent, 0), 100) / 100, height: 1)
                }
                .frame(maxWidth: 220).frame(height: 1)
                .background(Theme.white(0.08))
                .padding(.top, 8)
            }
            if state != .absent {
                ToneLine(tone: status.tone, text: state == .notReleased ? status.label : (status.detail ?? status.label))
                    .padding(.top, 6)
            }
            HStack(spacing: 2) {
                Text("Details")
                LucideIcon(.chevronRight, size: 11)
            }
            .font(.ui(11)).foregroundStyle(hover ? Theme.gray300 : Theme.gray500).padding(.top, 6)
        }
        .multilineTextAlignment(.leading)
    }
}

/// The muted fact lines the directory and the installed list write under a name.
enum PluginFacts {
    static func offers(_ listing: ConnectorListing, details: ConnectorDetails) -> [String] {
        if listing.kind == .extension {
            let text = ExtensionCopy.describeContributions(listing.contributes)
            return text.isEmpty ? [] : [text]
        }
        return (details.triggers.isEmpty ? [] : [ExtensionCopy.count(details.triggers.count, "trigger")])
            + (details.actions.isEmpty ? [] : [ExtensionCopy.count(details.actions.count, "action")])
    }

    /// ConnectorDirectory.tsx facts.
    static func directory(_ listing: ConnectorListing, details: ConnectorDetails, activeCards: Int?) -> String {
        var parts = [listing.category]
        let offers = offers(listing, details: details)
        if !offers.isEmpty { parts.append(offers.joined(separator: ", ")) }
        if let version = listing.catalogItem?.version, !version.isEmpty { parts.append("v\(version)") }
        if listing.kind == .extension {
            if listing.pack != nil, let activeCards {
                parts.append(activeCards > 0 ? "on \(ExtensionCopy.count(activeCards, "card"))" : "on no open card")
            }
        } else if listing.connectedCount > 0 {
            parts.append("in use")
        } else if listing.implicitlyConnected {
            parts.append("ready")
        }
        return parts.joined(separator: " · ")
    }
}

/// The confirm sheet for whatever pack is staged.
struct PendingSheet: View {
    @Bindable var model: PluginsModel

    var body: some View {
        if let pending = model.pending {
            VStack(alignment: .leading, spacing: 6) {
                PackInstallConfirm(preview: pending.preview, busy: model.installing,
                                   onConfirm: { Task { await model.confirm() } }, onCancel: { model.cancel() })
                if let error = model.installError { ToneLine(tone: .broken, text: error) }
            }
        }
    }
}
