import SwiftUI

/// ConnectorIcon.tsx: a package's own glyph, else GitHub, Linear and the plug drawn, HTTP a globe, else a link.
public struct ConnectorIcon: View {
    let connectorId: String
    let viewBox: String?
    let paths: [String]?
    let packaged: Bool
    let size: CGFloat
    let color: Color

    public init(connectorId: String, viewBox: String? = nil, paths: [String]? = nil, packaged: Bool = false,
                size: CGFloat = 12, color: Color = Theme.gray500) {
        self.connectorId = connectorId
        self.viewBox = viewBox
        self.paths = paths
        self.packaged = packaged
        self.size = size
        self.color = color
    }

    /// The square a `viewBox` string spans, read from its width and height.
    static func box(_ viewBox: String?) -> CGFloat {
        let parts = (viewBox ?? "").split(whereSeparator: { $0 == " " || $0 == "," }).compactMap { Double($0) }
        return parts.count == 4 ? CGFloat(max(parts[2], parts[3])) : 24
    }

    public var body: some View {
        if let paths, !paths.isEmpty {
            FilledPaths(paths: paths, viewBox: Self.box(viewBox)).fill(color).frame(width: size, height: size)
        } else {
            switch connectorId {
            case "github": FilledPaths(paths: [BrandPaths.github], viewBox: 16).fill(color).frame(width: size, height: size)
            case "linear": FilledPaths(paths: BrandPaths.linear).fill(color).frame(width: size, height: size)
            case "http": LucideIcon(.globe, size: size).foregroundStyle(color)
            case "mcp", "sdk": plug
            default:
                if packaged { plug } else { LucideIcon(.externalLink, size: size).foregroundStyle(color) }
            }
        }
    }

    private var plug: some View {
        FilledPaths(paths: BrandPaths.mcp).fill(color, style: FillStyle(eoFill: true)).frame(width: size, height: size)
    }
}
