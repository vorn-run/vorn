import SwiftUI
import VornCore
import VornUI

/// ConnectionGroups.tsx: connections under their connector, then installed connectors with none yet.
struct ConnectionGroupsView: View {
    @Bindable var model: PluginsModel

    var body: some View {
        let listings = model.listings
        let groups = ConnectorBrowse.groups(model.connections, listings: listings)
        let unconnected = listings.filter { $0.pack != nil && $0.kind == .connector && $0.connectedCount == 0 && !$0.implicitlyConnected }
        VStack(alignment: .leading, spacing: 20) {
            ForEach(groups) { group in
                groupView(group)
            }
            if !unconnected.isEmpty {
                VStack(alignment: .leading, spacing: 0) {
                    Text("INSTALLED, NO CONNECTION YET").font(.ui(10)).tracking(0.8).foregroundStyle(Theme.gray600).padding(.bottom, 4)
                    ForEach(unconnected) { listing in
                        HStack(spacing: 12) {
                            ListingIcon(id: listing.id, icon: listing.catalogItem?.icon ?? listing.icon, packaged: true, size: 17,
                                        color: Theme.gray200)
                                .frame(width: 32, height: 32)
                                .background(RoundedRectangle(cornerRadius: Theme.radiusSm).fill(Theme.white(0.04)))
                            VStack(alignment: .leading, spacing: 0) {
                                Text(listing.name).font(.ui(13.5, weight: .medium)).foregroundStyle(Theme.gray200)
                                Text(listing.pack.map { "v\($0.version)" } ?? "").font(.ui(11)).foregroundStyle(Theme.gray600)
                            }
                            Spacer(minLength: 8)
                            PluginButton(label: "Add connection", icon: .plus) { model.adding = listing }
                                .help("Connect this connector to an account")
                        }
                        .padding(.vertical, 10)
                        .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
                    }
                }
            }
        }
    }

    private func groupView(_ group: ConnectionGroup) -> some View {
        let manifest = model.builtIns.first { $0.id == group.connectorId }?.manifest
        let status = model.statuses[group.connectorId].flatMap { $0.authed ? nil : $0 }
        return VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 12) {
                ListingIcon(id: group.connectorId, icon: group.icon, packaged: group.listing?.packaged ?? false, size: 17,
                            color: Theme.gray200)
                    .frame(width: 32, height: 32)
                    .background(RoundedRectangle(cornerRadius: Theme.radiusSm).fill(Theme.white(0.04)))
                VStack(alignment: .leading, spacing: 0) {
                    Text(group.name).font(.ui(13.5, weight: .medium)).foregroundStyle(Theme.gray200)
                    Text(ExtensionCopy.count(group.connections.count, "connection") + (group.version.map { " · v\($0)" } ?? ""))
                        .font(.ui(11)).foregroundStyle(Theme.gray600)
                }
                Spacer(minLength: 8)
                if let listing = group.listing {
                    PluginButton(label: "Add another", icon: .plus) { model.adding = listing }
                }
            }
            if let status {
                HStack(alignment: .top, spacing: 8) {
                    LucideIcon(.triangleAlert, size: 13).foregroundStyle(Theme.amber400).padding(.top, 1)
                    Self.codeText(status.message?.isEmpty == false ? status.message! : "Not signed in.")
                        .font(.ui(11.5)).foregroundStyle(Theme.amber300.opacity(0.9)).lineSpacing(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(.horizontal, 12).padding(.vertical, 8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: Theme.radiusSm).fill(Theme.amber500.opacity(0.04)))
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusSm).strokeBorder(Theme.amber500.opacity(0.3), lineWidth: 1))
            }
            VStack(spacing: 8) {
                ForEach(group.connections) { conn in
                    ConnectionRowView(model: model, conn: conn, manifest: manifest,
                                      browserSignIn: group.listing?.pack?.auth.flatMap { $0.signsInThroughWindow ? $0 : nil })
                }
            }
        }
    }

    /// renderMessageWithCode: backticked spans in mono.
    static func codeText(_ text: String) -> Text {
        text.components(separatedBy: "`").enumerated().reduce(Text("")) { result, pair in
            let (index, part) = pair
            guard index % 2 == 1, !part.isEmpty else { return result + Text(part) }
            return result + Text(part).font(.system(size: 11, design: .monospaced)).foregroundColor(Theme.amber300)
        }
    }
}

