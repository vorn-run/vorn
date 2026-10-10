import SwiftUI

/// A project's chosen glyph and colour, from ProjectPicker.tsx; a folder when it has none.
public struct ProjectIcon: View {
    let icon: String?
    let color: String?
    let size: CGFloat

    public init(icon: String?, color: String?, size: CGFloat = 13) {
        self.icon = icon
        self.color = color
        self.size = size
    }

    static let glyphs: [String: LucideGlyph] = [
        "Folder": .folder, "FolderGit2": .folderGit2, "Code": .code, "Globe": .globe,
        "Database": .database, "Server": .server, "Smartphone": .smartphone, "Package": .package,
        "FileCode": .fileCode, "Terminal": .terminal, "Cpu": .cpu, "Cloud": .cloud, "Shield": .shield,
        "Zap": .zap, "Gamepad2": .gamepad2, "Music": .music, "Image": .image, "BookOpen": .bookOpen,
        "FlaskConical": .flaskConical, "Rocket": .rocket,
    ]

    public var body: some View {
        LucideIcon(icon.flatMap { Self.glyphs[$0] } ?? .folder, size: size, strokeWidth: 1.5)
            .foregroundStyle(color.flatMap(Color.init(cssHex:)) ?? Color(hex: 0x6B7280))
    }
}
