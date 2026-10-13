import Foundation

/// connector-browse.ts: one connector or extension as the directory lists it.
public struct ConnectorListing: Hashable, Sendable, Identifiable {
    public enum Source: String, Hashable, Sendable {
        case builtin, catalog, installed, mcp
    }
    public var key: String
    public var id: String
    public var name: String
    public var description: String?
    public var capabilities: [String]
    public var category: String
    public var kind: ConnectorKind
    public var source: Source
    public var mcpServer: McpServerEntry?
    public var keywords: [String]
    public var connectedCount: Int
    public var catalogItem: CatalogItem?
    public var icon: ConnectorIconData?
    public var pack: InstalledPack?
    public var authRung: ConnectorAuthRung?
    public var verified: CatalogVerification?
    public var implicitlyConnected: Bool
    public var contributes: ExtensionContributions?
    public var permissions: [ExtensionPermission]?
    public var activates: ExtensionActivation?

    public init(key: String, id: String, name: String, description: String? = nil, capabilities: [String] = [],
                category: String, kind: ConnectorKind = .connector, source: Source, mcpServer: McpServerEntry? = nil,
                keywords: [String] = [], connectedCount: Int = 0, catalogItem: CatalogItem? = nil, icon: ConnectorIconData? = nil,
                pack: InstalledPack? = nil, authRung: ConnectorAuthRung? = nil, verified: CatalogVerification? = nil,
                implicitlyConnected: Bool = false, contributes: ExtensionContributions? = nil,
                permissions: [ExtensionPermission]? = nil, activates: ExtensionActivation? = nil) {
        self.key = key
        self.id = id
        self.name = name
        self.description = description
        self.capabilities = capabilities
        self.category = category
        self.kind = kind
        self.source = source
        self.mcpServer = mcpServer
        self.keywords = keywords
        self.connectedCount = connectedCount
        self.catalogItem = catalogItem
        self.icon = icon
        self.pack = pack
        self.authRung = authRung
        self.verified = verified
        self.implicitlyConnected = implicitlyConnected
        self.contributes = contributes
        self.permissions = permissions
        self.activates = activates
    }

    /// Connected, or ready without a connection of its own.
    public var isReady: Bool { connectedCount > 0 || implicitlyConnected }
}

/// connector-browse.ts ConnectorDetails: what a listing can tell before anything is installed.
public struct ConnectorDetails: Hashable, Sendable {
    public struct Setting: Hashable, Sendable {
        public var name: String
        public var required: Bool
        public var description: String?
    }
    public var triggers: [ConnectorCapability]
    public var actions: [ConnectorCapability]
    public var settings: [Setting]
    public var known: Bool

    public static let empty = ConnectorDetails(triggers: [], actions: [], settings: [], known: false)
}

/// connector-browse.ts ListingSection.
public struct ListingSection: Hashable, Sendable {
    public var category: String
    public var listings: [ConnectorListing]
}

/// connector-browse.ts ConnectionGroup: the saved connections of one connector.
public struct ConnectionGroup: Hashable, Sendable, Identifiable {
    public var connectorId: String
    public var name: String
    public var icon: ConnectorIconData?
    public var version: String?
    public var listing: ConnectorListing?
    public var connections: [SourceConnection]
    public var id: String { connectorId }
}

/// connector-browse.ts, the pure half of the Plugins page.
public enum ConnectorBrowse {
    public static let callableFilter = "Can be called from a step"
    public static let connectedFilter = "Connected"
    static let uncategorized = "Other"

    public struct RungCopy: Hashable, Sendable {
        public var label: String
        public var badge: String
        public var detail: String
    }

    /// AUTH_RUNG.
    public static func rung(_ rung: ConnectorAuthRung) -> RungCopy {
        switch rung {
        case .none: RungCopy(label: "Needs no sign-in", badge: "no sign-in", detail: "Nothing — ready as soon as it is installed.")
        case .cli: RungCopy(label: "Signs in with a CLI", badge: "CLI login", detail: "Signs in with a CLI")
        case .key: RungCopy(label: "Needs a key", badge: "key", detail: "Needs a key")
        case .browser: RungCopy(label: "Signs in through a Vorn window", badge: "browser", detail: "Signs in through a Vorn window")
        case .oauth: RungCopy(label: "Signs in with OAuth", badge: "OAuth", detail: "Signs in with OAuth")
        }
    }

    /// CONNECTOR_KIND.
    public static func kindLabel(_ kind: ConnectorKind) -> String { kind == .connector ? "Connectors" : "Extensions" }
    public static func kindBadge(_ kind: ConnectorKind) -> String? { kind == .extension ? "extension" : nil }

