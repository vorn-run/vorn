import SwiftUI
import VornCore
import VornUI

/// SettingsPage.tsx: a full-window page with the category nav on the left.
public struct SettingsPage: View {
    @Bindable var model: SettingsModel
    let onClose: () -> Void

    public init(model: SettingsModel, onClose: @escaping () -> Void) {
        self.model = model
        self.onClose = onClose
    }

    private struct Item {
        let key: SettingsCategory
        let label: String
        let icon: LucideGlyph
    }

    private let sections: [(String, [Item])] = [
        ("APP SETTINGS", [
            Item(key: .appearance, label: "Appearance", icon: .navAppearance),
            Item(key: .general, label: "General", icon: .navGeneral),
            Item(key: .updates, label: "Updates", icon: .navUpdates),
            Item(key: .notifications, label: "Notifications", icon: .navNotifications),
            Item(key: .worktrees, label: "Worktrees", icon: .navWorktrees),
        ]),
        ("CONNECTIONS", [
            Item(key: .agents, label: "Coding Agents", icon: .navAgents),
            Item(key: .ssh, label: "SSH & Hosts", icon: .navSSH),
            Item(key: .mcp, label: "Skills & MCP", icon: .navMcp),
            Item(key: .connectors, label: "Plugins", icon: .navPlugins),
            Item(key: .keys, label: "Keys", icon: .navKeys),
            Item(key: .network, label: "Remote Access", icon: .navNetwork),
        ]),
    ]

    public var body: some View {
        HStack(spacing: 0) {
            nav
            ScrollView {
                content
                    .padding(.horizontal, 32)
                    .padding(.vertical, 32)
                    .frame(maxWidth: 672, alignment: .leading)
                    .frame(maxWidth: .infinity)
            }
            .scrollIndicators(.automatic)
        }
        .background(Theme.surfaceOverlay)
        .overlay(alignment: .bottom) { WorktreeSelectionHost(model: model) }
        .task { if model.config == nil { await model.load() } }
    }

    private var nav: some View {
        VStack(spacing: 0) {
            HStack {
                Hovering { hover in
                    Button(action: onClose) {
                        HStack(spacing: 8) {
                            LucideIcon(.navBack, size: 16)
                            Text("Back to app").font(.ui(14)).lineHeight(20, size: 14)
                        }
                        .foregroundStyle(hover ? .white : Theme.gray400)
                    }
                    .buttonStyle(.plain)
                }
                Spacer(minLength: 0)
            }
            .padding(.leading, 78)
            .padding(.trailing, 12)
            .frame(height: 52)
            .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1) }

            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(sections, id: \.0) { header, items in
                        Text(header).font(.ui(10, weight: .medium)).tracking(0.5).foregroundStyle(Theme.gray600)
                            .padding(.horizontal, 12).padding(.top, 20).padding(.bottom, 6)
                        VStack(spacing: 2) {
                            ForEach(items, id: \.key) { item in
                                navButton(item.key, item.label, item.icon, idle: Theme.gray400)
                            }
                        }
                    }
                }
                .padding(.horizontal, 12)
                .padding(.top, 4)
            }
            .scrollIndicators(.never)

            navButton(.about, "About", .navAbout, idle: Theme.gray500)
                .padding(.horizontal, 12)
                .padding(.top, 8)
                .padding(.bottom, 16)
                .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
        }
        .frame(width: 224)
        .background(Theme.surfaceSunken)
        .overlay(alignment: .trailing) { Rectangle().fill(Theme.hairline).frame(width: 1) }
    }

    private func navButton(_ key: SettingsCategory, _ label: String, _ icon: LucideGlyph, idle: Color) -> some View {
        let selected = model.category == key
        return Hovering { hover in
            Button { model.category = key } label: {
                HStack(spacing: 10) {
                    LucideIcon(icon, size: 16, strokeWidth: 1.5)
                    Text(label).font(.ui(13)).lineHeight(19.5, size: 13)
                    Spacer(minLength: 0)
                }
                .foregroundStyle(selected || hover ? .white : idle)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(RoundedRectangle(cornerRadius: Theme.radiusMd)
                    .fill(selected ? Theme.white(0.08) : hover ? Theme.white(0.04) : .clear))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }

    @ViewBuilder private var content: some View {
        switch model.category {
        case .appearance: AppearanceSettings(model: model)
        case .general: GeneralSettings(model: model)
        case .updates: UpdatesSettings(model: model)
        case .notifications: NotificationSettings(model: model)
        case .worktrees: WorktreeSettings(model: model)
        case .agents: AgentSettings(model: model)
        case .ssh: SSHSettings(model: model)
        case .mcp: McpSettings()
        case .keys: KeysSettings(model: model)
        case .about: AboutSettings(model: model)
        case .connectors:
            SettingsPageHeader(title: "Plugins", description: "Connectors watch a service and start workflows. Extensions add footers and panes to a card. Both install as packs from the same directory.")
        case .network: NetworkSettings(model: model)
        }
    }
}
