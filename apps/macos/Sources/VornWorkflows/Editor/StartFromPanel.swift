import SwiftUI
import VornCore
import VornUI

/// What picking a template puts on the canvas (template-requirements.ts templateSeed).
public struct WorkflowSeed: Sendable, Hashable {
    public var name: String
    public var icon: String?
    public var iconColor: String?
    public var nodes: [WorkflowNode]
    public var edges: [WorkflowEdge]
    /// The requirements the seed could not bind, worded for the "still needs" toast.
    public var unresolved: [String]

    public init(name: String, icon: String? = nil, iconColor: String? = nil, nodes: [WorkflowNode],
                edges: [WorkflowEdge], unresolved: [String] = []) {
        self.name = name
        self.icon = icon
        self.iconColor = iconColor
        self.nodes = nodes
        self.edges = edges
        self.unresolved = unresolved
    }
}

/// What the Start from list shows: the catalog's templates and what this machine is connected to.
public struct StartFromData: Sendable {
    public var templates: [WorkflowTemplate]
    public var connections: [PortableConnection]
    public var suggestions: [ConnectorSuggestion]

    public init(templates: [WorkflowTemplate], connections: [PortableConnection], suggestions: [ConnectorSuggestion]) {
        self.templates = templates
        self.connections = connections
        self.suggestions = suggestions
    }
}

extension WorkflowsStore {
    /// connector:catalog, connection:list and connector:list, read together for the Start from list.
    public func startFromData() async -> StartFromData {
        async let templates = workflowTemplates()
        async let connections = portableConnections()
        async let listings = connectorListings()
        let (t, c, l) = await (templates, connections, listings)
        return StartFromData(
            templates: t, connections: c,
            suggestions: TemplateRequirements.connectorSuggestions(connections: c, connectors: l))
    }
}

/// StartFromPanel.tsx: the templates a new workflow can start from.
/// A requirement's own fix (install, add connection, create profile) is not offered here yet.
struct StartFromPanel: View {
    let store: WorkflowsStore
    let onPick: (WorkflowSeed) -> Void
    let onClose: () -> Void
    /// Data already in hand, so a render shows the list without a round trip.
    var preloaded: StartFromData? = nil

    @State private var loaded: StartFromData?
    @State private var profileFor: String?

    private var data: StartFromData { preloaded ?? loaded ?? StartFromData(templates: [], connections: [], suggestions: []) }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Start from").font(.system(size: 13, weight: .medium)).foregroundStyle(.white)
                Spacer()
                PanelCloseButton(help: "Close", action: onClose)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .overlay(alignment: .bottom) { Rectangle().fill(Theme.white(0.08)).frame(height: 1) }
            RequirementProfileSlot(store: store, profileFor: $profileFor) {
                Task { loaded = await store.startFromData() }
            }

            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    PickRow(action: onClose) { hovered in
                        rowIcon(.filePlusCorner)
                        VStack(alignment: .leading, spacing: 0) {
                            Text("Blank canvas").font(.system(size: 12, weight: .medium)).frame(height: 18)
                                .foregroundStyle(hovered ? Color.white : Theme.gray300)
                            Text("Pick your own trigger").font(.system(size: 11)).frame(height: 16.5).foregroundStyle(Theme.gray500)
                        }
                    }
                    if !data.templates.isEmpty { sectionHeader("Templates") }
                    ForEach(data.templates) { template in
                        TemplateRow(template: template, connections: data.connections, onFix: { fix in
                            if case .createProfile(let name) = fix { profileFor = name }
                        }) { pickTemplate(template) }
                    }
                    if !data.suggestions.isEmpty { sectionHeader("From your connections") }
                    ForEach(data.suggestions) { suggestion in
                        PickRow(action: { pickSuggestion(suggestion) }) { hovered in
                            rowIcon(.plug)
                            VStack(alignment: .leading, spacing: 0) {
                                Text(suggestion.name).font(.system(size: 12, weight: .medium)).frame(height: 18)
                                    .foregroundStyle(hovered ? Color.white : Theme.gray300)
                                Text(suggestion.connectionName).font(.system(size: 11)).frame(height: 16.5).foregroundStyle(Theme.gray500)
                            }
                            .lineLimit(1)
                        }
                    }
                }
                .padding(8)
            }
        }
        .frame(width: 280)
        .frame(maxHeight: .infinity, alignment: .top)
        .background(Theme.surfaceNode)
        .overlay(alignment: .leading) { Rectangle().fill(Theme.white(0.08)).frame(width: 1) }
        .onExitCommand(perform: onClose)
        .task(id: store.catalogGeneration) {
            guard preloaded == nil else { return }
            loaded = await store.startFromData()
        }
    }

    private func sectionHeader(_ title: String) -> some View {
        Text(title.uppercased())
            .font(.system(size: 10, design: .monospaced))
            .frame(height: 15)
            .tracking(0.5)
            .foregroundStyle(Theme.gray600)
            .padding(.horizontal, 8)
            .padding(.top, 12)
            .padding(.bottom, 4)
    }

    /// WorkflowEditor.tsx handlePickTemplate: the active project, else the first one.
    private func pickTemplate(_ template: WorkflowTemplate) {
        let project = store.projects.first { $0.name == store.activeProjectName } ?? store.projects.first
        let seed = TemplateRequirements.templateSeed(
            template, project: project.map { ($0.name, $0.path) }, connections: data.connections)
        onPick(seed)
        onClose()
        if !seed.unresolved.isEmpty {
            store.notify("Started from \"\(template.name)\" — still needs \(seed.unresolved.joined(separator: ", "))", tone: .warning)
        }
    }

    /// WorkflowEditor.tsx handlePickSuggestion: vornd builds it from the connector's manifest.
    private func pickSuggestion(_ suggestion: ConnectorSuggestion) {
        store.perform { [store, onClose] in
            let result = try await store.backend.rpc(
                "connector:seedWorkflow",
                .object(["connectionId": .string(suggestion.connectionId), "event": .string(suggestion.event)]))
            onClose()
            await store.reload()
            if let id = result["workflowId"]?.stringValue { store.openWorkflow(id) }
            store.notify("Started \"\(suggestion.name)\"", tone: .success)
        }
    }
}

