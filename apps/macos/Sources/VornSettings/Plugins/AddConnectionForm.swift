import SwiftUI
import VornCore
import VornUI

/// AddConnectionForm.tsx: connects a built-in connector (or an MCP server) to an account and a project.
struct AddConnectionForm: View {
    @Bindable var model: PluginsModel
    let connector: BuiltInConnector
    var initialAuth: [String: String] = [:]
    var extraFilters: [String: String] = [:]
    var startManual = false

    @State private var fromPackage = true
    @State private var project = ""
    @State private var detected: ConnectorsAPI.Repo?
    @State private var detecting = false
    @State private var auth: [String: String] = [:]
    @State private var filters: [String: String] = [:]
    @State private var owner = ""
    @State private var repo = ""
    @State private var saving = false
    @State private var error: String?
    @State private var started = false

    private var manifest: ConnectorManifest { connector.manifest ?? ConnectorManifest() }
    private var isMcp: Bool { connector.id == "mcp" }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 8) {
                ConnectorIcon(connectorId: connector.id, size: 14, color: Theme.gray400)
                Text("Connect \(connector.name)").font(.ui(13, weight: .medium)).foregroundStyle(Theme.gray200)
            }
            if isMcp {
                HStack(spacing: 4) {
                    tab("From a package", true)
                    tab("Manual setup", false)
                }
            }
            if isMcp && fromPackage {
                SdkConnectorForm(model: model, onDone: done, onCancel: cancel)
            } else {
                form
            }
        }
        .padding(.horizontal, 16).padding(.vertical, 12)
        .pluginBox(padding: 0)
        .padding(.bottom, 24)
        .onAppear {
            guard !started else { return }
            started = true
            fromPackage = isMcp && !startManual
            auth = initialAuth
            project = model.projects.first?.name ?? ""
        }
        .task(id: project) { await detect() }
    }

    private func tab(_ label: String, _ value: Bool) -> some View {
        Hovering { hover in
            Button { fromPackage = value } label: {
                Text(label).font(.ui(12))
                    .foregroundStyle(fromPackage == value ? Theme.gray200 : hover ? Theme.gray300 : Theme.gray500)
                    .padding(.horizontal, 12).padding(.vertical, 4)
                    .background(RoundedRectangle(cornerRadius: Theme.radiusSm).fill(fromPackage == value ? Theme.white(0.1) : .clear))
            }
            .buttonStyle(.plain)
        }
    }

    private var form: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 0) {
                PluginFieldLabel(text: "Project")
                PluginSelect(options: model.projects.map { ($0.name, $0.name) }, value: project, fill: true) { project = $0 }
            }
            if manifest.detectRepo { repoFields }
            ForEach(manifest.auth) { field in
                DynamicField(field: field, value: binding($auth, field.key))
            }
            ForEach(manifest.taskFilters) { field in
                DynamicField(field: field, value: binding($filters, field.key))
            }
            if let error { FormError(text: error) }
            FormButtons(label: saving ? "Connecting…" : "Connect", busy: saving, disabled: !canSave,
                        onSave: { Task { await save() } }, onCancel: cancel)
        }
    }

    @ViewBuilder
    private var repoFields: some View {
        Group {
            if detecting {
                Text("Detecting repository...").foregroundStyle(Theme.gray500)
            } else if let detected {
                HStack(spacing: 4) {
                    LucideIcon(.check, size: 12)
                    Text("Detected: \(detected.owner)/\(detected.repo)")
                }
                .foregroundStyle(Theme.green400)
            } else if !project.isEmpty {
                Text("No GitHub repo detected. Enter the repo manually below.").foregroundStyle(Theme.amber400)
            }
        }
        .font(.ui(12))
        if !detecting && detected == nil {
            HStack(spacing: 8) {
                VStack(alignment: .leading, spacing: 0) {
                    PluginFieldLabel(text: "Owner")
                    PluginInput(text: $owner, placeholder: "e.g. octocat")
                }
                VStack(alignment: .leading, spacing: 0) {
                    PluginFieldLabel(text: "Repository")
                    PluginInput(text: $repo, placeholder: "e.g. hello-world")
                }
            }
        }
    }

    private func binding(_ dict: Binding<[String: String]>, _ key: String) -> Binding<String> {
        Binding(get: { dict.wrappedValue[key] ?? "" }, set: { dict.wrappedValue[key] = $0 })
    }

    private var canSave: Bool {
        if manifest.detectRepo {
            return detected != nil || (!owner.trimmed.isEmpty && !repo.trimmed.isEmpty)
        }
        return !manifest.auth.contains { $0.required && (auth[$0.key] ?? "").trimmed.isEmpty }
    }

    private func detect() async {
        guard manifest.detectRepo, let path = model.projects.first(where: { $0.name == project })?.path else { return }
        detecting = true
        detected = nil
        detected = try? await model.api.detectRepo(path)
        detecting = false
    }

    private func save() async {
        error = nil
        saving = true
        defer { saving = false }
        var values: [String: JSONValue] = [:]
        for field in manifest.auth { if let v = auth[field.key], !v.isEmpty { values[field.key] = .string(v) } }
        for (k, v) in filters { values[k] = .string(v) }
        for (k, v) in extraFilters { values[k] = .string(v) }
        let name: String
        if manifest.detectRepo {
            let o = detected?.owner ?? owner.trimmed
            let r = detected?.repo ?? repo.trimmed
            values["owner"] = .string(o)
            values["repo"] = .string(r)
            name = "\(o)/\(r)"
        } else if let profile = auth["profileName"]?.trimmed, !profile.isEmpty {
            name = profile
        } else if let team = filters["teamKey"], !team.isEmpty {
            name = "\(connector.name): \(team)"
        } else {
            name = connector.name + (project.isEmpty ? "" : " · \(project)")
        }
        let mapping = Dictionary(manifest.statusMapping.map { ($0.upstream, $0.local) }, uniquingKeysWith: { _, last in last })
        do {
            try await model.api.createConnection(connectorId: connector.id, name: name, filters: values, statusMapping: mapping,
                                                 executionProject: project)
            await done()
        } catch {
            self.error = ConnectorsAPI.describe(error, fallback: "The connection could not be made")
        }
    }

    private func done() async { await model.finishAdding() }
    private func cancel() { model.adding = nil }
}

