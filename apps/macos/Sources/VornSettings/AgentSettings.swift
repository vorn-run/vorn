import AppKit
import SwiftUI
import VornCore
import VornUI

/// AgentSettings.tsx: each agent's command and arguments, and how to install the missing ones.
struct AgentSettings: View {
    @Bindable var model: SettingsModel
    @State private var argsDraft: [String: String] = [:]
    @State private var headlessDraft: [String: String] = [:]
    @State private var expanded: Set<String> = []
    @State private var copied: String?

    private func command(_ def: AgentDefinition) -> JSONValue {
        model.config?["agentCommands"]?[def.type.rawValue] ?? def.defaultCommand
    }

    private func update(_ def: AgentDefinition, _ patch: [String: JSONValue]) {
        var current = command(def).objectValue ?? [:]
        for (k, v) in patch { current[k] = v }
        var all: [String: JSONValue] = [:]
        for d in AgentDefinition.all { all[d.type.rawValue] = d.defaultCommand }
        for (k, v) in model.config?["agentCommands"]?.objectValue ?? [:] { all[k] = v }
        all[def.type.rawValue] = .object(current)
        model.updateConfig(["agentCommands": .object(all)])
    }

    private static func words(_ s: String) -> JSONValue {
        let t = s.trimmingCharacters(in: .whitespaces)
        return .array(t.isEmpty ? [] : t.split(whereSeparator: \.isWhitespace).map { .string(String($0)) })
    }

    private static func joined(_ v: JSONValue?) -> String { (v?.arrayValue ?? []).compactMap(\.stringValue).joined(separator: " ") }

    private func isModified(_ def: AgentDefinition) -> Bool {
        let c = command(def)
        return c["command"]?.stringValue != def.command || (c["args"]?.arrayValue ?? []) != []
            || (c["headlessArgs"]?.arrayValue ?? []) != def.headlessArgs.map { .string($0) }
    }