private func rowIcon(_ glyph: LucideGlyph) -> some View {
    LucideIcon(glyph, size: 14, strokeWidth: 1.5).foregroundStyle(Theme.gray500).padding(.top, 2)
}

/// px-2.5 py-2 rounded-md row, white/.06 on hover.
private struct PickRow<Content: View>: View {
    let action: () -> Void
    @ViewBuilder let content: (Bool) -> Content

    var body: some View {
        Hovering { hovered in
            Button(action: action) {
                HStack(alignment: .top, spacing: 10) { content(hovered) }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
                    .background(hovered ? Theme.white(0.06) : .clear, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }
}

/// A template, its steps, and what it still wants connected.
private struct TemplateRow: View {
    let template: WorkflowTemplate
    let connections: [PortableConnection]
    var onFix: ((RequirementFix) -> Void)?
    let onPick: () -> Void

    var body: some View {
        let requirements = TemplateRequirements.templateRequirements(template, connections: connections)
        let pending = requirements.filter { $0.connectionId == nil }
        Hovering { hovered in
            VStack(alignment: .leading, spacing: 0) {
                Button(action: onPick) {
                    HStack(alignment: .top, spacing: 10) {
                        StepStrip(count: template.nodeCount).padding(.top, 2)
                        VStack(alignment: .leading, spacing: 0) {
                            Text(template.name).font(.system(size: 12, weight: .medium)).frame(height: 18)
                                .foregroundStyle(hovered ? Color.white : Theme.gray300)
                            Text(template.steps.joined(separator: " · ")).font(.system(size: 11)).frame(height: 16.5).foregroundStyle(Theme.gray500)
                            if pending.isEmpty && !requirements.isEmpty {
                                HStack(spacing: 4) {
                                    Icon(.check, size: 10, weight: .semibold)
                                    Text("Connected")
                                }
                                .font(.system(size: 11))
                                .frame(height: 16.5)
                                .foregroundStyle(Theme.statusSage)
                            }
                        }
                        .lineLimit(1)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                ForEach(pending, id: \.self) { entry in
                    RequirementRow(requirement: entry, onFix: onFix)
                }
            }
            .background(hovered ? Theme.white(0.06) : .clear, in: RoundedRectangle(cornerRadius: Theme.radiusMd))
        }
    }
}

/// The shape of the flow, drawn rather than counted.
private struct StepStrip: View {
    let count: Int

    var body: some View {
        VStack(spacing: 3) {
            ForEach(0..<min(count, 4), id: \.self) { _ in
                RoundedRectangle(cornerRadius: 1).fill(Theme.white(0.18)).frame(height: 3)
            }
        }
        .frame(width: 22)
        .opacity(0.7)
    }
}