    public static func kindOf(pack: InstalledPack?, catalogItem: CatalogItem?) -> ConnectorKind {
        pack?.kind ?? catalogItem?.kind ?? .connector
    }

    /// matchesListing: whether a staged preview is exactly what the directory already described.
    public static func matches(_ preview: PackPreview, _ listing: ConnectorListing) -> Bool {
        preview.id == listing.id && preview.version == listing.catalogItem?.version
            && preview.auth?.rung != nil && preview.auth?.rung == listing.authRung
            && listing.verified?.version == preview.version && preview.installedVersion == nil
    }

    public static func details(_ listing: ConnectorListing, builtIns: [BuiltInConnector] = []) -> ConnectorDetails {
        if let pack = listing.pack {
            return ConnectorDetails(triggers: pack.triggers.map(\.capability), actions: pack.actions,
                                    settings: pack.env.map { .init(name: $0.name, required: $0.required, description: $0.description) },
                                    known: true)
        }
        if listing.source == .catalog {
            guard let entry = listing.catalogItem, entry.triggers != nil || entry.actions != nil else {
                var details = ConnectorDetails.empty
                details.known = listing.kind == .extension || listing.contributes != nil
                return details
            }
            return ConnectorDetails(triggers: entry.triggers ?? [], actions: entry.actions ?? [],
                                    settings: (entry.env ?? []).map {
                                        .init(name: $0.name, required: $0.required,
                                              description: $0.description?.isEmpty == false ? $0.description : nil)
                                    }, known: true)
        }
        guard let manifest = builtIns.first(where: { $0.id == listing.id })?.manifest else { return .empty }
        return ConnectorDetails(triggers: manifest.triggers, actions: manifest.actions,
                                settings: manifest.auth.map { .init(name: $0.label, required: $0.required) }, known: true)
    }

    private static func packCapabilities(_ pack: InstalledPack) -> [String] {
        (pack.triggers.isEmpty ? [] : ["triggers"]) + (pack.actions.isEmpty ? [] : ["actions"])
    }

    /// buildConnectorListings: everything the directory offers, ready ones first, then by name.
    public static func listings(builtIns: [BuiltInConnector], catalog: [CatalogItem], connections: [SourceConnection],
                                packs: [InstalledPack] = [], mcpServers: [McpServerEntry] = []) -> [ConnectorListing] {
        var counts: [String: Int] = [:]
        var implicit: Set<String> = []
        for conn in connections {
            let id = conn.realConnectorId
            if conn.isImplicit { implicit.insert(id) } else { counts[id, default: 0] += 1 }
        }
        let packFor = { (id: String) in packs.first { $0.id == id } }

        var all: [ConnectorListing] = builtIns.filter { $0.addable != false }.map {
            ConnectorListing(key: $0.id, id: $0.id, name: $0.name, capabilities: $0.capabilities, category: "Built in",
                             source: .builtin, connectedCount: counts[$0.id] ?? 0, implicitlyConnected: implicit.contains($0.id))
        }
        all += catalog.map { entry in
            let pack = packFor(entry.id)
            return ConnectorListing(
                key: "catalog:\(entry.id)", id: entry.id, name: entry.name, description: entry.description,
                capabilities: entry.capabilities, category: entry.category ?? uncategorized,
                kind: kindOf(pack: pack, catalogItem: entry), source: .catalog, keywords: entry.keywords,
                connectedCount: counts[entry.id] ?? 0, catalogItem: entry, icon: entry.icon, pack: pack,
                authRung: pack?.auth?.rung ?? entry.authRung, verified: entry.verified,
                implicitlyConnected: implicit.contains(entry.id), contributes: pack?.contributes ?? entry.contributes,
                permissions: pack?.permissions ?? entry.permissions, activates: pack?.activates ?? entry.activates)
        }
        all += packs.filter { pack in
            !catalog.contains { $0.id == pack.id } && !builtIns.contains { $0.id == pack.id }
        }.map { pack in
            ConnectorListing(key: "installed:\(pack.id)", id: pack.id, name: pack.name, description: pack.description,
                             capabilities: packCapabilities(pack), category: "Installed", kind: kindOf(pack: pack, catalogItem: nil),
                             source: .installed, connectedCount: counts[pack.id] ?? 0, icon: pack.icon, pack: pack,
                             authRung: pack.auth?.rung, implicitlyConnected: implicit.contains(pack.id),
                             contributes: pack.contributes, permissions: pack.permissions, activates: pack.activates)
        }
        all += mcpServers.map { server in
            ConnectorListing(key: "mcp:\(server.id)", id: server.id, name: server.name, description: server.description,
                             capabilities: ["actions"], category: server.category ?? "MCP servers", source: .mcp,
                             mcpServer: server, keywords: server.keywords, connectedCount: counts[server.id] ?? 0,
                             implicitlyConnected: implicit.contains(server.id))
        }
        return all.enumerated().sorted { a, b in
            if a.element.isReady != b.element.isReady { return a.element.isReady }
            let order = a.element.name.localizedCompare(b.element.name)
            return order == .orderedSame ? a.offset < b.offset : order == .orderedAscending
        }.map(\.element)
    }

