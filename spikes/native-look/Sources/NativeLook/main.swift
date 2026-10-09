import AppKit
import SwiftUI

let windowSize = CGSize(width: 1000, height: 600)

struct NativeLookApp: App {
    var body: some Scene {
        WindowGroup {
            MainScreen()
                .frame(minWidth: 800, minHeight: 500)
                .ignoresSafeArea()
                .background(WindowConfigurator())
        }
        .windowStyle(.hiddenTitleBar)
        .defaultSize(windowSize)
    }
}

/// Gives the hidden titlebar the height of the 40pt top bar, so the native
/// window buttons sit centred on it as they do in the Electron window.
private struct WindowConfigurator: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        DispatchQueue.main.async {
            guard let window = view.window else { return }
            window.titleVisibility = .hidden
            window.titlebarAppearsTransparent = true
            window.styleMask.insert(.fullSizeContentView)
            window.toolbar = NSToolbar(identifier: "main")
            window.toolbarStyle = .unifiedCompact
            window.backgroundColor = NSColor(red: 0x0D / 255, green: 0x0D / 255, blue: 0x0F / 255, alpha: 1)
            window.appearance = NSAppearance(named: .darkAqua)
        }
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {}
}

@MainActor
func render(to path: String) -> Int32 {
    _ = NSApplication.shared
    let content = MainScreen(fauxChrome: true)
        .frame(width: windowSize.width, height: windowSize.height)
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .strokeBorder(Color.white.opacity(0.1), lineWidth: 0.5)
        )
    let renderer = ImageRenderer(content: content)
    renderer.scale = 2
    renderer.isOpaque = false
    guard let cgImage = renderer.cgImage else {
        FileHandle.standardError.write(Data("render failed\n".utf8))
        return 1
    }
    let rep = NSBitmapImageRep(cgImage: cgImage)
    guard let png = rep.representation(using: .png, properties: [:]) else { return 1 }
    do {
        try png.write(to: URL(fileURLWithPath: path))
    } catch {
        FileHandle.standardError.write(Data("\(error)\n".utf8))
        return 1
    }
    print("wrote \(path) (\(cgImage.width)x\(cgImage.height))")
    return 0
}

let args = CommandLine.arguments
if let i = args.firstIndex(of: "--render"), i + 1 < args.count {
    let code = MainActor.assumeIsolated { render(to: args[i + 1]) }
    exit(code)
}
NativeLookApp.main()
