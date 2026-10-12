import AppKit
import SwiftUI

/// OpenInButton.tsx's editor and app marks, drawn in the current colour.
public struct IDEIcon: View {
    let id: String
    let size: CGFloat

    public init(_ id: String, size: CGFloat = 14) {
        self.id = id
        self.size = size
    }

    static let files = ["vscode": "vscode", "vscode-insiders": "vscode", "cursor": "cursor", "windsurf": "windsurf", "zed": "zed",
                        "sublime": "sublime", "webstorm": "webstorm", "intellij": "intellij", "xcode": "xcode",
                        "terminal": "terminal", "finder": "finder"]

    @MainActor static var cache: [String: NSImage] = [:]

    @MainActor static func image(_ id: String) -> NSImage? {
        guard let name = files[id] else { return nil }
        if let hit = cache[name] { return hit }
        guard let url = Bundle.module.url(forResource: "ide-\(name)", withExtension: "svg"),
              let img = NSImage(contentsOf: url) else { return nil }
        img.isTemplate = true
        cache[name] = img
        return img
    }

    public var body: some View {
        if let img = Self.image(id) {
            Image(nsImage: img).renderingMode(.template).resizable().aspectRatio(contentMode: .fit).frame(width: size, height: size)
        }
    }
}