    /// groupListingsByCategory, in first-seen order.
    public static func sections(_ listings: [ConnectorListing]) -> [ListingSection] {
        var sections: [ListingSection] = []
        for listing in listings {
            if let i = sections.firstIndex(where: { $0.category == listing.category }) {
                sections[i].listings.append(listing)
            } else {
                sections.append(ListingSection(category: listing.category, listings: [listing]))
            }
        }
        return sections
    }

    /// groupConnections: implicit connections are the pack's, not the person's, so they stay out.
    public static func groups(_ connections: [SourceConnection], listings: [ConnectorListing] = []) -> [ConnectionGroup] {
        var groups: [ConnectionGroup] = []
        for conn in connections where !conn.isImplicit {
            let id = conn.realConnectorId
            if let i = groups.firstIndex(where: { $0.connectorId == id }) {
                groups[i].connections.append(conn)
                continue
            }
            let listing = listings.first { $0.id == id }
            let version = listing?.catalogItem?.version
            groups.append(ConnectionGroup(connectorId: id, name: listing?.name ?? id, icon: listing?.catalogItem?.icon ?? conn.icon,
                                          version: version?.isEmpty == false ? version : nil, listing: listing, connections: [conn]))
        }
        return groups
    }

    /// filterConnectorListings: every word must appear somewhere the listing describes itself.
    public static func filter(_ listings: [ConnectorListing], query: String) -> [ConnectorListing] {
        let terms = query.lowercased().split(whereSeparator: \.isWhitespace).map(String.init)
        guard !terms.isEmpty else { return listings }
        return listings.filter { listing in
            var words: [String] = [listing.name, listing.description ?? "", listing.category]
            words += listing.capabilities
            words += listing.keywords
            words += listing.contributes?.all.map(\.title) ?? []
            words += (listing.permissions ?? []).map { ExtensionCopy.permission($0).phrase }
            let haystack = words.joined(separator: " ").lowercased()
            return terms.allSatisfy { haystack.contains($0) }
        }
    }

    public static func categories(_ listings: [ConnectorListing]) -> [String] {
        var result = Array(Set(listings.map(\.category))).sorted()
        if listings.contains(where: { $0.capabilities.contains("actions") }) { result.append(callableFilter) }
        if listings.contains(where: { $0.connectedCount > 0 }) { result.append(connectedFilter) }
        return result
    }

    public static func authRungs(_ listings: [ConnectorListing]) -> [ConnectorAuthRung] {
        ConnectorAuthRung.allCases.filter { rung in listings.contains { $0.authRung == rung } }
    }

    public static func kinds(_ listings: [ConnectorListing]) -> [ConnectorKind] {
        ConnectorKind.allCases.filter { kind in listings.contains { $0.kind == kind } }
    }

    public static func filter(_ listings: [ConnectorListing], kind: ConnectorKind?) -> [ConnectorListing] {
        guard let kind else { return listings }
        return listings.filter { $0.kind == kind }
    }

    public static func filter(_ listings: [ConnectorListing], rung: ConnectorAuthRung?) -> [ConnectorListing] {
        guard let rung else { return listings }
        return listings.filter { $0.authRung == rung }
    }

    public static func filter(_ listings: [ConnectorListing], category: String?) -> [ConnectorListing] {
        guard let category, !category.isEmpty else { return listings }
        if category == callableFilter { return listings.filter { $0.capabilities.contains("actions") } }
        if category == connectedFilter { return listings.filter { $0.connectedCount > 0 } }
        return listings.filter { $0.category == category }
    }

    /// describeCatalogAge; `fetchedAt` and `now` are epoch milliseconds.
    public static func catalogAge(fetchedAt: Double?, now: Double = Date().timeIntervalSince1970 * 1000) -> String {
        guard let fetchedAt else { return "Showing the connectors that shipped with Vorn" }
        let minutes = Int(((now - fetchedAt) / 60_000).rounded(.down))
        if minutes < 1 { return "Updated just now" }
        if minutes < 60 { return "Updated \(ExtensionCopy.count(minutes, "minute")) ago" }
        let hours = minutes / 60
        if hours < 24 { return "Updated \(ExtensionCopy.count(hours, "hour")) ago" }
        return "Updated \(ExtensionCopy.count(hours / 24, "day")) ago"
    }
}
