import AppKit

/// What a key press in a terminal does before the grid's key encoder sees it.
enum TerminalKey: Equatable {
    /// The encoder's: the input method composes, then vornd encodes for the program's modes.
    case encode
    /// Bytes for the program as they are.
    case send(String)
    /// A page of history, up or down.
    case page(up: Bool)
}

/// What a paste sends: the clipboard's text, or Ctrl-V when it holds only an
/// image, which agent CLIs answer by reading the image themselves.
enum TerminalPaste: Equatable {
    case text(String)
    case ctrlV
    case nothing
}

/// The terminal's own commands, taken before the app's menus.
enum TerminalShortcut: Equatable {
    case find, findNext, findPrevious, clear
}

/// The Mac editing chords a shell does not know as keys, sent as the readline
/// and zle bytes they mean (as the desktop app's terminal sends them), and Option as Meta.
enum TerminalKeys {
    static let esc = "\u{1b}"

    static func route(keyCode: UInt16, modifiers: NSEvent.ModifierFlags, unmodified: String?,
                      alternateScreen: Bool, optionAsMeta: Bool) -> TerminalKey {
        let mods = modifiers.intersection([.shift, .option, .control, .command])
        switch (mods, keyCode) {
        case (.shift, 116) where !alternateScreen: return .page(up: true)
        case (.shift, 121) where !alternateScreen: return .page(up: false)
        case (.option, 123): return .send(esc + "b")          // word left
        case (.option, 124): return .send(esc + "f")          // word right
        case (.option, 51): return .send(esc + "\u{7f}")      // delete word before
        case (.command, 123): return .send("\u{01}")          // line start
        case (.command, 124): return .send("\u{05}")          // line end
        case (.command, 51): return .send("\u{15}")           // delete to line start
        default: break
        }
        guard optionAsMeta, mods.contains(.option), mods.isDisjoint(with: [.command, .control]),
              let s = unmodified, !s.isEmpty, s.unicodeScalars.allSatisfy(isPrintable)
        else { return .encode }
        return .send(esc + s)
    }

    /// ⌘F find, ⌘G and ⇧⌘G next and previous match, ⌥⌘K clear (⌘K stays the command palette).
    static func shortcut(keyCode: UInt16, modifiers: NSEvent.ModifierFlags) -> TerminalShortcut? {
        switch (modifiers.intersection([.shift, .option, .control, .command]), keyCode) {
        case (.command, 3): .find
        case (.command, 5): .findNext
        case ([.command, .shift], 5): .findPrevious
        case ([.command, .option], 40): .clear
        default: nil
        }
    }

    /// Not a control character, nor one of the private-use characters AppKit gives named keys.
    private static func isPrintable(_ c: Unicode.Scalar) -> Bool {
        c.value >= 0x20 && c.value != 0x7f && !(0xF700...0xF8FF).contains(c.value)
    }

    /// Paths as a shell reads them, space separated, with a trailing space to type on after.
    static func quotedPaths(_ urls: [URL]) -> String {
        urls.map { shellQuoted($0.path) }.joined(separator: " ") + " "
    }

    static func shellQuoted(_ s: String) -> String {
        let plain = CharacterSet(charactersIn:
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_@%+=:,./-")
        if !s.isEmpty, s.unicodeScalars.allSatisfy(plain.contains) { return s }
        return "'" + s.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }

    static func paste(text: String?, hasImage: Bool) -> TerminalPaste {
        if let text, !text.isEmpty { return .text(text) }
        return hasImage ? .ctrlV : .nothing
    }

    /// The image types a paste or a drop takes as an image.
    static let imageTypes: [NSPasteboard.PasteboardType] = [.png, .tiff]

    /// A dropped image as PNG: as it is when it is one, else drawn from whatever image type it holds.
    static func png(from pb: NSPasteboard) -> Data? {
        if let png = pb.data(forType: .png) { return png }
        guard let tiff = pb.data(forType: .tiff) ?? NSImage(pasteboard: pb)?.tiffRepresentation else { return nil }
        return NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:])
    }

    /// Writes `png` to a new file in `dir`, named for the time.
    static func saveImage(_ png: Data, in dir: URL, now: Date = Date()) throws -> URL {
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let stamp = Int(now.timeIntervalSince1970 * 1000)
        for n in 0... {
            let url = dir.appendingPathComponent(n == 0 ? "image-\(stamp).png" : "image-\(stamp)-\(n).png")
            do {
                try png.write(to: url, options: .withoutOverwriting)
                return url
            } catch CocoaError.fileWriteFileExists {
                continue
            }
        }
        fatalError("a free name exists")
    }

    /// Where dropped images are kept for the program to read.
    static var imageDirectory: URL {
        FileManager.default.temporaryDirectory.appendingPathComponent("vorn-images", isDirectory: true)
    }
}
