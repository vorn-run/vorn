import SwiftUI
import VornCore
import VornUI

/// PackInstallConfirm.tsx: what a staged pack will add and ask for, before it is kept.
struct PackInstallConfirm: View {
    let preview: PackPreview
    var busy = false
    let onConfirm: () -> Void
    let onCancel: () -> Void

    private var replacing: Bool {
        guard let installed = preview.installedVersion, !installed.isEmpty else { return false }
        return installed != preview.version
    }

    private var rows: [(label: String, items: [String])] {
        let required = preview.env.filter(\.required).map(\.name)
        let borrows = ExtensionCopy.borrowable(auth: preview.auth, env: preview.env).map {
            "\($0) from \(preview.auth?.probeCommand ?? "a signed-in tool")"
        }
        let contributes = preview.contributes
        let adds = (contributes?.panes ?? []).map { "\($0.title) pane" }
            + (contributes?.footers ?? []).map { "\($0.title) footer" }
            + (contributes?.linkHandlers ?? []).map { "\($0.title) link" }
        var rows: [(String, [String])] = [
            ("Triggers", preview.triggers.map { $0.label.isEmpty ? $0.type : $0.label }),
            ("Actions", preview.actions.map { $0.label.isEmpty ? $0.type : $0.label }),
            ("Needs", required),
            ("Borrows", borrows),
            ("Adds", adds),
        ]
        rows += ExtensionCopy.permissionRows(preview.permissions).map { ($0.label.rawValue, $0.items) }
        return rows.filter { !$0.1.isEmpty }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .top, spacing: 10) {
                ListingIcon(id: preview.id, icon: preview.icon, packaged: true, size: 20, color: Theme.gray300).padding(.top, 2)
                VStack(alignment: .leading, spacing: 2) {
                    (Text(preview.name).foregroundStyle(Theme.gray200).font(.ui(13, weight: .medium))
                        + Text(" v\(preview.version)").foregroundStyle(Theme.gray500).font(.ui(13)))
                    if let description = preview.description, !description.isEmpty {
                        Text(description).font(.ui(11)).foregroundStyle(Theme.gray500).fixedSize(horizontal: false, vertical: true)
                    }
                    if replacing, let installed = preview.installedVersion {
                        Text("Replaces v\(installed), which is kept so you can roll back.").font(.ui(11)).foregroundStyle(Theme.gray400)
                            .padding(.top, 2)
                    }
                }
            }
            VStack(alignment: .leading, spacing: 6) {
                ForEach(rows, id: \.label) { row in
                    HStack(alignment: .top, spacing: 8) {
                        Text(row.label).foregroundStyle(Theme.gray600).frame(width: 56, alignment: .leading)
                        Text(row.items.joined(separator: ", ")).foregroundStyle(Theme.gray400)
                            .frame(maxWidth: .infinity, alignment: .leading).fixedSize(horizontal: false, vertical: true)
                    }
                    .font(.ui(11))
                }
            }
            .padding(.top, 12)
            HStack(spacing: 8) {
                Spacer()
                PluginButton(label: "Cancel", disabled: busy, action: onCancel)
                PluginButton(label: busy ? "Installing…" : replacing ? "Update" : "Install", strong: true, disabled: busy,
                             action: onConfirm)
            }
            .padding(.top, 12)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: Theme.radiusSm).fill(Theme.white(0.02)))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusSm).strokeBorder(Theme.white(0.1), lineWidth: 1))
    }
}
