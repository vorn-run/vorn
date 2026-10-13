import AppKit
import SwiftUI
import VornCore
import VornUI

/// McpSettings.tsx: per-agent commands that install the Vorn plugin.
struct McpSettings: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Skills & MCP").font(.ui(20, weight: .semibold)).foregroundStyle(.white).padding(.bottom, 4)
            Text("Connect your coding agents to Vorn. Each setup below installs the MCP server along with the guidance that explains what its tools are for.")
                .font(.ui(14)).foregroundStyle(Theme.gray500).fixedSize(horizontal: false, vertical: true).padding(.bottom, 24)
            Text("Set up an agent").font(.ui(14, weight: .medium)).foregroundStyle(Theme.gray200).padding(.bottom, 12)
            VStack(spacing: 12) {
                ForEach(AgentMcpSetup.all, id: \.agent.rawValue) { setup in
                    VStack(alignment: .leading, spacing: 0) {
                        HStack(spacing: 12) {
                            AgentIcon(setup.agent, size: 20)
                            Text(setup.agent.displayName).font(.ui(14, weight: .medium)).foregroundStyle(Theme.gray200)
                            Text(setup.inAgent ? "Run in \(setup.agent.displayName)" : "Run in your terminal").font(.ui(11)).foregroundStyle(Theme.gray500)
                        }
                        .padding(.bottom, 12)
                        VStack(spacing: 8) {
                            ForEach(setup.commands, id: \.self) { command in
                                HStack(spacing: 8) {
                                    ScrollView(.horizontal) {
                                        Text(command).font(.system(size: 12, design: .monospaced)).foregroundStyle(Theme.gray300)
                                            .fixedSize().padding(.horizontal, 12).padding(.vertical, 6)
                                    }
                                    .scrollIndicators(.never)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.white(0.04)))
                                    .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.08), lineWidth: 1))
                                    CopyTextButton(text: command)
                                }
                            }
                        }
                        if let note = setup.note {
                            Text(note).font(.ui(11)).foregroundStyle(Theme.gray500).padding(.top, 8).fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    .padding(16)
                    .sunkenCard(radius: Theme.radiusLg)
                }
            }
        }
    }
}

struct CopyTextButton: View {
    let text: String
    @State private var copied = false
    var body: some View {
        QuietButton(label: copied ? "Copied" : "Copy", bordered: false, fontSize: 12, radius: Theme.radius,
                    color: Theme.gray400, hoverColor: Theme.gray200, padding: (8, 4)) {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)
            copied = true
            Task { try? await Task.sleep(for: .seconds(2)); copied = false }
        }
    }
}

/// KeysSettings.tsx: the secrets connections hold, with test and rotate.
struct KeysSettings: View {
    @Bindable var model: SettingsModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Keys").font(.ui(20, weight: .semibold)).foregroundStyle(.white).padding(.bottom, 4)
            Text("The secrets your connections hold, stored encrypted with your OS keychain").font(.ui(14))
                .foregroundStyle(Theme.gray500).padding(.bottom, 24)
            VStack(spacing: 8) {
                ForEach(model.connectorKeys) { KeyRow(model: model, entry: $0) }
            }
            if model.connectorKeysLoaded && model.connectorKeys.isEmpty {
                VStack(spacing: 0) {
                    LucideIcon(.keyRound, size: 16).foregroundStyle(Theme.gray600).padding(.bottom, 8)
                    Text("No keys yet").font(.ui(14)).foregroundStyle(Theme.gray400)
                    Text("A connection that signs in with a key or a token shows up here.").font(.ui(11)).foregroundStyle(Theme.gray600).padding(.top, 4)
                }
                .frame(maxWidth: .infinity)
                .padding(.horizontal, 16)
                .padding(.vertical, 24)
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusSm).strokeBorder(Theme.hairline, lineWidth: 1))
            }
        }
        .task { await model.loadConnectorKeys() }
    }
}

private struct KeyRow: View {
    let model: SettingsModel
    let entry: ConnectorKey
    @State private var testing = false
    @State private var result: (ok: Bool?, message: String?)?
    @State private var rotating: String?
    @State private var value = ""
    @State private var saving = false
    @State private var error: String?

    private func describe(_ f: ConnectorKey.Field) -> String {
        if !f.readable { return "Locked — this machine cannot read it" }
        if let env = f.envNames { return env.isEmpty ? "No variables set" : env.joined(separator: " · ") }
        return f.hint.flatMap { $0.isEmpty ? nil : $0 } ?? "Set"
    }

