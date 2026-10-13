import SwiftUI
import VornCore
import VornUI

/// GeneralSettings.tsx.
struct GeneralSettings: View {
    @Bindable var model: SettingsModel

    var body: some View {
        if model.config != nil {
            let d = model.defaults
            VStack(alignment: .leading, spacing: 0) {
                SettingsPageHeader(title: "General", description: "Application preferences and defaults")
                VStack(spacing: 4) {
                    SettingRow(label: "Default Coding Agent", description: "Pre-selected agent when creating new sessions") {
                        agentPicker(current: d["defaultAgent"]?.stringValue ?? "claude")
                    }
                    SettingRow(label: "Default Shell",
                               description: "Shell used for terminal sessions. Shells differ in how much they can report about each command.") {
                        ShellPicker(model: model, value: d["shell"]?.stringValue ?? "/bin/zsh") {
                            model.updateDefaults(["shell": .string($0)])
                        }
                    }
                    SettingRow(label: "Start Vorn When I Sign In",
                               description: "Vorn opens with the machine, so a restart puts you back where you were.",
                               note: d["reopenSessions"]?.boolValue != false
                                   ? "With Reopen Sessions on, your agents start again at sign-in rather than when you next open Vorn."
                                   : nil) {
                        ToggleSwitch(checked: d["startAtLogin"]?.boolValue == true) { on in
                            model.updateDefaults(["startAtLogin": .bool(on)])
                            SettingsModel.applyLoginItem(on)
                        }
                    }
                    toggle("Reopen Sessions on Startup",
                           "Bring back the panes you had open. Agents still running are reconnected to, keeping the turn they were in; ones whose process was stopped are started again where they left off, and shells reopen in the directory they were in.",
                           "reopenSessions", true)
                    SettingRow(label: "Keep Sessions Running When Vorn Closes",
                               description: "Agents keep working while Vorn is shut. Reopening reconnects to them instead of restarting them. A background server stays running until every session ends, then shuts down on its own.",
                               note: serverStaysUp) {
                        ToggleSwitch(checked: d["keepSessionsRunning"]?.boolValue ?? true) {
                            model.updateDefaults(["keepSessionsRunning": .bool($0)])
                        }
                    }
                    toggle("Command Blocks",
                           "Draw finished commands as blocks with their own padding, boundary and copy button, instead of leaving them in the terminal grid",
                           "domBlockRendering", true)
                    toggle("Minimal Shell Prompt",
                           "Replace your shell prompt in terminal sessions so each command reads as a heading. Turn off to keep your own prompt exactly as your shell renders it",
                           "minimalShellPrompt", true)
                    toggle("Hover Preview", "Preview sessions by hovering over them in the sidebar", "enableHoverPreview", false)
                    SettingRow(label: "Floating Widget", description: "Show agent status widget when app is not focused") {
                        ToggleSwitch(checked: d["widgetEnabled"]?.boolValue == true) { model.updateDefaults(["widgetEnabled": .bool($0)]) }
                    }
                    SettingRow(label: "Show Headless Agents", description: "Display background agent sessions above the session grid") {
                        ToggleSwitch(checked: d["showHeadlessAgents"]?.boolValue != false) {
                            model.updateDefaults(["showHeadlessAgents": .bool($0)])
                        }
                    }
                    if d["showHeadlessAgents"]?.boolValue != false {
                        SettingRow(label: "Completed Agent Retention", description: "How long to show completed headless agents before auto-hiding") {
                            SettingsSelect(options: [(1, "1 minute"), (5, "5 minutes"), (15, "15 minutes"), (30, "30 minutes"), (60, "1 hour")],
                                           value: d["headlessRetentionMinutes"]?.numberValue ?? 1) {
                                model.updateDefaults(["headlessRetentionMinutes": .number($0)])
                            }
                        }
                    }
                    SettingRow(label: "Workflow Step Timeout",
                               description: "How long a headless workflow step may run before its agent is killed and the step fails. Individual steps can override this.") {
                        SettingsSelect(options: [(15, "15 minutes"), (30, "30 minutes"), (60, "1 hour"), (120, "2 hours"), (360, "6 hours"), (0, "No limit")],
                                       value: d["headlessStepTimeoutMinutes"]?.numberValue ?? 60) {
                            model.updateDefaults(["headlessStepTimeoutMinutes": .number($0)])
                        }
                    }
                }
            }
            .task { if model.installedAgents == nil { await model.detectAgents() } }
        }
    }

    /// Two settings quietly override the keep-running promise; say so where it is made.
    private var serverStaysUp: String? {
        let d = model.defaults
        let polling = (model.config?["workflows"]?.arrayValue ?? []).contains { w in
            guard w["enabled"]?.boolValue == true else { return false }
            let trigger = w["nodes"]?.arrayValue?.first { $0["type"]?.stringValue == "trigger" }
            return trigger?["config"]?["triggerType"]?.stringValue == "connectorPoll"
        }
        let reasons = [
            d["networkAccessEnabled"]?.boolValue == true
                ? "Network Access is on, so it stays up to be reached from your other devices" : nil,
            polling ? "a connector is polling on a schedule, which it services itself" : nil,
        ].compactMap { $0 }
        return reasons.isEmpty ? nil : "\(reasons.joined(separator: ", and ")). This server will not shut down on its own."
    }

    private func toggle(_ label: String, _ desc: String, _ key: String, _ fallback: Bool) -> some View {
        SettingRow(label: label, description: desc) {
            ToggleSwitch(checked: model.defaults[key]?.boolValue ?? fallback) { model.updateDefaults([key: .bool($0)]) }
        }
    }