/// ConnectionRow.tsx: one connection, its seeded workflows, and what it last did.
struct ConnectionRowView: View {
    @Bindable var model: PluginsModel
    let conn: SourceConnection
    let manifest: ConnectorManifest?
    let browserSignIn: SdkAuth?

    var body: some View {
        let seeded = SeededWorkflow.seeded(model.workflows, for: conn.id)
        let missing = SeededWorkflow.missing(model.workflows, for: conn.id, manifest: manifest)
        VStack(alignment: .leading, spacing: 0) {
            header
            if browserSignIn != nil { signIn.padding(.top, 4) }
            if let test = model.tests[conn.id] {
                Text(test.message?.isEmpty == false ? test.message! : test.ok == true ? "Reachable" : "Failed")
                    .font(.ui(11)).foregroundStyle(test.ok == true ? Theme.green400 : Theme.red400).padding(.top, 4)
            }
            if !seeded.isEmpty || !missing.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(seeded) { wf in workflowRow(wf) }
                    ForEach(seeded) { wf in
                        if let error = model.rowState(wf.id, [.run]).error { ToneLine(tone: .broken, text: error) }
                    }
                    ForEach(missing, id: \.event) { event in
                        HStack {
                            Text("No workflow for \(event.name) — polling disabled").italic().foregroundStyle(Theme.gray600)
                            Spacer()
                            Hovering { hover in
                                Button {
                                    Task { await model.resetWorkflow(connectionId: conn.id, event: event.event) }
                                } label: {
                                    Text("Reset default workflow").font(.ui(10)).foregroundStyle(hover ? Theme.gray200 : Theme.gray400)
                                        .padding(.horizontal, 8).padding(.vertical, 2)
                                        .overlay(RoundedRectangle(cornerRadius: Theme.radiusSm).strokeBorder(Theme.white(0.1), lineWidth: 1))
                                }
                                .buttonStyle(.plain)
                            }
                            .help("Re-seed the default workflow for this event (same as when you first connected)")
                        }
                        .font(.ui(11))
                    }
                }
                .padding(.top, 6)
            }
            footer.padding(.top, 4)
            chips
        }
        .padding(.horizontal, 16).padding(.vertical, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: Theme.radiusSm).fill(Theme.white(0.03)))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusSm).strokeBorder(Theme.hairline, lineWidth: 1))
    }

    private var header: some View {
        let backfilling = model.isBusy(.backfill, conn.id)
        let deleting = model.isBusy(.delete, conn.id)
        return HStack(spacing: 12) {
            ConnectorIcon(connectorId: conn.connectorId, viewBox: conn.icon?.viewBox, paths: conn.icon?.paths,
                          packaged: conn.connectorId == "sdk", size: 14, color: Theme.gray200)
                .frame(width: 24, height: 24)
                .background(RoundedRectangle(cornerRadius: Theme.radiusSm).fill(Theme.white(0.04)))
            Text(conn.name).font(.ui(13, weight: .medium)).foregroundStyle(Theme.gray200).lineLimit(1)
            Spacer(minLength: 8)
            HStack(spacing: 2) {
                RowIconButton(icon: .importItems, busy: backfilling,
                              help: "Import existing items matching this connection's filters. Bypasses the cron cursor.") {
                    Task { await model.backfill(conn.id) }
                }
                if conn.connectorId == "http" {
                    RowIconButton(icon: .activity, busy: model.testing.contains(conn.id),
                                  help: "Send a request through this profile now and report the status") {
                        Task { await model.test(conn.id) }
                    }
                }
                RowIconButton(icon: .trash2, busy: deleting, disabled: conn.isImplicit,
                              help: conn.isImplicit ? "This connection came with its connector. Remove the pack instead."
                                  : "Remove this connection (seeded workflows are also deleted)") {
                    Task { await model.deleteConnection(conn.id) }
                }
            }
        }
    }

    private var signIn: some View {
        let signed = conn.signedInAt != nil
        return HStack(spacing: 8) {
            Circle().fill(signed ? Theme.statusSage : Theme.inkGhost).frame(width: 6, height: 6)
            Text(signed ? "Signed in" + (conn.signedInAs.map { " as \($0)" } ?? "") : "Not signed in")
                .foregroundStyle(signed ? Theme.gray300 : Theme.gray500)
            // Signing in opens a browser profile the desktop shell owns; this build has no such window yet.
            Text("· sign in from the Vorn desktop app").foregroundStyle(Theme.gray600)
        }
        .font(.ui(11))
    }

    private func workflowRow(_ wf: SeededWorkflow) -> some View {
        let polling = model.isBusy(.run, wf.id)
        return HStack {
            Hovering { hover in
                Button {
                    model.openWorkflowEditor?(wf.id)
                } label: {
                    HStack(spacing: 6) {
                        LucideIcon(.workflow, size: 12, strokeWidth: 1.75).foregroundStyle(Theme.gray500)
                        Text(wf.name).foregroundStyle(hover ? .white : Theme.gray400).underline(hover, color: Theme.white(0.3))
                        Text("· \(ExtensionCopy.humanCron(wf.cron))").foregroundStyle(Theme.gray600)
                    }
                }
                .buttonStyle(.plain)
            }
            .help("Open this workflow in the editor to customize the schedule, filters, or add steps")
            Spacer()
            RowIconButton(icon: .play, size: 11, busy: polling, help: "Poll the connector now instead of waiting for the next cron tick") {
                Task { await model.run(wf.id, connectionId: conn.id) }
            }
        }
        .font(.ui(11))
    }

    private var footer: some View {
        let reporting = model.rowState(conn.id, [.backfill, .delete, .signIn, .signOut])
        let result = model.backfills[conn.id]
        return FlowLayout(spacing: 8) {
            ActivityLine(state: reporting)
            if let at = conn.lastSyncAt {
                Text("Last synced \(Self.local(at))").foregroundStyle(Theme.gray600)
            }
            if let error = conn.lastSyncError, !error.isEmpty {
                HStack(spacing: 4) {
                    LucideIcon(.circleAlert, size: 10)
                    Text(error)
                }
                .foregroundStyle(Theme.red400)
            }
            if let result {
                if let error = result.error {
                    HStack(spacing: 4) {
                        LucideIcon(.circleAlert, size: 10)
                        Text(error)
                    }
                    .foregroundStyle(Theme.red400)
                } else {
                    Text("+\(result.imported) imported" + (result.updated > 0 ? ", \(result.updated) updated" : ""))
                        .foregroundStyle(Theme.green400)
                }
            }
        }
        .font(.ui(11))
    }

    @ViewBuilder
    private var chips: some View {
        let entries = conn.filters.filter { $0.key != "discoveredTools" }.sorted { $0.key < $1.key }
        if !entries.isEmpty {
            FlowLayout(spacing: 4) {
                ForEach(entries, id: \.key) { key, value in
                    Text("\(key): \(display(key, value))").font(.ui(10)).foregroundStyle(Theme.gray400).lineLimit(1)
                        .padding(.horizontal, 6).padding(.vertical, 2)
                        .background(RoundedRectangle(cornerRadius: Theme.radiusSm).fill(Theme.white(0.04)))
                }
            }
            .padding(.top, 6)
        }
    }

    private func display(_ key: String, _ value: JSONValue) -> String {
        if (manifest?.auth ?? []).contains(where: { $0.key == key && $0.type == "password" }) { return "••••••" }
        let text: String = switch value {
        case .string(let s): s
        case .bool(let b): b ? "true" : "false"
        case .number(let n): n.rounded() == n && abs(n) < 1e15 ? String(Int(n)) : String(n)
        case .null: "null"
        default: value.compactJSON
        }
        if case .string = value, text.count > 60 { return String(text.prefix(57)) + "…" }
        return text
    }

    static func local(_ iso: String) -> String {
        guard let date = PluginsModel.date(iso) else { return iso }
        return date.formatted(date: .numeric, time: .standard)
    }
}

/// The row's small icon buttons (`p-1 text-gray-500 hover:text-gray-200`).
struct RowIconButton: View {
    let icon: LucideGlyph
    var size: CGFloat = 13
    var busy = false
    var disabled = false
    let help: String
    let action: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                BusyIcon(busy: busy, icon: icon, size: size)
                    .foregroundStyle(hover && !disabled && !busy ? Theme.gray200 : Theme.gray500)
                    .padding(4)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(disabled || busy)
            .opacity(disabled ? 0.4 : busy ? 0.5 : 1)
        }
        .help(help)
    }
}
