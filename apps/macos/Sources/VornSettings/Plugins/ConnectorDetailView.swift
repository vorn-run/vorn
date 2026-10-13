import AppKit
import SwiftUI
import VornCore
import VornUI

/// ConnectorDetail.tsx: everything a listing offers, asks for and has on disk, with its actions.
struct ConnectorDetailView: View {
    @Bindable var model: PluginsModel
    let listing: ConnectorListing
    let backLabel: String

    var body: some View {
        let details = ConnectorBrowse.details(listing, builtIns: model.builtIns)
        let entry = listing.catalogItem
        let state = model.state(listing)
        let status = PackStatusView(state)
        VStack(alignment: .leading, spacing: 0) {
            Hovering { hover in
                Button { model.selectedKey = nil } label: {
                    HStack(spacing: 6) {
                        LucideIcon(.arrowLeft, size: 12)
                        Text(backLabel).font(.ui(12))
                    }
                    .foregroundStyle(hover ? Theme.gray300 : Theme.gray500)
                }
                .buttonStyle(.plain)
            }
            .padding(.bottom, 16)

            header(entry: entry)
            if let description = listing.description, !description.isEmpty {
                Text(description).font(.ui(12.5)).foregroundStyle(Theme.gray400).lineSpacing(3)
                    .fixedSize(horizontal: false, vertical: true).padding(.top, 16)
            }

            if !details.known {
                Text("This connector does not describe itself yet. Add it to see what it offers.")
                    .font(.ui(12)).foregroundStyle(Theme.gray500).padding(.top, 20)
            } else if listing.kind == .extension {
                extensionSections
            } else {
                connectorSections(details)
            }
            signsIn(entry: entry)
            if let pack = listing.pack { machine(pack) }

            if let detail = status.detail, !state.isInstalled {
                ToneLine(tone: status.tone, text: detail, size: 12).padding(.top, 16)
            }
            ActivityLine(state: model.rowState(listing.id, [.rollback, .remove])).padding(.top, 16)
            buttons(state: state, status: status, entry: entry)
            if let pending = model.pending, pending.rowKey == listing.key {
                PendingSheet(model: model).padding(.top, 12)
            }
        }
    }