    var body: some View {
        if model.config != nil {
            let installed = AgentDefinition.all.filter { model.isInstalled($0.type) }
            let missing = AgentDefinition.all.filter { !model.isInstalled($0.type) }
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    Text("Coding Agents").font(.ui(20, weight: .semibold)).tracking(-0.4).foregroundStyle(.white)
                    Spacer()
                    QuietButton(label: model.detectingAgents ? "Scanning..." : "Re-detect", icon: .refreshCw, iconSize: 11,
                                fontSize: 12, radius: Theme.radiusLg, color: Theme.gray400, hoverColor: .white,
                                padding: (12, 6), disabled: model.detectingAgents) { Task { await model.detectAgents() } }
                        .help("Re-detect installed agents")
                }
                .padding(.bottom, 4)
                Text("Configure the command and arguments for each coding agent").font(.ui(14)).foregroundStyle(Theme.gray500).padding(.bottom, 24)

                HStack(spacing: 16) {
                    HStack(spacing: 8) {
                        Circle().fill(Color(hex: 0x00D492)).frame(width: 8, height: 8)
                        (Text("\(installed.count)").foregroundColor(Theme.gray200).fontWeight(.medium) + Text(" installed"))
                            .font(.ui(12)).foregroundStyle(Theme.gray400)
                    }
                    if !missing.isEmpty {
                        HStack(spacing: 8) {
                            Circle().fill(Theme.gray600).frame(width: 8, height: 8)
                            (Text("\(missing.count)").fontWeight(.medium) + Text(" available")).font(.ui(12)).foregroundStyle(Theme.gray400)
                        }
                    }
                    Spacer()
                    Text("\(AgentDefinition.all.count) AGENTS").font(.ui(10)).tracking(0.5).foregroundStyle(Theme.gray600)
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
                .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.surfaceSunken))
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(0.04), lineWidth: 1))
                .padding(.bottom, 24)

                if !installed.isEmpty {
                    VStack(spacing: 8) { ForEach(installed, id: \.type.rawValue) { installedCard($0) } }
                        .padding(.bottom, 32)
                }
                if !missing.isEmpty {
                    HStack(spacing: 8) {
                        LucideIcon(.download, size: 12).foregroundStyle(Theme.gray600)
                        Text("AVAILABLE TO INSTALL").font(.ui(12, weight: .medium)).tracking(0.6).foregroundStyle(Theme.gray500)
                    }
                    .padding(.bottom, 12)
                    VStack(spacing: 6) { ForEach(missing, id: \.type.rawValue) { missingCard($0) } }
                }
            }
            .task { if model.installedAgents == nil { await model.detectAgents() } }
        }
    }

    private func installedCard(_ def: AgentDefinition) -> some View {
        let key = def.type.rawValue
        let c = command(def)
        return Hovering { hover in
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 12) {
                    AgentIcon(def.type, size: 18)
                        .frame(width: 36, height: 36)
                        .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(def.bg))
                    VStack(alignment: .leading, spacing: 0) {
                        HStack(spacing: 8) {
                            Text(def.type.displayName).font(.ui(14, weight: .medium)).foregroundStyle(Color(hex: 0xF3F4F6)).lineHeight(20, size: 14)
                            HStack(spacing: 4) {
                                LucideIcon(.terminal, size: 8)
                                Text("Ready").font(.ui(10, weight: .medium))
                            }
                            .foregroundStyle(def.tint)
                            .padding(.horizontal, 6).padding(.vertical, 2)
                            .background(Capsule().fill(Color(hex: def.color, opacity: 0x12 / 255)))
                        }
                        Text(def.description).font(.ui(11)).foregroundStyle(Theme.gray500).lineHeight(16.5, size: 11)
                    }
                    Spacer(minLength: 0)
                    if isModified(def) {
                        QuietButton(label: "Reset", icon: .rotateCcw, iconSize: 10, fontSize: 11, fill: Theme.white(0.03),
                                    hoverFill: Theme.white(0.06), color: Theme.gray500, hoverColor: Theme.gray300, padding: (8, 4)) {
                            var all = model.config?["agentCommands"]?.objectValue ?? [:]
                            for d in AgentDefinition.all where all[d.type.rawValue] == nil { all[d.type.rawValue] = d.defaultCommand }
                            all[key] = def.defaultCommand
                            model.updateConfig(["agentCommands": .object(all)])
                            argsDraft[key] = nil
                            headlessDraft[key] = nil
                        }
                        .opacity(hover ? 1 : 0)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
                .overlay(alignment: .leading) { Rectangle().fill(Color(hex: def.color, opacity: 0x30 / 255)).frame(width: 2) }

                VStack(alignment: .leading, spacing: 8) {
                    HStack(alignment: .top, spacing: 8) {
                        VStack(alignment: .leading, spacing: 0) {
                            FieldLabel("Command")
                            SettingsField(text: Binding(get: { c["command"]?.stringValue ?? "" },
                                                        set: { update(def, ["command": .string($0)]) }), mono: true)
                        }
                        .frame(width: 140)
                        VStack(alignment: .leading, spacing: 0) {
                            FieldLabel("Arguments")
                            SettingsField(text: Binding(get: { argsDraft[key] ?? Self.joined(c["args"]) }, set: { argsDraft[key] = $0 }),
                                          placeholder: "e.g. --dangerously-skip-permissions", mono: true) {
                                if let v = argsDraft[key] {
                                    update(def, ["args": Self.words(v)])
                                    argsDraft[key] = nil
                                }
                            }
                        }
                    }
                    VStack(alignment: .leading, spacing: 0) {
                        FieldLabel("Headless Arguments")
                        SettingsField(text: Binding(get: { headlessDraft[key] ?? Self.joined(c["headlessArgs"]) }, set: { headlessDraft[key] = $0 }),
                                      placeholder: "e.g. --dangerously-skip-permissions (used for headless/workflow runs)", mono: true) {
                            if let v = headlessDraft[key] {
                                update(def, ["headlessArgs": Self.words(v)])
                                headlessDraft[key] = nil
                            }
                        }
                    }
                }
                .padding(.horizontal, 16)
                .padding(.top, 4)
                .padding(.bottom, 12)
            }
            .sunkenCard(border: hover ? Theme.white(0.1) : Theme.hairline)
        }
    }

    private func missingCard(_ def: AgentDefinition) -> some View {
        let key = def.type.rawValue
        let open = expanded.contains(key)
        return VStack(alignment: .leading, spacing: 0) {
            Button {
                if open { expanded.remove(key) } else { expanded.insert(key) }
            } label: {
                HStack(spacing: 12) {
                    AgentIcon(def.type, size: 18)
                        .frame(width: 36, height: 36)
                        .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.white(0.03)))
                        .opacity(0.5)
                    (Text(def.type.displayName).font(.ui(14, weight: .medium)).foregroundColor(Theme.gray400)
                        + Text("  " + def.description).font(.ui(11)).foregroundColor(Theme.gray600))
                        .frame(maxWidth: .infinity, alignment: .leading)
                    LucideIcon(open ? .chevronDown : .chevronRight, size: 14).foregroundStyle(Theme.gray600)
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if open {
                VStack(alignment: .leading, spacing: 0) {
                    HStack(spacing: 8) {
                        LucideIcon(.terminal, size: 10).foregroundStyle(Theme.gray500)
                        Text("INSTALL COMMAND").font(.ui(10, weight: .medium)).tracking(0.5).foregroundStyle(Theme.gray500)
                    }
                    .padding(.bottom, 8)
                    HStack(spacing: 8) {
                        HStack(spacing: 8) {
                            Text("$").font(.ui(12)).foregroundStyle(Theme.gray600)
                            Text(def.installCommand).font(.system(size: 12, design: .monospaced)).foregroundStyle(Theme.gray300).textSelection(.enabled)
                            Spacer(minLength: 0)
                        }
                        .padding(.horizontal, 12).padding(.vertical, 8)
                        .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.white(0.03)))
                        .overlay(RoundedRectangle(cornerRadius: Theme.radiusMd).strokeBorder(Theme.white(0.04), lineWidth: 1))
                        squareButton(copied == key ? .circleCheckBig : .copy, tint: copied == key ? Color(hex: 0x00D492) : nil,
                                     help: "Copy install command") {
                            NSPasteboard.general.clearContents()
                            NSPasteboard.general.setString(def.installCommand, forType: .string)
                            copied = key
                            Task { try? await Task.sleep(for: .seconds(2)); if copied == key { copied = nil } }
                        }
                        squareButton(.externalLink, help: "View documentation") {
                            if let url = URL(string: def.installUrl) { NSWorkspace.shared.open(url) }
                        }
                    }
                    Text("After installing, click Re-detect above to enable this agent.").font(.ui(10)).foregroundStyle(Theme.gray600).padding(.top, 8)
                }
                .padding(12)
                .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Color.black.opacity(0.3)))
                .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.hairline, lineWidth: 1))
                .padding(.horizontal, 16)
                .padding(.bottom, 16)
            }
        }
        .sunkenCard(border: Theme.white(0.04))
    }

    private func squareButton(_ icon: LucideGlyph, tint: Color? = nil, help: String, action: @escaping () -> Void) -> some View {
        Hovering { hover in
            Button(action: action) {
                LucideIcon(icon, size: 14).foregroundStyle(tint ?? (hover ? .white : Theme.gray500))
                    .padding(8)
                    .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.white(hover ? 0.08 : 0.04)))
                    .overlay(RoundedRectangle(cornerRadius: Theme.radiusLg).strokeBorder(Theme.white(hover ? 0.08 : 0.04), lineWidth: 1))
            }
            .buttonStyle(.plain)
            .help(help)
        }
    }
}
