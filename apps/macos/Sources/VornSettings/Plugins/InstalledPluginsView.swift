import SwiftUI
import VornCore
import VornUI

/// InstalledPlugins.tsx: what has files on this machine, extensions first.
struct InstalledPluginsView: View {
    @Bindable var model: PluginsModel

    var body: some View {
        let installed = model.listings.filter { $0.pack != nil }
        let groups = [ConnectorKind.extension, .connector].map { kind in (kind, installed.filter { $0.kind == kind }) }
            .filter { !$0.1.isEmpty }
        if installed.isEmpty {
            HStack(spacing: 4) {
                Text("Nothing is installed yet.")
                PluginLink(label: "Browse", size: 13, color: Theme.gray300, hoverColor: .white) { model.show(.browse) }
                    .underline()
                Text("for a connector or an extension.")
            }
            .font(.ui(13)).foregroundStyle(Theme.gray500).padding(.vertical, 16)
        } else {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(groups, id: \.0) { kind, listings in
                    if groups.count > 1 {
                        Text(ConnectorBrowse.kindLabel(kind).uppercased()).font(.ui(10)).tracking(0.8).foregroundStyle(Theme.gray600)
                            .padding(.top, 16).padding(.bottom, 4)
                    }
                    ForEach(listings) { listing in
                        InstalledRow(model: model, listing: listing)
                        if listing.key == model.pending?.rowKey {
                            PendingSheet(model: model).padding(.top, 8).padding(.bottom, 12)
                        }
                    }
                }
            }
        }
    }
}

/// InstalledRow: an installed pack, what it offers, and its update and uninstall buttons.
struct InstalledRow: View {
    @Bindable var model: PluginsModel
    let listing: ConnectorListing

    var body: some View {
        let state = model.state(listing)
        let status = PackStatusView(state)
        let removing = model.isBusy(.remove, listing.id)
        HStack(alignment: .top, spacing: 12) {
            Hovering { hover in
                Button {
                    model.selectedKey = listing.key
                } label: {
                    HStack(alignment: .top, spacing: 12) {
                        ListingIcon(id: listing.id, icon: listing.icon, packaged: true, size: 17, color: Theme.gray200)
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
                if listing.catalogItem != nil, let action = status.action {
                    PluginButton(label: status.label, icon: action == .install ? .download : .refreshCw, busy: status.busy) {
                        Task { await model.inspect(listing, direct: true) }
                    }
                    .help(action == .update ? "Install the newer version" : "Try installing again")
                }
                if listing.kind != .extension && !listing.implicitlyConnected && listing.connectedCount == 0 {
                    PluginButton(label: "Add connection", icon: .plus) { model.adding = listing }
                        .help("Connect this connector to an account")
                }
                PluginButton(label: "Uninstall", icon: .trash2, busy: removing) {
                    Task { await model.removePack(listing.id) }
                }
                .help("Delete the installed files")
            }
            .frame(maxHeight: .infinity, alignment: .center)
        }
        .padding(.vertical, 12)
        .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
    }

    private func info(state: PackState, status: PackStatusView, hover: Bool) -> some View {
        let details = ConnectorBrowse.details(listing, builtIns: model.builtIns)
        let offers = PluginFacts.offers(listing, details: details)
        return VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Text(listing.name).font(.ui(13.5, weight: .medium)).foregroundStyle(Theme.gray200).lineLimit(1)
                    .underline(hover, color: Theme.white(0.25))
                if let badge = ConnectorBrowse.kindBadge(listing.kind) { PluginBadge(text: badge) }
            }
            Text(offers.isEmpty ? (listing.kind == .extension ? "Adds no surface yet" : "Nothing declared yet") : offers.joined(separator: ", "))
                .font(.ui(12.5)).foregroundStyle(Theme.gray500).padding(.top, 2)
            Text(facts(state)).font(.ui(11)).foregroundStyle(Theme.gray600).padding(.top, 6)
            if let percent = status.percent {
                GeometryReader { geo in
                    Rectangle().fill(Theme.ink).frame(width: geo.size.width * min(max(percent, 0), 100) / 100, height: 1)
                }
                .frame(maxWidth: 220).frame(height: 1).background(Theme.white(0.08)).padding(.top, 8)
            }
            if status.tone != .settled, let detail = status.detail {
                ToneLine(tone: status.tone, text: detail).padding(.top, 6)
            }
            ActivityLine(state: model.rowState(listing.id, [.remove])).padding(.top, 6)
            HStack(spacing: 2) {
                Text("Details")
                LucideIcon(.chevronRight, size: 11)
            }
            .font(.ui(11)).foregroundStyle(hover ? Theme.gray300 : Theme.gray500).padding(.top, 6)
        }
        .multilineTextAlignment(.leading)
    }

    private func facts(_ state: PackState) -> String {
        var parts: [String] = []
        if let pack = listing.pack { parts.append("v\(pack.version)") }
        if case .installed(_, let available?, _) = state { parts.append("v\(available) available") }
        if listing.kind == .extension {
            if let cards = model.activeCards {
                let active = cards[listing.id] ?? 0
                parts.append(active > 0 ? "on \(active) of \(ExtensionCopy.count(max(model.openCards, active), "open card"))" : "on no open card")
            }
        } else if listing.connectedCount > 0 {
            parts.append(ExtensionCopy.count(listing.connectedCount, "connection"))
        } else if listing.implicitlyConnected {
            parts.append("ready")
        } else {
            parts.append("no connection yet")
        }
        return parts.joined(separator: " · ")
    }
}