    private var usage: String {
        entry.usageCount == 0 ? "unused" : entry.usageCount == 1 ? "used by 1 step" : "used by \(entry.usageCount) steps"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 12) {
                LucideIcon(.keyRound, size: 14).foregroundStyle(Theme.gray400)
                VStack(alignment: .leading, spacing: 0) {
                    Text(entry.name).font(.ui(13)).foregroundStyle(Theme.gray200).lineLimit(1)
                    Text((entry.fields.map(describe) + [usage]).joined(separator: " · ")).font(.ui(11)).foregroundStyle(Theme.gray500).lineLimit(1)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                if let result {
                    Circle().fill(result.ok == nil ? Theme.gray600 : result.ok == true ? Theme.statusSage : Theme.danger).frame(width: 6, height: 6)
                }
                if entry.connectorId == "http" {
                    small(testing ? "Testing…" : "Test", disabled: testing) {
                        testing = true
                        result = nil
                        Task {
                            let r = await model.preflight(entry.connectionId)
                            result = (r.ok, r.message)
                            testing = false
                        }
                    }
                }
                ForEach(entry.fields, id: \.key) { field in
                    small(entry.fields.count > 1 ? "Rotate \(field.label)" : "Rotate", icon: .refreshCw) {
                        rotating = rotating == field.key ? nil : field.key
                        value = ""
                        error = nil
                    }
                }
            }
            if let result {
                Text(result.ok == nil ? "This connector has nothing to check" : result.message ?? (result.ok == true ? "Reached it" : "Could not reach it"))
                    .font(.ui(11)).foregroundStyle(result.ok == false ? Theme.danger : Theme.gray500).padding(.top, 6)
            }
            if let rotating {
                HStack(spacing: 8) {
                    SecureField("", text: $value, prompt: Text(entry.fields.first { $0.key == rotating }?.envNames != nil ? "{\"TOKEN\": \"…\"}" : "The replacement value"))
                        .textFieldStyle(.plain).font(.ui(12)).foregroundStyle(Theme.gray200)
                        .padding(.horizontal, 8).padding(.vertical, 4)
                        .background(RoundedRectangle(cornerRadius: Theme.radiusSm).fill(Theme.white(0.05)))
                        .overlay(RoundedRectangle(cornerRadius: Theme.radiusSm).strokeBorder(Theme.white(0.1), lineWidth: 1))
                    small(saving ? "Saving…" : "Save", disabled: saving || value.trimmingCharacters(in: .whitespaces).isEmpty) {
                        saving = true
                        Task {
                            error = await model.rotateSecret(connectionId: entry.connectionId, field: rotating, plaintext: value)
                            if error == nil { self.rotating = nil; value = ""; result = nil }
                            saving = false
                        }
                    }
                    Button("Cancel") { self.rotating = nil; value = ""; error = nil }
                        .buttonStyle(.plain).font(.ui(11)).foregroundStyle(Theme.gray500)
                }
                .padding(.top, 8)
            }
            if let error { Text(error).font(.ui(11)).foregroundStyle(Theme.danger).padding(.top, 6) }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(RoundedRectangle(cornerRadius: Theme.radiusSm).fill(Theme.white(0.03)))
        .overlay(RoundedRectangle(cornerRadius: Theme.radiusSm).strokeBorder(Theme.hairline, lineWidth: 1))
    }

    private func small(_ label: String, icon: LucideGlyph? = nil, disabled: Bool = false, action: @escaping () -> Void) -> some View {
        QuietButton(label: label, icon: icon, iconSize: 11, fontSize: 11, radius: Theme.radiusSm, fill: .clear,
                    hoverFill: .clear, color: Theme.gray400, hoverColor: Theme.gray200, padding: (8, 4), disabled: disabled, action: action)
    }
}

/// AboutSettings.tsx and NativeCoreStatus.tsx.
struct AboutSettings: View {
    @Bindable var model: SettingsModel
    @State private var endFailure: String?
    @State private var confirming: SessionHolder?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("About").font(.ui(20, weight: .semibold)).foregroundStyle(.white).padding(.bottom, 24)
            VornLogo(height: 24).opacity(0.6).padding(.bottom, 12)
            Text("Vorn v\(model.appVersion)").font(.ui(14)).foregroundStyle(Theme.gray300)
            Text("© 2026 Javier Canizalez").font(.ui(12)).foregroundStyle(Theme.gray600).padding(.top, 4)
            VStack(alignment: .leading, spacing: 8) {
                if let v = model.core?.version { Text("Native core \(v)").font(.ui(12)).foregroundStyle(Theme.gray500) }
                if let h = model.holders, let err = h.error, h.current == nil {
                    notice { Text("The session holder is not running: \(err).") }
                }
                ForEach((model.holders?.older ?? []).filter { $0.sessions != 0 }, id: \.instance) { h in
                    notice {
                        HStack(spacing: 12) {
                            Text(Self.olderNote(h)).frame(maxWidth: .infinity, alignment: .leading)
                            QuietButton(label: "End them", fontSize: 12, radius: Theme.radius, fill: .clear, hoverFill: Theme.white(0.06),
                                        padding: (8, 4)) { confirming = h }
                        }
                    }
                }
                if let endFailure { Text(endFailure).font(.ui(12)).foregroundStyle(Theme.red400) }
            }
            .padding(.top, 24)
        }
        .task { await model.loadCore() }
        .alert(confirming.map { "End \($0.sessions.map { Self.plural($0, "session", "sessions") } ?? "the sessions") on the older session holder? Their processes stop." } ?? "",
               isPresented: Binding(get: { confirming != nil }, set: { if !$0 { confirming = nil } })) {
            Button("Cancel", role: .cancel) { confirming = nil }
            Button("OK") {
                guard let h = confirming else { return }
                confirming = nil
                Task {
                    let why = await model.endHolder(h)
                    endFailure = why.map { "Could not end them: \($0)." }
                }
            }
        }
    }

    static func plural(_ n: Int, _ one: String, _ many: String) -> String { "\(n) \(n == 1 ? one : many)" }

    static func olderNote(_ h: SessionHolder) -> String {
        let held = h.sessions.map { "\(plural($0, "session", "sessions")) started before Vorn was updated" }
            ?? "Sessions started before Vorn was updated"
        let verb = h.sessions == 1 ? "is" : "are"
        return h.compatible
            ? "\(held) \(verb) still on the older session holder (\(h.build)). It exits after the last one ends."
            : "\(held) \(verb) on a session holder (\(h.build)) this version cannot talk to. They keep running until you end them."
    }

    private func notice<C: View>(@ViewBuilder _ c: () -> C) -> some View {
        c().font(.ui(12)).foregroundStyle(Theme.gray400)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16).padding(.vertical, 12)
            .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.white(0.03)))
            .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.08), lineWidth: 1))
    }
}
