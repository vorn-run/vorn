import SwiftUI
import VornCore
import VornUI

/// SdkConnectorForm.tsx: reads a package's manifest by starting it, then connects it.
struct SdkConnectorForm: View {
    @Bindable var model: PluginsModel
    var catalogEntry: CatalogItem?
    var pack: InstalledPack?
    let onDone: () async -> Void
    let onCancel: () -> Void

    @State private var spec = ""
    @State private var probing = false
    @State private var manifest: SdkManifest?
    @State private var launch: LaunchSpec?
    @State private var triggerType = ""
    @State private var values: [String: String] = [:]
    @State private var project = ""
    @State private var saving = false
    @State private var error: String?
    @State private var identity: AuthProbeReport?
    @State private var useToken = false
    @State private var probedKey: String?

    private var rung: ConnectorAuthRung? { manifest?.auth?.rung }
    private var borrowing: Bool { rung == .cli && !useToken }
    private var trigger: SdkTrigger? { manifest?.triggers.first { $0.type == triggerType } }
    private var borrowed: [String] {
        guard borrowing, let manifest else { return [] }
        return ExtensionCopy.borrowable(auth: manifest.auth, env: manifest.env)
    }
    private var fields: [ConnectorEnvVar] {
        let covered = Set(borrowed.map { $0.uppercased() })
        return (manifest?.env ?? []).filter { rung != ConnectorAuthRung.none && !($0.secret && covered.contains($0.name.uppercased())) }
    }
    private var missing: Bool { fields.contains { $0.required && (values[$0.name] ?? "").trimmed.isEmpty } }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if catalogEntry == nil { lookup }
            if probing {
                Text("Downloading and starting the connector to read what it needs. This can take a moment the first time.")
                    .font(.ui(11)).foregroundStyle(Theme.gray500).fixedSize(horizontal: false, vertical: true)
            }
            if let manifest { found(manifest) }
            if let error { FormError(text: error) }
            FormButtons(label: buttonLabel, busy: saving, disabled: manifest == nil || saving || (rung != ConnectorAuthRung.none && missing),
                        onSave: {
                            Task {
                                if rung == ConnectorAuthRung.none { await onDone() } else { await save() }
                            }
                        },
                        onCancel: onCancel)
        }
        .task(id: autoKey) { await autoProbe() }
        .task(id: manifest?.id) { await probeIdentity() }
        .onAppear { if project.isEmpty { project = model.projects.first?.name ?? "" } }
    }

    private var buttonLabel: String {
        if rung == ConnectorAuthRung.none { return "Done" }
        if saving { return "Connecting…" }
        return rung == .browser ? "Connect and sign in" : "Connect"
    }

    private var lookup: some View {
        VStack(alignment: .leading, spacing: 0) {
            PluginFieldLabel(text: "Connector package")
            HStack(spacing: 8) {
                PluginInput(text: $spec, placeholder: "@vornrun/connector-kusto") {
                    if !spec.trimmed.isEmpty && !probing { Task { await probe(LaunchSpec(parsing: spec)) } }
                }
                PluginButton(label: probing ? "Reading…" : "Look up", icon: .search, busy: probing, strong: true,
                             disabled: probing || spec.trimmed.isEmpty) {
                    Task { await probe(LaunchSpec(parsing: spec)) }
                }
            }
            Text("An npm package name, or a command to run a connector from a local checkout.")
                .font(.ui(11)).foregroundStyle(Theme.gray600).padding(.top, 4)
        }
    }

    @ViewBuilder
    private func found(_ manifest: SdkManifest) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                if let icon = manifest.icon {
                    ConnectorIcon(connectorId: "sdk", viewBox: icon.viewBox, paths: icon.paths, packaged: true, size: 13, color: Theme.gray200)
                } else {
                    LucideIcon(.check, size: 12).foregroundStyle(Theme.green400)
                }
                Text(manifest.name).font(.ui(13, weight: .medium)).foregroundStyle(Theme.gray200)
                Text("v\(manifest.version)").font(.ui(11)).foregroundStyle(Theme.gray500)
            }
            if let description = manifest.description, !description.isEmpty {
                Text(description).font(.ui(11)).foregroundStyle(Theme.gray500).fixedSize(horizontal: false, vertical: true)
            }
            if !manifest.actions.isEmpty {
                Text("\(ExtensionCopy.count(manifest.actions.count, "action")) available to workflow steps.")
                    .font(.ui(11)).foregroundStyle(Theme.gray600)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .pluginBox()

        if rung == ConnectorAuthRung.none {
            Text("This connector asks for no sign-in — installing it was the whole setup, and its actions are ready to use in a workflow.")
                .font(.ui(11)).foregroundStyle(Theme.gray500).fixedSize(horizontal: false, vertical: true)
        } else {
            if !manifest.triggers.isEmpty {
                VStack(alignment: .leading, spacing: 0) {
                    PluginFieldLabel(text: "Trigger")
                    PluginSelect(options: manifest.triggers.map { ($0.type, $0.label) }, value: triggerType, fill: true) { triggerType = $0 }
                    if let description = trigger?.description {
                        Text(description).font(.ui(11)).foregroundStyle(Theme.gray600).padding(.top, 4)
                    }
                }
            }
            VStack(alignment: .leading, spacing: 0) {
                PluginFieldLabel(text: "Project")
                PluginSelect(options: model.projects.map { ($0.name, $0.name) }, value: project, fill: true) { project = $0 }
            }
        }
        if rung == .cli { cliBox(manifest) }
        if rung == .browser, let url = manifest.auth?.browserSignInUrl { browserBox(manifest, url: url) }
        ForEach(fields) { entry in
            VStack(alignment: .leading, spacing: 0) {
                PluginFieldLabel(text: entry.name, mono: true, required: entry.required, secret: entry.secret)
                PluginInput(text: Binding(get: { values[entry.name] ?? "" }, set: { values[entry.name] = $0 }), secure: entry.secret)
                if let description = entry.description, !description.isEmpty {
                    Text(description).font(.ui(11)).foregroundStyle(Theme.gray600).padding(.top, 4)
                }
            }
        }
    }

    private func cliBox(_ manifest: SdkManifest) -> some View {
        let command = manifest.auth?.probeCommand
        return VStack(alignment: .leading, spacing: 4) {
            if let identity, identity.ok == true {
                HStack(spacing: 6) {
                    Circle().fill(Theme.statusSage).frame(width: 6, height: 6)
                    Text("Signed in" + (identity.identity.map { " as \($0)" } ?? "")).font(.ui(13)).foregroundStyle(Theme.gray200)
                    if let command {
                        Text("· from \(([command] + (manifest.auth?.probeArgs ?? [])).joined(separator: " "))")
                            .font(.system(size: 11, design: .monospaced)).foregroundStyle(Theme.gray500)
                    }
                }
            } else if let identity {
                Text(identity.message ?? "Nothing to check for this connector.").font(.ui(11)).foregroundStyle(Theme.gray400)
                if let hint = identity.installHint { Text(hint).font(.ui(11)).foregroundStyle(Theme.gray600) }
            } else {
                HStack(spacing: 6) {
                    Spinner(size: 11)
                    Text("Checking whether \(command ?? "the tool it borrows") is signed in…")
                }
                .font(.ui(11)).foregroundStyle(Theme.gray400)
            }
            if !borrowed.isEmpty {
                Text("Hands over \(borrowed.joined(separator: ", ")) from \(command ?? "").").font(.ui(11)).foregroundStyle(Theme.gray600)
            }
            PluginLink(label: useToken ? "Borrow the signed-in tool instead" : "Use a token instead") {
                // A field about to be hidden takes what was typed into it along.
                let hidden = Set(ExtensionCopy.borrowable(auth: manifest.auth, env: manifest.env).map { $0.uppercased() })
                for entry in manifest.env where entry.secret && hidden.contains(entry.name.uppercased()) {
                    values[entry.name] = nil
                }
                useToken.toggle()
            }
            .padding(.top, 2)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .pluginBox()
    }

    private func browserBox(_ manifest: SdkManifest, url: String) -> some View {
        let host = URL(string: url)?.host ?? url
        let origins = (manifest.auth?.browserOrigins ?? []).map(ExtensionCopy.originLabel).joined(separator: ", ")
        return VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Circle().fill(Theme.inkGhost).frame(width: 6, height: 6)
                Text("Signs in through a Vorn window").font(.ui(13)).foregroundStyle(Theme.gray200)
            }
            Text("After Connect, a window opens on \(host). The login stays in a browser profile only this connection uses, and it acts as you on \(origins).")
                .font(.ui(11)).foregroundStyle(Theme.gray600).fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .pluginBox()
    }

    // MARK: Actions

    private var autoKey: String? {
        if let pack { return "\(pack.id)@\(pack.version)" }
        return catalogEntry?.packageName
    }

    /// The installed pack wins: it is the code that will run, and needs no registry.
    private func autoProbe() async {
        guard let key = autoKey, probedKey != key, let target = pack?.launch ?? catalogEntry?.launch else { return }
        probedKey = key
        await probe(target)
    }

    private func probe(_ target: LaunchSpec) async {
        error = nil
        manifest = nil
        probing = true
        defer { probing = false }
        switch await model.api.probeSdk(target) {
        case .ok(let found):
            manifest = found
            launch = target
            triggerType = found.triggers.first?.type ?? ""
            values = Dictionary(found.env.map { ($0.name, "") }, uniquingKeysWith: { first, _ in first })
        case .failed(let message):
            error = message
        }
    }

    private func probeIdentity() async {
        guard let manifest, manifest.auth?.rung == .cli else { return }
        identity = nil
        identity = await model.api.probeAuth(manifest.id)
    }

    private func save() async {
        guard let manifest, let launch else { return }
        error = nil
        saving = true
        defer { saving = false }
        var plain: [String: String] = [:]
        var secret: [String: String] = [:]
        for entry in fields {
            let value = (values[entry.name] ?? "").trimmed
            guard !value.isEmpty else { continue }
            if entry.secret { secret[entry.name] = value } else { plain[entry.name] = value }
        }
        var filters: [String: JSONValue] = [
            "command": .string(launch.command),
            "args": .string(JSONValue.array(launch.args.map { .string($0) }).compactJSON),
            "env": .string(JSONValue.object(plain.mapValues { .string($0) }).compactJSON),
        ]
        filters.merge(ExtensionCopy.sdkFilters(id: manifest.id, version: manifest.version, icon: manifest.icon, trigger: trigger?.type)) { _, new in new }
        // vornd keeps it in the keychain; the row never holds it.
        if !secret.isEmpty { filters["secretEnv"] = .string(JSONValue.object(secret.mapValues { .string($0) }).compactJSON) }
        do {
            try await model.api.createConnection(connectorId: "sdk", name: trigger.map { "\(manifest.name): \($0.label)" } ?? manifest.name,
                                                 filters: filters, statusMapping: trigger?.statusMapping ?? [:],
                                                 executionProject: project, seedWorkflow: trigger?.defaultWorkflow)
            await onDone()
        } catch {
            self.error = ConnectorsAPI.describe(error, fallback: "The connection could not be made")
        }
    }
}