    private func header(entry: CatalogItem?) -> some View {
        HStack(alignment: .top, spacing: 12) {
            ListingIcon(id: listing.id, icon: entry?.icon ?? listing.icon, packaged: listing.packaged, size: 21, color: Theme.gray200)
                .frame(width: 40, height: 40)
                .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.white(0.05)))
            VStack(alignment: .leading, spacing: 0) {
                Text(listing.name).font(.ui(16, weight: .medium)).tracking(-0.2).foregroundStyle(Theme.ink)
                Text(subline(entry)).font(.ui(11.5)).foregroundStyle(Theme.gray600)
                if let verified = listing.verified {
                    HStack(spacing: 12) {
                        ForEach(verified.checks, id: \.self) { check in
                            HStack(spacing: 4) {
                                LucideIcon(.check, size: 10)
                                Text(check)
                            }
                        }
                        Text("checked \(Self.day(verified.checkedAt)) against v\(verified.version)").foregroundStyle(Theme.gray600)
                    }
                    .font(.ui(11)).foregroundStyle(Theme.gray500).padding(.top, 6)
                }
            }
        }
    }

    private func subline(_ entry: CatalogItem?) -> String {
        var text = entry.map(\.packageName).flatMap { $0.isEmpty ? nil : $0 } ?? (listing.source == .builtin ? "Built in" : listing.id)
        if let version = entry?.version, !version.isEmpty { text += " · v\(version)" }
        if listing.connectedCount > 0 { text += " · \(ExtensionCopy.count(listing.connectedCount, "connection"))" }
        return text
    }

    static func day(_ iso: String) -> String {
        guard let at = PluginsModel.date(iso) ?? ISO8601DateFormatter().date(from: iso + "T00:00:00Z") else { return iso }
        let f = DateFormatter()
        f.setLocalizedDateFormatFromTemplate("d MMM yyyy")
        return f.string(from: at)
    }

    @ViewBuilder
    private func connectorSections(_ details: ConnectorDetails) -> some View {
        DetailSection(label: "Starts a workflow when") {
            if details.triggers.isEmpty {
                DetailNote(text: "Nothing — this one is only called from a step.")
            } else {
                ForEach(details.triggers) { DetailItem(title: $0.label, detail: $0.description) }
            }
        }
        DetailSection(label: "A step can ask it to") {
            if details.actions.isEmpty {
                DetailNote(text: "Nothing — this one only watches.")
            } else {
                ForEach(details.actions) { DetailItem(title: $0.label, detail: $0.description) }
            }
        }
        if !details.settings.isEmpty {
            DetailSection(label: "You will need") {
                details.settings.enumerated().reduce(Text("")) { text, pair in
                    let (index, setting) = pair
                    let name = Text(setting.name + (setting.required ? "" : " (optional)"))
                        .foregroundColor(setting.required ? Theme.gray400 : Theme.gray600)
                    return index == 0 ? text + name : text + Text("   ") + name
                }
                .font(.system(size: 11, design: .monospaced)).lineSpacing(3)
            }
        }
    }

    private typealias Surface = (key: String, title: String, detail: String, when: ExtensionActivation?)

    private static func surfaces(_ contributes: ExtensionContributions?) -> [Surface] {
        var out: [Surface] = []
        for pane in contributes?.panes ?? [] {
            out.append(("pane:\(pane.id)", "\(pane.title) · pane", pane.description ?? ExtensionCopy.describePaneKind(pane), pane.when))
        }
        for footer in contributes?.footers ?? [] {
            let detail = footer.description ?? ExtensionCopy.describeFooterInterval(Int(footer.every))
            out.append(("footer:\(footer.id)", "\(footer.title) · footer", detail, footer.when))
        }
        for link in contributes?.linkHandlers ?? [] {
            out.append(("link:\(link.id)", "\(link.title) · link", link.description ?? "matches \(link.pattern ?? "")", link.when))
        }
        return out
    }

    @ViewBuilder
    private var extensionSections: some View {
        let surfaces = Self.surfaces(listing.contributes)
        let stated = listing.permissions != nil || listing.pack != nil
        let rows = ExtensionCopy.permissionRows(listing.permissions)
        let notAsked = ExtensionCopy.notAsked(listing.permissions)
        DetailSection(label: "Contributes") {
            if surfaces.isEmpty {
                DetailNote(text: "Nothing — this one adds no surface yet.")
            } else {
                ForEach(surfaces, id: \.key) { surface in
                    DetailItem(title: surface.title, detail: surface.detail,
                               when: surface.when.map { "Shows on \(ExtensionCopy.describeActivation($0).joined(separator: " and "))" })
                }
            }
        }
        DetailSection(label: "Asks to") {
            if !stated {
                DetailNote(text: "The catalog does not say what it asks for yet.")
            } else {
                if rows.isEmpty {
                    DetailNote(text: "Nothing. It draws what it is given.")
                } else {
                    ForEach(rows, id: \.label) { row in
                        (Text(row.label.rawValue + " ").foregroundColor(Theme.gray500) + Text(row.items.joined(separator: ", ")))
                            .font(.ui(12.5)).foregroundStyle(Theme.gray300)
                    }
                }
                if !notAsked.isEmpty {
                    DetailNote(text: "Not asked: \(notAsked.joined(separator: ", ")).")
                }
            }
        }
        DetailSection(label: "Shows when") {
            Text(ExtensionCopy.describeActivation(listing.activates).joined(separator: " and ")).font(.ui(12.5))
                .foregroundStyle(Theme.gray300)
        }
    }

    @ViewBuilder
    private func signsIn(entry: CatalogItem?) -> some View {
        let auth = entry?.auth.flatMap { $0.isEmpty ? nil : $0 }
        if listing.authRung != nil || auth != nil || listing.kind == .extension {
            DetailSection(label: "Signs in with") {
                if let rung = listing.authRung {
                    Text(ConnectorBrowse.rung(rung).detail).font(.ui(12.5)).foregroundStyle(Theme.gray300)
                }
                if let origins = listing.pack?.auth, origins.signsInThroughWindow {
                    Text("Acts as you on \(origins.browserOrigins.map(ExtensionCopy.originLabel).joined(separator: ", "))")
                        .font(.ui(12.5)).foregroundStyle(Theme.gray500)
                }
                if let auth { Text(auth).font(.ui(12.5)).foregroundStyle(Theme.gray500) }
                if listing.kind == .extension && listing.authRung == nil && auth == nil {
                    Text("Nothing. It only reads what the session already has.").font(.ui(12.5)).foregroundStyle(Theme.gray300)
                }
            }
        }
    }

    private func machine(_ pack: InstalledPack) -> some View {
        DetailSection(label: "On this machine") {
            VStack(alignment: .leading, spacing: 3) {
                DetailFact(term: "Installed", value: "v\(pack.version)")
                DetailFact(term: "On disk", value: pack.path, mono: true)
                if listing.kind == .extension {
                    if let cards = model.activeCards {
                        let active = cards[listing.id] ?? 0
                        DetailFact(term: "Active", value: "on \(active) of \(ExtensionCopy.count(model.openCards, "open card"))")
                    }
                } else {
                    DetailFact(term: "Runs via", value: "node \(pack.path)/index.js", mono: true)
                }
            }
        }
    }

    private func buttons(state: PackState, status: PackStatusView, entry: CatalogItem?) -> some View {
        let busy = model.rowState(listing.id, [.rollback, .remove]).phrase != nil
        return HStack(spacing: 8) {
            if listing.implicitlyConnected {
                if let open = model.openWorkflowEditor {
                    DetailPrimary(label: "Use in a workflow", icon: .workflow) { open(nil) }
                } else {
                    Text("Ready to use in a workflow.").font(.ui(12)).foregroundStyle(Theme.gray500)
                }
            } else if model.canAdd(listing) {
                DetailPrimary(label: "Add a connection", icon: .plus) { model.adding = listing }
            }
            if listing.source != .builtin && listing.source != .mcp, let action = status.action {
                PluginButton(label: status.label, icon: action == .install ? .download : .refreshCw, busy: status.busy) {
                    Task { await model.inspect(listing, direct: true) }
                }
                .help(state == .absent ? "Install this connector as a file" : "Install the newer version")
            }
            if case .installed(_, _, let previous?) = state {
                PluginButton(label: "Roll back", icon: .undo2, busy: model.isBusy(.rollback, listing.id), disabled: busy) {
                    Task { await model.rollback(listing.id) }
                }
                .help("Go back to v\(previous)")
            }
            if listing.pack != nil {
                PluginButton(label: listing.kind == .extension ? "Uninstall" : "Remove", icon: .trash2,
                             busy: model.isBusy(.remove, listing.id), danger: true, disabled: busy) {
                    Task {
                        await model.removePack(listing.id)
                    }
                }
                .help("Delete the installed files")
            }
            if let package = entry?.packageName, !package.isEmpty, let url = URL(string: "https://www.npmjs.com/package/\(package)") {
                Hovering { hover in
                    Button { NSWorkspace.shared.open(url) } label: {
                        HStack(spacing: 4) {
                            LucideIcon(.externalLink, size: 11)
                            Text("View on npm").font(.ui(12))
                        }
                        .foregroundStyle(hover ? Theme.gray300 : Theme.gray500)
                        .padding(.horizontal, 8).padding(.vertical, 6)
                    }
                    .buttonStyle(.plain)
                }
            }
            if listing.source == .catalog && listing.pack == nil && state == .absent {
                Spacer(minLength: 8)
                Text("Nothing is on disk until you install it.").font(.ui(11)).foregroundStyle(Theme.gray600)
            }
        }
        .padding(.top, 24)
    }
}