/// DynamicField.tsx: one manifest field as a text box, a secret box, a textarea or a select.
struct DynamicField: View {
    let field: ConnectorField
    @Binding var value: String

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            PluginFieldLabel(text: field.label, required: field.required, secret: field.type == "password")
            switch field.type {
            case "select":
                PluginSelect(options: [("", "—")] + field.options.map { ($0.value, $0.label) }, value: value, fill: true) { value = $0 }
            case "textarea":
                TextEditor(text: $value).font(.ui(13)).foregroundStyle(Theme.gray200).scrollContentBackground(.hidden)
                    .frame(height: 60).padding(.horizontal, 8).padding(.vertical, 4)
                    .background(RoundedRectangle(cornerRadius: Theme.radiusSm).fill(Theme.white(0.05)))
                    .overlay(RoundedRectangle(cornerRadius: Theme.radiusSm).strokeBorder(Theme.white(0.1), lineWidth: 1))
            default:
                PluginInput(text: $value, placeholder: field.placeholder ?? "", secure: field.type == "password")
            }
            if let description = field.description, !description.isEmpty {
                Text(description).font(.ui(10)).foregroundStyle(Theme.gray600).padding(.top, 2)
            }
        }
    }
}

/// The plugin forms' input (`bg-white/[0.05] border-white/[0.1] rounded-sm text-sm`).
struct PluginInput: View {
    @Binding var text: String
    var placeholder = ""
    var secure = false
    var mono = false
    var onSubmit: () -> Void = {}
    @FocusState private var focused: Bool

    var body: some View {
        ZStack(alignment: .leading) {
            if text.isEmpty {
                Text(placeholder).foregroundStyle(Theme.gray600).lineLimit(1).allowsHitTesting(false)
            }
            if secure {
                SecureField("", text: $text)
            } else {
                TextField("", text: $text)
            }
        }
        .textFieldStyle(.plain)
        .font(mono ? .system(size: 12, design: .monospaced) : .ui(13))
        .foregroundStyle(Theme.gray200)
        .focused($focused)
        .onSubmit(onSubmit)
        .padding(.horizontal, 12).padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: Theme.radiusSm).fill(Theme.white(0.05)))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusSm).strokeBorder(Theme.white(focused ? 0.2 : 0.1), lineWidth: 1))
    }
}

extension String {
    var trimmed: String { trimmingCharacters(in: .whitespacesAndNewlines) }
}
