import AppKit
import SwiftUI
import UniformTypeIdentifiers
import VornCore
import VornUI

/// SSHSettings.tsx: remote hosts in the config, keys in vornd's vault.
struct SSHSettings: View {
    @Bindable var model: SettingsModel
    @State private var expandedHosts: Set<String> = []
    @State private var expandedKeys: Set<String> = []
    @State private var passwords: [String: String] = [:]
    @State private var confirmDelete: String?
    @State private var tests: [String: (status: String, message: String?)] = [:]
    @State private var showAddKey = false
    @State private var keyLabel = ""
    @State private var keyPrivate = ""
    @State private var keyPublic = ""
    @State private var keyCert = ""
    @State private var dragOver = false

    static let authMethods = [("agent", "SSH Agent"), ("password", "Password"), ("key-stored", "Stored Key"), ("key-file", "Key File")]

    private var hosts: [[String: JSONValue]] { (model.config?["remoteHosts"]?.arrayValue ?? []).compactMap(\.objectValue) }

    private func setHosts(_ list: [[String: JSONValue]]) { model.updateConfig(["remoteHosts": .array(list.map { .object($0) })]) }

    private func updateHost(_ id: String, _ patch: [String: JSONValue?]) {
        setHosts(hosts.map { h in
            guard h["id"]?.stringValue == id else { return h }
            var n = h
            for (k, v) in patch { n[k] = v }
            return n
        })
    }

