import SwiftUI
import VornCore
import VornUI

/// ConnectorSettings.tsx: Settings › Plugins, the installed packs, the connections and the directory.
public struct PluginsSettings: View {
    @Bindable var model: PluginsModel

    public init(model: PluginsModel) {
        self.model = model
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SettingsPageHeader(
                title: "Plugins",
                description: "Connectors watch a service and start workflows. Extensions add footers and panes to a card. Both install as packs from the same directory."
            )
            SegmentedControl(options: PluginsModel.Tab.allCases.map { SegmentOption(value: $0.rawValue, label: $0.label) },
                             value: model.tab.rawValue) { model.show(PluginsModel.Tab(rawValue: $0) ?? .browse) }
                .padding(.bottom, 16)
            content
        }
        .task {
            model.start()
            if !model.loaded { await model.load() }
        }
        .onDisappear { model.stop() }
    }

    @ViewBuilder
    private var content: some View {
        let selected = model.selectedKey.flatMap { key in model.listings.first { $0.key == key } }
        if let adding = model.adding {
            addForm(adding)
        } else if model.tab != .connections, let selected {
            // Opened from whichever tab was showing, and closing goes back to it.
            ConnectorDetailView(model: model, listing: selected, backLabel: model.tab == .installed ? "All plugins" : "All connectors")
        } else {
            switch model.tab {
            case .installed: InstalledPluginsView(model: model)
            case .browse: ConnectorDirectoryView(model: model)
            case .connections:
                VStack(alignment: .leading, spacing: 20) {
                    if !model.loaded {
                        HStack(spacing: 8) {
                            Spinner(size: 12)
                            Text("Loading connections…")
                        }
                        .font(.ui(13)).foregroundStyle(Theme.gray500)
                    } else {
                        ConnectionGroupsView(model: model)
                        if model.connections.isEmpty && model.packs.isEmpty {
                            Text("No connections yet. Browse the connectors Vorn can talk to.").font(.ui(13)).foregroundStyle(Theme.gray500)
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func addForm(_ adding: ConnectorListing) -> some View {
        if adding.catalogItem != nil || adding.pack != nil {
            // A side-loaded pack has no catalog entry but does have files to probe.
            VStack(alignment: .leading, spacing: 12) {
                Text("Add \(adding.name) connection").font(.ui(13, weight: .medium)).foregroundStyle(Theme.gray200)
                SdkConnectorForm(model: model, catalogEntry: adding.catalogItem, pack: adding.pack,
                                 onDone: { await model.finishAdding() }, onCancel: { model.adding = nil })
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .pluginBox(padding: 0)
            .id(adding.key)
        } else if adding.source == .builtin, let connector = model.builtIns.first(where: { $0.id == adding.id }) {
            AddConnectionForm(model: model, connector: connector).id(adding.key)
        } else if let server = adding.mcpServer {
            // A generic server has no manifest to probe, so it goes to the manual form with its launch line written.
            if let mcp = model.mcpConnector {
                AddConnectionForm(model: model, connector: mcp,
                                  initialAuth: ["command": server.command,
                                                "args": JSONValue.array(server.args.map { .string($0) }).compactJSON],
                                  extraFilters: [SdkFilterKey.connectorId: server.id], startManual: true)
                    .id(adding.key)
            } else {
                Text("The MCP connector is not available in this build, so \(adding.name) cannot be added here.")
                    .font(.ui(12)).foregroundStyle(Theme.danger)
                    .padding(16)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .overlay(RoundedRectangle(cornerRadius: Theme.radiusSm).strokeBorder(Theme.white(0.08), lineWidth: 1))
            }
        }
    }
}

extension PluginsModel.Tab {
    var label: String {
        switch self {
        case .installed: "Installed"
        case .connections: "Connections"
        case .browse: "Browse"
        }
    }
}
