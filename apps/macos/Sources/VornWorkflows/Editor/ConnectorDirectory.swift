import Foundation
import Observation
import SwiftUI
import VornCore
import VornUI

/// SdkConnectorIcon: the glyph a connector package ships, as path data only.
public struct ConnectorGlyphData: Hashable, Sendable, Decodable {
    public var viewBox: String
    public var paths: [String]

    public init(viewBox: String = "0 0 24 24", paths: [String]) {
        self.viewBox = viewBox
        self.paths = paths
    }

    /// connection-icon.ts connectionIcon: re-validated, since a third-party manifest wrote it.
    static func parse(_ raw: String?) -> ConnectorGlyphData? {
        guard let raw, !raw.isEmpty, let value = JSONValue.parse(raw), case .object = value else { return nil }
        let paths = value["paths"]?.arrayValue?.compactMap(\.stringValue) ?? []
        guard !paths.isEmpty, paths.allSatisfy({ !$0.isEmpty }),
              paths.count == value["paths"]?.arrayValue?.count else { return nil }
        return ConnectorGlyphData(viewBox: value.str("viewBox") ?? "0 0 24 24", paths: paths)
    }

    static func from(_ value: JSONValue?) -> ConnectorGlyphData? {
        guard let value, let paths = value["paths"]?.arrayValue?.compactMap(\.stringValue), !paths.isEmpty else { return nil }
        return ConnectorGlyphData(viewBox: value.str("viewBox") ?? "0 0 24 24", paths: paths)
    }
}

/// use-connections.ts ConnectorLook: how a connector is drawn.
public struct ConnectorLook: Hashable, Sendable {
    public var connectorId: String
    public var icon: ConnectorGlyphData?
    public var packaged: Bool
}

/// ConnectorIcon.tsx: a package's own glyph, else GitHub, Linear and the plug drawn, HTTP a globe, else a link.
struct ConnectorGlyph: View {
    let connectorId: String
    var icon: ConnectorGlyphData?
    var packaged = false
    var size: CGFloat = 12
    var color: Color = Theme.ink

    var body: some View {
        ConnectorIcon(connectorId: connectorId, viewBox: icon?.viewBox, paths: icon?.paths, packaged: packaged, size: size, color: color)
    }
}

/// One published connector in the catalog (connector:catalog items).
public struct CatalogConnector: Hashable, Sendable {
    public var id: String
    public var name: String
    public var icon: ConnectorGlyphData?
    public var keywords: [String]
    public var verified: Bool
    public var version: String?
    public var packUrl: String?
    public var actions: [(type: String, label: String)]

    public static func == (a: Self, b: Self) -> Bool { a.id == b.id && a.name == b.name && a.version == b.version }
    public func hash(into h: inout Hasher) { h.combine(id) }
}

/// What the editor knows about this machine's connectors: connections, packs, the catalog and each connection's actions.
@Observable @MainActor
public final class ConnectorDirectory {
    public var connections: [FormConnection] = []
    public var packs: [(id: String, version: String?, icon: ConnectorGlyphData?)] = []
    public var catalog: [CatalogConnector] = []
    /// Trigger events by connector id (connector:list manifests).
    public var triggers: [String: [ConnectorInfo.Trigger]] = [:]
    public var actions: [String: [ConnectorActionDef]] = [:]
    @ObservationIgnored private var loading: Task<Void, Never>?

    public init() {}

    func load(_ backend: any WorkflowsBackend) {
        guard loading == nil else { return }
        loading = Task {
            async let conns = try? backend.listConnections()
            async let packs = try? backend.rpc("connector:listPacks", nil)
            async let catalog = try? backend.rpc("connector:catalog", nil)
            async let connectors = try? backend.listConnectors()
            if let v = await conns { connections = v }
            if let v = await packs?.arrayValue {
                self.packs = v.compactMap { p in
                    p.str("id").map { ($0, p.str("version"), ConnectorGlyphData.from(p["icon"])) }
                }
            }
            if let items = await catalog?["items"]?.arrayValue {
                self.catalog = items.compactMap(Self.catalogItem)
            }
            if let v = await connectors {
                triggers = Dictionary(v.map { ($0.id, $0.triggers) }, uniquingKeysWith: { a, _ in a })
            }
            for conn in connections {
                actions[conn.id] = (try? await backend.listConnectionActions(conn.id)) ?? []
            }
        }
    }

    static func catalogItem(_ item: JSONValue) -> CatalogConnector? {
        guard let id = item.str("id") else { return nil }
        let actions = (item["actions"]?.arrayValue ?? []).compactMap { a -> (String, String)? in
            guard let type = a.str("type") else { return nil }
            let label = a.str("label") ?? ""
            return (type, label.isEmpty ? type : label)
        }
        return CatalogConnector(
            id: id, name: item.str("name") ?? id, icon: ConnectorGlyphData.from(item["icon"]),
            keywords: item["keywords"]?.arrayValue?.compactMap(\.stringValue) ?? [],
            verified: item.bool("verified") ?? false, version: item.str("version"), packUrl: item.str("packUrl"),
            actions: actions.map { (type: $0.0, label: $0.1) })
    }

    /// connectorLookFor.
    func look(_ connectionId: String?) -> ConnectorLook? {
        guard let connectionId, !connectionId.isEmpty, let conn = connections.first(where: { $0.id == connectionId })
        else { return nil }
        let real = conn.realConnectorId
        let icon = ConnectorGlyphData.parse(conn.filters?.str("sdkIcon")) ?? packs.first { $0.id == real }?.icon
        return ConnectorLook(connectorId: real, icon: icon, packaged: real != conn.connectorId)
    }
}