    var body: some View {
        if model.config != nil {
            VStack(alignment: .leading, spacing: 0) {
                SettingsPageHeader(title: "SSH & Hosts", description: "Configure remote hosts and SSH keys for running agents on remote machines")
                if !model.safeStorage {
                    Text("Keychain encryption is not available on this system. Install libsecret (gnome-keyring) to use the SSH key vault.")
                        .font(.ui(14)).foregroundStyle(Theme.amber400).lineHeight(20, size: 14)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 16).padding(.vertical, 12)
                        .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Color(red: 245 / 255, green: 158 / 255, blue: 11 / 255, opacity: 0.08)))
                        .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.amber500.opacity(0.3), lineWidth: 1))
                        .padding(.bottom, 24)
                }
                VStack(alignment: .leading, spacing: 0) {
                    GroupHeading(icon: .server, title: "Remote Hosts", count: hosts.count)
                    VStack(spacing: 8) { ForEach(hosts, id: \.["id"]) { hostCard($0) } }
                    DashedAddButton(label: "Add Host", icon: .plus, strokeWidth: 2) {
                        let id = UUID().uuidString.lowercased()
                        setHosts(hosts + [["id": .string(id), "label": "New Host", "hostname": "", "user": "", "port": 22, "authMethod": "agent"]])
                        expandedHosts.insert(id)
                    }
                    .padding(.top, 8)
                }
                .padding(.bottom, 32)
                keysSection
            }
            .task { await model.loadSSHKeys() }
            .onAppear {
                for h in model.renderHints where h.hasPrefix("host:") { expandedHosts.insert(String(h.dropFirst(5))) }
                if model.renderHints.contains("paste") { showAddKey = true }
            }
        }
    }

    // MARK: Hosts

    private func hostCard(_ h: [String: JSONValue]) -> some View {
        let id = h["id"]?.stringValue ?? ""
        let open = expandedHosts.contains(id)
        let user = h["user"]?.stringValue ?? "", hostname = h["hostname"]?.stringValue ?? ""
        let port = Int(h["port"]?.numberValue ?? 22)
        let auth = h["authMethod"]?.stringValue ?? "agent"
        let subtitle = !user.isEmpty && !hostname.isEmpty ? "\(user)@\(hostname)\(port != 22 ? ":\(port)" : "")" : (hostname.isEmpty ? "Not configured" : hostname)
        return Hovering { hover in
            VStack(alignment: .leading, spacing: 0) {
                Button { toggle(&expandedHosts, id) } label: {
                    HStack(spacing: 12) {
                        LucideIcon(.server, size: 18, strokeWidth: 1.5).foregroundStyle(Theme.blue400)
                            .frame(width: 36, height: 36).background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.blue500.opacity(0.1)))
                        VStack(alignment: .leading, spacing: 0) {
                            Text(h["label"]?.stringValue ?? "").font(.ui(14, weight: .medium)).foregroundStyle(Color(hex: 0xF3F4F6))
                            Text(subtitle).font(.system(size: 11, design: .monospaced)).foregroundStyle(Theme.gray500).lineLimit(1)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        LucideIcon(open ? .chevronDown : .chevronRight, size: 14).foregroundStyle(Theme.gray600)
                    }
                    .padding(.horizontal, 16).padding(.vertical, 12).contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                if open { hostForm(h, id: id, auth: auth) }
            }
            .sunkenCard(border: hover ? Theme.white(0.1) : Theme.hairline)
        }
    }

    private func text(_ h: [String: JSONValue], _ key: String, _ id: String) -> Binding<String> {
        Binding(get: { h[key]?.stringValue ?? "" }, set: { updateHost(id, [key: .string($0)]) })
    }

    @ViewBuilder private func hostForm(_ h: [String: JSONValue], id: String, auth: String) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 0) { FieldLabel("Label"); SettingsField(text: text(h, "label", id), placeholder: "Host label") }
            HStack(spacing: 8) {
                VStack(alignment: .leading, spacing: 0) {
                    FieldLabel("Hostname"); SettingsField(text: text(h, "hostname", id), placeholder: "e.g. dev.example.com", mono: true)
                }
                VStack(alignment: .leading, spacing: 0) {
                    FieldLabel("Port")
                    SettingsField(text: Binding(get: { "\(Int(h["port"]?.numberValue ?? 22))" },
                                                set: { updateHost(id, ["port": .number(Double(Int($0) ?? 22))]) }), mono: true)
                }
                .frame(width: 100)
            }
            VStack(alignment: .leading, spacing: 0) { FieldLabel("Username"); SettingsField(text: text(h, "user", id), placeholder: "e.g. ubuntu", mono: true) }
            VStack(alignment: .leading, spacing: 0) {
                FieldLabel("Authentication", bottom: 6)
                HStack(spacing: 2) {
                    ForEach(Self.authMethods, id: \.0) { value, label in
                        Hovering { hover in
                            Button { setAuth(id, value) } label: {
                                Text(label).font(.ui(11, weight: .medium))
                                    .foregroundStyle(auth == value ? .white : hover ? Theme.gray300 : Theme.gray500)
                                    .frame(maxWidth: .infinity).padding(.vertical, 6)
                                    .background(RoundedRectangle(cornerRadius: Theme.radius).fill(auth == value ? Theme.white(0.1) : .clear))
                                    .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
                .padding(2)
                .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.white(0.04)))
            }
            switch auth {
            case "agent":
                Text("Uses your running SSH agent (ssh-agent)").font(.ui(11)).foregroundStyle(Theme.gray600)
            case "password":
                VStack(alignment: .leading, spacing: 0) {
                    FieldLabel("Password")
                    SettingsField(text: Binding(get: { passwords[id] ?? "" }, set: { passwords[id] = $0 }),
                                  placeholder: h["encryptedPassword"] != nil ? "(saved)" : "Enter password", secure: true) {
                        if let pw = passwords[id], !pw.isEmpty {
                            updateHost(id, ["password": .string(pw)])
                            passwords[id] = nil
                        }
                    }
                    Text("Encrypted with your OS keychain").font(.ui(10)).foregroundStyle(Theme.gray600).padding(.top, 4)
                }
            case "key-stored":
                VStack(alignment: .leading, spacing: 0) {
                    FieldLabel("Stored Key")
                    Menu {
                        Button("Select a key...") { updateHost(id, ["credentialId": .string("")]) }
                        ForEach(model.sshKeys) { k in
                            Button(k.label + (k.keyType.map { " (\($0))" } ?? "")) { updateHost(id, ["credentialId": .string(k.id)]) }
                        }
                    } label: {
                        let chosen = model.sshKeys.first { $0.id == h["credentialId"]?.stringValue }
                        HStack {
                            Text(chosen.map { $0.label + ($0.keyType.map { " (\($0))" } ?? "") } ?? "Select a key...").font(.ui(12)).foregroundStyle(Theme.gray200)
                            Spacer()
                            LucideIcon(.chevronDown, size: 12).foregroundStyle(Theme.gray400)
                        }
                        .padding(.horizontal, 10).padding(.vertical, 6)
                        .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Color.black.opacity(0.3)))
                        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.hairline, lineWidth: 1))
                    }
                    .menuStyle(.button).buttonStyle(.plain).menuIndicator(.hidden)
                    if model.sshKeys.isEmpty {
                        Text("No stored keys yet. Add one below.").font(.ui(10)).foregroundStyle(Theme.gray600).padding(.top, 4)
                    }
                }
            default:
                VStack(alignment: .leading, spacing: 0) {
                    FieldLabel("SSH Key Path")
                    HStack(spacing: 6) {
                        SettingsField(text: text(h, "sshKeyPath", id), placeholder: "~/.ssh/id_ed25519", mono: true)
                        IconButton(.key, size: 12, strokeWidth: 1.5, padding: 7, color: Theme.gray400, hoverColor: .white,
                                   hoverFill: Theme.white(0.08)) {
                            if let path = Self.pickFile() { updateHost(id, ["sshKeyPath": .string(path)]) }
                        }
                        .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Color.black.opacity(0.3)))
                        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.hairline, lineWidth: 1))
                    }
                }
            }
            VStack(alignment: .leading, spacing: 0) {
                FieldLabel("Extra SSH Options"); SettingsField(text: text(h, "sshOptions", id), placeholder: "e.g. -o ForwardAgent=yes", mono: true)
            }
            HStack {
                let testing = tests[id]?.status == "testing"
                QuietButton(label: "Test Connection", icon: testing ? .loaderCircle : .wifi, iconSize: 11, fontSize: 12,
                            fill: Theme.white(0.04), color: Theme.gray400, hoverColor: .white, padding: (12, 6), disabled: testing) {
                    test(h, id: id)
                }
                Spacer()
                deleteControl(id: id, label: "Delete Host") {
                    setHosts(hosts.filter { $0["id"]?.stringValue != id })
                    confirmDelete = nil
                }
            }
            .padding(.top, 8)
            if let t = tests[id], t.status != "testing" {
                let ok = t.status == "success"
                HStack(spacing: 6) {
                    LucideIcon(ok ? .circleCheck : .circleX, size: 12)
                    Text(t.message ?? "")
                }
                .font(.ui(11))
                .foregroundStyle(ok ? Theme.green400 : Theme.red400)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 10).padding(.vertical, 6)
                .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill((ok ? Theme.green500 : Theme.red500).opacity(0.1)))
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder((ok ? Theme.green500 : Theme.red500).opacity(0.2), lineWidth: 1))
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 4)
        .padding(.bottom, 16)
    }

    private func setAuth(_ id: String, _ method: String) {
        var patch: [String: JSONValue?] = ["authMethod": .string(method)]
        if method != "key-file" { patch["sshKeyPath"] = .some(nil) }
        if method != "key-stored" { patch["credentialId"] = .some(nil) }
        if method != "password" { patch["encryptedPassword"] = .some(nil); passwords[id] = nil }
        updateHost(id, patch)
    }

    private func test(_ h: [String: JSONValue], id: String) {
        guard !(h["hostname"]?.stringValue ?? "").isEmpty, !(h["user"]?.stringValue ?? "").isEmpty else {
            tests[id] = ("error", "Hostname and username are required")
            return
        }
        tests[id] = ("testing", nil)
        Task {
            let r = await model.testSSH(.object(h))
            tests[id] = (r.ok ? "success" : "error", r.message)
        }
    }

    @ViewBuilder private func deleteControl(id: String, label: String, onConfirm: @escaping () -> Void) -> some View {
        if confirmDelete == id {
            HStack(spacing: 8) {
                QuietButton(label: "Confirm Delete", fontSize: 12, fill: Theme.red400.opacity(0.1), hoverFill: Theme.red400.opacity(0.2),
                            color: Theme.red400, padding: (12, 6), action: onConfirm)
                QuietButton(label: "Cancel", bordered: false, fontSize: 12, fill: .clear, hoverFill: .clear, color: Theme.gray400,
                            hoverColor: .white, padding: (12, 6)) { confirmDelete = nil }
            }
        } else {
            QuietButton(label: label, icon: .trash2, iconSize: 11, fontSize: 12, fill: Theme.white(0.03), hoverFill: Theme.red400.opacity(0.1),
                        color: Theme.gray500, hoverColor: Theme.red400, padding: (12, 6)) { confirmDelete = id }
        }
    }

    // MARK: Keys

    private var keysSection: some View {
        VStack(alignment: .leading, spacing: 0) {
            GroupHeading(icon: .fileKey, title: "SSH Keys", count: model.sshKeys.count)
            if dragOver {
                Text("Drop key file to import").font(.ui(14)).foregroundStyle(Theme.blue400)
                    .frame(maxWidth: .infinity).padding(.vertical, 24)
                    .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.blue400.opacity(0.05)))
                    .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.blue400.opacity(0.4), style: StrokeStyle(lineWidth: 2, dash: [6, 4])))
                    .padding(.bottom, 8)
            }
            VStack(spacing: 8) { ForEach(model.sshKeys) { keyCard($0) } }
            if showAddKey { addKeyForm.padding(.top, 8) }
            HStack(spacing: 8) {
                DashedAddButton(label: "Import Key", icon: .upload, disabled: !model.safeStorage) {
                    if let path = Self.pickFile() { Task { await model.importSSHKeyFile(path) } }
                }
                if !showAddKey { DashedAddButton(label: "Paste Key", icon: .plus, strokeWidth: 2, disabled: !model.safeStorage) { showAddKey = true } }
            }
            .padding(.top, 8)
        }
        .onDrop(of: [.fileURL], isTargeted: Binding(get: { dragOver }, set: { dragOver = $0 && model.safeStorage })) { providers in
            guard model.safeStorage else { return false }
            for p in providers {
                _ = p.loadObject(ofClass: URL.self) { url, _ in
                    if let url { Task { @MainActor in await model.importSSHKeyFile(url.path) } }
                }
            }
            return true
        }
    }

    private func keyCard(_ k: SSHKeyMeta) -> some View {
        let open = expandedKeys.contains(k.id)
        let usedBy = hosts.filter { $0["authMethod"]?.stringValue == "key-stored" && $0["credentialId"]?.stringValue == k.id }
        return Hovering { hover in
            VStack(alignment: .leading, spacing: 0) {
                Button { toggle(&expandedKeys, k.id) } label: {
                    HStack(spacing: 12) {
                        LucideIcon(.fileKey, size: 18, strokeWidth: 1.5).foregroundStyle(Theme.amber400)
                            .frame(width: 36, height: 36).background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.amber500.opacity(0.1)))
                        VStack(alignment: .leading, spacing: 2) {
                            HStack(spacing: 8) {
                                Text(k.label).font(.ui(14, weight: .medium)).foregroundStyle(Color(hex: 0xF3F4F6))
                                if let t = k.keyType {
                                    Text(t.uppercased()).font(.ui(10, weight: .medium)).tracking(0.5).foregroundStyle(Theme.gray400)
                                        .padding(.horizontal, 6).padding(.vertical, 2)
                                        .background(RoundedRectangle(cornerRadius: Theme.radius).fill(Theme.white(0.06)))
                                }
                            }
                            if let pk = k.publicKey {
                                Text(pk.count <= 60 ? pk : String(pk.prefix(30)) + "..." + String(pk.suffix(25)))
                                    .font(.system(size: 11, design: .monospaced)).foregroundStyle(Theme.gray600).lineLimit(1)
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        LucideIcon(open ? .chevronDown : .chevronRight, size: 14).foregroundStyle(Theme.gray600)
                    }
                    .padding(.horizontal, 16).padding(.vertical, 12).contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                if open {
                    VStack(alignment: .leading, spacing: 12) {
                        if let pk = k.publicKey {
                            VStack(alignment: .leading, spacing: 0) {
                                FieldLabel("Public Key")
                                Text(pk).font(.system(size: 11, design: .monospaced)).foregroundStyle(Theme.gray500).lineSpacing(3)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .padding(.horizontal, 10).padding(.vertical, 8)
                                    .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Color.black.opacity(0.3)))
                                    .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.hairline, lineWidth: 1))
                            }
                        }
                        VStack(alignment: .leading, spacing: 0) {
                            FieldLabel("Used By", bottom: 6)
                            if usedBy.isEmpty {
                                Text("No hosts using this key").font(.ui(11)).foregroundStyle(Theme.gray600)
                            } else {
                                HStack(spacing: 6) {
                                    ForEach(usedBy, id: \.["id"]) { host in
                                        HStack(spacing: 6) {
                                            LucideIcon(.server, size: 10, strokeWidth: 1.5).foregroundStyle(Theme.blue400)
                                            Text(host["label"]?.stringValue ?? "").font(.ui(11)).foregroundStyle(Theme.gray300)
                                        }
                                        .padding(.horizontal, 8).padding(.vertical, 4)
                                        .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.white(0.04)))
                                        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.hairline, lineWidth: 1))
                                    }
                                }
                            }
                        }
                        HStack {
                            Spacer()
                            deleteControl(id: k.id, label: "Delete Key") {
                                confirmDelete = nil
                                Task { await model.deleteSSHKey(k.id) }
                            }
                        }
                        .padding(.top, 8)
                    }
                    .padding(.horizontal, 16).padding(.top, 4).padding(.bottom, 16)
                }
            }
            .sunkenCard(border: hover ? Theme.white(0.1) : Theme.hairline)
        }
    }

    private var addKeyForm: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("New Key").font(.ui(14, weight: .medium)).foregroundStyle(Theme.gray200)
            VStack(alignment: .leading, spacing: 0) { FieldLabel("Label"); SettingsField(text: $keyLabel, placeholder: "e.g. Work Server Key") }
            area("Private key *", $keyPrivate, "-----BEGIN OPENSSH PRIVATE KEY-----", rows: 3)
            area("Public key", $keyPublic, "ssh-ed25519 AAAA...", rows: 2)
            area("Certificate", $keyCert, "Optional", rows: 2)
            (Text("If your key has a passphrase, remove it first with ") + Text("ssh-keygen -p").foregroundColor(Theme.gray500))
                .font(.ui(10)).foregroundStyle(Theme.gray600)
            HStack(spacing: 8) {
                let empty = keyPrivate.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                QuietButton(label: "Save Key", bordered: false, fontSize: 12, fill: Color(hex: 0x155DFC), hoverFill: Theme.blue500,
                            color: .white, padding: (16, 6), disabled: empty) {
                    let t = { (s: String) -> String? in let v = s.trimmingCharacters(in: .whitespacesAndNewlines); return v.isEmpty ? nil : v }
                    let label = t(keyLabel) ?? "Untitled Key", priv = t(keyPrivate) ?? "", pub = t(keyPublic), cert = t(keyCert)
                    Task { await model.storeSSHKey(label: label, privateKey: priv, publicKey: pub, certificate: cert) }
                    resetKeyForm()
                }
                QuietButton(label: "Cancel", bordered: false, fontSize: 12, fill: .clear, hoverFill: .clear, color: Theme.gray400,
                            hoverColor: .white, padding: (16, 6)) { resetKeyForm() }
            }
        }
        .padding(16)
        .sunkenCard(border: Theme.white(0.08))
    }

    private func area(_ label: String, _ text: Binding<String>, _ placeholder: String, rows: Int) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            FieldLabel(label)
            TextField("", text: text, prompt: Text(placeholder).foregroundColor(Theme.gray600), axis: .vertical)
                .lineLimit(rows, reservesSpace: true)
                .textFieldStyle(.plain).font(.system(size: 12, design: .monospaced)).foregroundStyle(Theme.gray200)
                .padding(.horizontal, 10).padding(.vertical, 6)
                .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Color.black.opacity(0.3)))
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.hairline, lineWidth: 1))
        }
    }

    private func resetKeyForm() {
        showAddKey = false
        keyLabel = ""; keyPrivate = ""; keyPublic = ""; keyCert = ""
    }

    private func toggle(_ set: inout Set<String>, _ id: String) {
        if set.contains(id) { set.remove(id) } else { set.insert(id) }
    }

    static func pickFile() -> String? {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.showsHiddenFiles = true
        return panel.runModal() == .OK ? panel.url?.path : nil
    }
}