/// Section: a small uppercase label over its rows.
struct DetailSection<Content: View>: View {
    let label: String
    @ViewBuilder var content: () -> Content

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(label.uppercased()).font(.ui(10)).tracking(0.5).foregroundStyle(Theme.gray600)
            content()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.top, 20)
    }
}

struct DetailNote: View {
    let text: String
    var body: some View {
        Text(text).font(.ui(12)).foregroundStyle(Theme.gray600).fixedSize(horizontal: false, vertical: true)
    }
}

/// Item: a title, what it does, and when it shows.
struct DetailItem: View {
    let title: String
    var detail: String?
    var when: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).foregroundStyle(Theme.gray300)
            if let detail, !detail.isEmpty { Text(detail).foregroundStyle(Theme.gray600) }
            if let when { Text(when).foregroundStyle(Theme.gray600) }
        }
        .font(.ui(12.5))
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// Fact: a term and its value on one line.
struct DetailFact: View {
    let term: String
    let value: String
    var mono = false

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text(term).font(.ui(12)).foregroundStyle(Theme.gray600).frame(width: 72, alignment: .leading)
            Text(value).font(mono ? .system(size: 11, design: .monospaced) : .ui(12)).foregroundStyle(Theme.gray300)
                .textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// The filled button the detail leads with (`text-xs bg-white/[0.1] px-3 py-1.5`).
struct DetailPrimary: View {
    let label: String
    let icon: LucideGlyph
    let action: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                HStack(spacing: 4) {
                    LucideIcon(icon, size: 12)
                    Text(label).font(.ui(12))
                }
                .foregroundStyle(Theme.ink)
                .padding(.horizontal, 12).padding(.vertical, 6)
                .background(RoundedRectangle(cornerRadius: Theme.radiusSm).fill(Theme.white(hover ? 0.16 : 0.1)))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .fixedSize()
    }
}