    private func agentPicker(current: String) -> some View {
        HStack(spacing: 2) {
            ForEach(AgentType.agents, id: \.rawValue) { agent in
                let installed = model.isInstalled(agent)
                let selected = current == agent.rawValue
                Hovering { hover in
                    Button {
                        if installed { model.updateDefaults(["defaultAgent": .string(agent.rawValue)]) }
                    } label: {
                        HStack(spacing: 6) {
                            AgentIcon(agent, size: 14)
                            Text(String(agent.displayName.split(separator: " ").first ?? "")).font(.ui(12))
                        }
                        .foregroundStyle(!installed ? Theme.gray600 : selected ? .white : hover ? Theme.gray300 : Theme.gray500)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 4)
                        .background(RoundedRectangle(cornerRadius: Theme.radiusMd).fill(selected && installed ? Theme.white(0.1) : .clear))
                        .opacity(installed ? 1 : 0.3)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .help(installed ? agent.displayName : "\(agent.displayName) is not installed")
                }
            }
        }
        .padding(2)
        .background(RoundedRectangle(cornerRadius: Theme.radiusLg).fill(Theme.white(0.04)))
    }
}

/// ShellPicker.tsx: detected shells, best first, each with what it can report; or a typed path.
struct ShellPicker: View {
    @Bindable var model: SettingsModel
    let value: String
    let onChange: (String) -> Void
    @State private var open = false
    @State private var editing = false
    @State private var draft = ""

    static let sigils = ["zsh": "%", "bash": "$", "fish": "><>", "powershell": "PS", "cmd": "C:\\"]

    var body: some View {
        if editing {
            SettingsField(text: $draft, placeholder: "Path to a shell", mono: true) {
                let next = draft.trimmingCharacters(in: .whitespaces)
                if !next.isEmpty && next != value { onChange(next) }
                editing = false
            }
            .onExitCommand { editing = false }
            .frame(width: 288)
        } else {
            let current = model.shells?.first { $0.path == value }
            Hovering { hover in
                Button { open.toggle() } label: {
                    HStack(spacing: 8) {
                        glyph(current?.family)
                        VStack(alignment: .leading, spacing: 0) {
                            Text(current?.name ?? value.split(whereSeparator: { $0 == "/" || $0 == "\\" }).last.map(String.init) ?? "Select a shell")
                                .font(.ui(13)).foregroundStyle(Theme.gray200).lineLimit(1)
                            Text(value).font(.system(size: 11, design: .monospaced)).foregroundStyle(Theme.gray600).lineLimit(1)
                        }
                        Spacer(minLength: 0)
                        LucideIcon(.chevronDown, size: 12).foregroundStyle(Theme.gray500)
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 6)
                    .frame(width: 288)
                    .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(hover ? 0.25 : 0.12), lineWidth: 1))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            .popover(isPresented: $open, arrowEdge: .bottom) { menu }
            .task { if model.shells == nil { await model.loadShells() } }
        }
    }

    private var menu: some View {
        VStack(alignment: .leading, spacing: 0) {
            if model.shells == nil {
                Text("Looking for shells…").font(.ui(12)).foregroundStyle(Theme.gray500).padding(.horizontal, 12).padding(.vertical, 8)
            } else if model.shells?.isEmpty == true {
                Text("No shells detected. Enter a path instead.").font(.ui(12)).foregroundStyle(Theme.gray500)
                    .padding(.horizontal, 12).padding(.vertical, 8)
            }
            ForEach(model.shells ?? []) { shell in
                Hovering { hover in
                    Button {
                        open = false
                        if shell.path != value { onChange(shell.path) }
                    } label: {
                        HStack(alignment: .top, spacing: 8) {
                            glyph(shell.family).padding(.top, 1)
                            VStack(alignment: .leading, spacing: 0) {
                                HStack(alignment: .firstTextBaseline, spacing: 6) {
                                    Text(shell.name).font(.ui(13)).foregroundStyle(Theme.gray200)
                                    if let v = shell.version {
                                        Text(v).font(.system(size: 11, design: .monospaced)).foregroundStyle(Theme.gray600)
                                    }
                                }
                                Text(shell.path).font(.system(size: 11, design: .monospaced)).foregroundStyle(Theme.gray600).lineLimit(1)
                                blockSupport(shell.blocks).padding(.top, 2)
                            }
                            Spacer(minLength: 0)
                            if shell.path == value { LucideIcon(.check, size: 13).foregroundStyle(Theme.gray400).padding(.top, 4) }
                        }
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .background(hover ? Theme.white(0.06) : .clear)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
            Rectangle().fill(Theme.hairline).frame(height: 1).padding(.vertical, 4)
            Hovering { hover in
                Button {
                    open = false
                    draft = value
                    editing = true
                } label: {
                    HStack(spacing: 8) {
                        LucideIcon(.penLine, size: 13).foregroundStyle(Theme.gray500)
                        Text("Enter a path…").font(.ui(12)).foregroundStyle(Theme.gray400)
                        Spacer(minLength: 0)
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(hover ? Theme.white(0.06) : .clear)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.vertical, 4)
        .frame(minWidth: 288)
        .background(Theme.surfaceOverlay)
    }

    private func glyph(_ family: String?) -> some View {
        Text(family.flatMap { Self.sigils[$0] } ?? "·")
            .font(.system(size: 9, design: .monospaced))
            .foregroundStyle(Theme.gray400)
            .frame(width: 24, height: 24)
            .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.white(0.12), lineWidth: 1))
    }

    @ViewBuilder private func blockSupport(_ b: InstalledShell.Blocks) -> some View {
        if b.level == "full" {
            Text("Full command blocks").font(.ui(11)).foregroundStyle(Theme.gray500)
        } else {
            Text(b.limitation ?? "").font(.ui(11)).foregroundStyle(b.level == "limited" ? Theme.amber500.opacity(0.8) : Theme.gray500)
        }
    }
}
