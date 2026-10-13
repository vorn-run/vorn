import SwiftUI
import VornCore
import VornTerminals
import VornUI

/// What the panes ask of the shell: the connection, the sessions, terminals and toasts.
public struct PaneServices {
    public var client: VornClient
    public var session: (String) -> TerminalSession?
    public var engine: TerminalEngine?
    public var toasts: ToastCenter
    public var cardActions: CardActions
    /// Starts a shell in the owner's tree and claims it for the owner's panel.
    public var addPanelTerminal: (String) -> Void
    public var closeTerminal: (String) -> Void
    public var minimize: (String) -> Void
    public var write: (String, String) -> Void
    public var reloadGit: (String) -> Void

    public init(client: VornClient, session: @escaping (String) -> TerminalSession?, engine: TerminalEngine?,
                toasts: ToastCenter, cardActions: CardActions, addPanelTerminal: @escaping (String) -> Void,
                closeTerminal: @escaping (String) -> Void, minimize: @escaping (String) -> Void,
                write: @escaping (String, String) -> Void, reloadGit: @escaping (String) -> Void) {
        self.client = client
        self.session = session
        self.engine = engine
        self.toasts = toasts
        self.cardActions = cardActions
        self.addPanelTerminal = addPanelTerminal
        self.closeTerminal = closeTerminal
        self.minimize = minimize
        self.write = write
        self.reloadGit = reloadGit
    }
}

/// One open file (FilePanel in FileTreeExplorer.tsx): path strip, find, save, discard, kept drafts and the moved-file banner.
@MainActor
@Observable
final class FileBuffer {
    enum ReadOnly { case binary, unreadable, truncated }

    let client: VornClient
    let filePath: String
    let remoteHostId: String?
    let draftKey: String?
    var content: String?
    var loading = true
    var readOnly: ReadOnly?
    var draft = ""
    var ready = false
    var saving = false
    var saveError: String?
    var base: FileStamp?
    var conflict = false

    init(client: VornClient, filePath: String, remoteHostId: String?, draftKey: String?) {
        self.client = client
        self.filePath = filePath
        self.remoteHostId = remoteHostId
        self.draftKey = draftKey
    }

    var editable: Bool { readOnly == nil && content != nil && !loading }
    var dirty: Bool { ready && editable && draft != content }

    func load() async {
        loading = true
        do {
            if let text = try await client.readFile(filePath, remoteHostId: remoteHostId) {
                readOnly = isTruncatedRead(text) ? .truncated : nil
                content = text
            } else {
                let stamp = try? await client.fileStamp(filePath, remoteHostId: remoteHostId)
                readOnly = stamp == nil ? .unreadable : .binary
            }
        } catch {
            readOnly = .unreadable
        }
        loading = false
        guard editable, let content else { return }
        let kept = draftKey.flatMap { EditorDrafts.shared.read($0, filePath: filePath) }
        ready = true
        guard let kept, kept.text != content else {
            if kept != nil, let draftKey { EditorDrafts.shared.forget(draftKey) }
            draft = content
            await stampBase()
            return
        }
        base = kept.base
        draft = kept.text
        let current = try? await client.fileStamp(filePath, remoteHostId: remoteHostId)
        conflict = FileStamp.hasMoved(kept.base, current)
    }

    func stampBase() async {
        base = try? await client.fileStamp(filePath, remoteHostId: remoteHostId)
    }

    /// Keeps the draft after typing settles (400ms), or forgets it once it matches the file.
    func keepDraft() async {
        guard let draftKey, ready, let content else { return }
        if draft == content {
            EditorDrafts.shared.forget(draftKey)
            return
        }
        try? await Task.sleep(for: .milliseconds(400))
        guard !Task.isCancelled else { return }
        EditorDrafts.shared.write(draftKey, filePath: filePath, text: draft, base: base)
    }

    /// Writes the buffer; `force` skips the moved-file check, which only the conflict banner passes.
    func save(force: Bool = false) async {
        guard dirty else { return }
        saving = true
        saveError = nil
        defer { saving = false }
        do {
            if !force {
                let current = try await client.fileStamp(filePath, remoteHostId: remoteHostId)
                if FileStamp.hasMoved(base, current) {
                    conflict = true
                    return
                }
            }
            if let err = try await client.writeFile(filePath, content: draft, remoteHostId: remoteHostId) {
                saveError = err
                return
            }
            if let draftKey { EditorDrafts.shared.forget(draftKey) }
            if isTruncatedRead(draft) { readOnly = .truncated }
            content = draft
            conflict = false
            base = nil
            await stampBase()
        } catch {
            saveError = error.localizedDescription
        }
    }

    func discard() {
        guard let content else { return }
        if let draftKey { EditorDrafts.shared.forget(draftKey) }
        draft = content
        saveError = nil
        conflict = false
    }

    func takeDisk() async {
        guard let next = try? await client.readFile(filePath, remoteHostId: remoteHostId) else {
            saveError = "The file could not be read."
            return
        }
        if let draftKey { EditorDrafts.shared.forget(draftKey) }
        content = next
        draft = next
        conflict = false
        base = nil
        await stampBase()
    }
}

struct FilePanel<Controls: View>: View {
    let cwd: String
    let filePath: String
    let remoteHostId: String?
    let draftKey: String?
    let services: PaneServices
    let onDirty: (Bool) -> Void
    @ViewBuilder var controls: () -> Controls
    @State private var buffer: FileBuffer?
    @State private var findOpen = false
    @State private var findQuery = ""
    @State private var findCount = 0
    @State private var findIndex = 0
    @FocusState private var findFocused: Bool

    private var fileName: String { (filePath as NSString).lastPathComponent }

    private var relPath: String {
        if !cwd.isEmpty && filePath.hasPrefix(cwd) {
            let rel = String(filePath.dropFirst(cwd.count)).trimmingCharacters(in: CharacterSet(charactersIn: "/\\"))
            return rel.isEmpty ? fileName : rel
        }
        return filePath
    }

    var body: some View {
        let b = buffer
        VStack(spacing: 0) {
            HStack(spacing: 4) {
                FileTypeIcon(name: fileName, size: 12)
                Text(relPath).font(.system(size: 11, design: .monospaced)).foregroundStyle(Theme.gray400)
                    .lineLimit(1).truncationMode(.head).frame(maxWidth: .infinity, alignment: .trailing).padding(.leading, 4)
                    .help(filePath)
                if b?.dirty == true {
                    Circle().fill(Theme.amber400).frame(width: 6, height: 6).padding(.horizontal, 4).help("Unsaved changes")
                }
                let canFind = b?.content != nil && b?.loading == false
                PaneIconButton(icon: .search, help: "Find in file", active: findOpen, disabled: !canFind) {
                    if canFind { findOpen.toggle() }
                }
                if b?.dirty == true {
                    PaneIconButton(icon: .save, help: b?.saving == true ? "Saving…  ⌘S" : "Save  ⌘S", disabled: b?.saving == true) {
                        Task { await b?.save() }
                    }
                    PaneIconButton(icon: .undo2, help: "Discard changes") {
                        if PanesModel.askDiscard() { b?.discard() }
                    }
                }
                controls()
            }
            .padding(.leading, 8)
            .padding(.trailing, 4)
            .padding(.vertical, 2)
            .background(Theme.surfacePanel)

            if findOpen, b?.content != nil { findBar }

            if b?.conflict == true {
                HStack(spacing: 12) {
                    Text("This file changed on disk while your edit was open.").frame(maxWidth: .infinity, alignment: .leading)
                    link("Save mine anyway") { Task { await b?.save(force: true) } }
                    link("Discard mine") { Task { await b?.takeDisk() } }
                    link("Keep editing") { b?.conflict = false }
                }
                .font(.ui(11)).foregroundStyle(Theme.bronzo)
                .padding(.horizontal, 12).padding(.vertical, 6)
                .background(Theme.bronzo.opacity(0.1))
                .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
            }
            if b?.readOnly == .truncated {
                Text("Too large to edit here. Showing the first part, read-only.").font(.ui(11)).foregroundStyle(Theme.inkFaint)
                    .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 12).padding(.vertical, 4)
                    .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
            }
            body(b)
            if let err = b?.saveError {
                Text(err).font(.ui(11)).foregroundStyle(Theme.danger)
                    .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 12).padding(.vertical, 4)
                    .background(Theme.danger.opacity(0.1))
                    .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
            }
        }
        .task(id: filePath) {
            let fresh = FileBuffer(client: services.client, filePath: filePath, remoteHostId: remoteHostId, draftKey: draftKey)
            buffer = fresh
            findOpen = false
            findQuery = ""
            findIndex = 0
            await fresh.load()
        }
        .task(id: b?.draft) { await b?.keepDraft() }
        .onChange(of: b?.dirty ?? false, initial: true) { _, d in onDirty(d) }
        .onDisappear { onDirty(false) }
    }

    @ViewBuilder private func body(_ b: FileBuffer?) -> some View {
        if b == nil || b?.loading == true {
            SpinningGlyph(size: 16).foregroundStyle(Theme.gray500).frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let b, b.readOnly == .binary || b.readOnly == .unreadable {
            Text(b.readOnly == .binary ? "Binary file — preview unavailable" : "This file could not be read")
                .font(.ui(13)).foregroundStyle(Theme.gray600).frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let b, b.editable, b.ready {
            CodeEditor(text: Binding(get: { b.draft }, set: { b.draft = $0 }), fileName: fileName,
                       findQuery: findOpen ? findQuery : "", findIndex: findIndex, onMatches: matches) {
                Task { await b.save() }
            }
        } else if let b, let content = b.content {
            CodeEditor(text: .constant(Self.capped(content)), fileName: fileName, readOnly: true,
                       findQuery: findOpen ? findQuery : "", findIndex: findIndex, onMatches: matches)
        } else {
            Color.clear
        }
    }

    /// ReadView: the first 2000 lines of a file too large to edit.
    static func capped(_ content: String) -> String {
        let lines = content.components(separatedBy: "\n")
        guard lines.count > 2000 else { return content }
        return lines.prefix(2000).joined(separator: "\n")
    }

    private func matches(_ count: Int) {
        DispatchQueue.main.async {
            findCount = count
            findIndex = count == 0 ? 0 : min(findIndex, count - 1)
        }
    }

    private var findBar: some View {
        HStack(spacing: 8) {
            LucideIcon(.search, size: 12).foregroundStyle(Theme.gray500)
            TextField("", text: $findQuery, prompt: Text("Find in file").foregroundColor(Theme.gray600))
                .textFieldStyle(.plain).font(.system(size: 13, design: .monospaced)).foregroundStyle(Theme.gray200)
                .focused($findFocused)
                .onChange(of: findQuery) { findIndex = 0 }
                .onSubmit { cycle(NSEvent.modifierFlags.contains(.shift) ? -1 : 1) }
                .onExitCommand { findOpen = false; findQuery = "" }
            Text(findCount == 0 ? "0/0" : "\(findIndex + 1)/\(findCount)").font(.ui(11).monospacedDigit()).foregroundStyle(Theme.gray500)
            arrow("↑", "Previous (Shift+Enter)") { cycle(-1) }
            arrow("↓", "Next (Enter)") { cycle(1) }
            IconButton(.x, size: 12, padding: 2, color: Theme.gray500, help: "Close (Esc)") { findOpen = false; findQuery = "" }
        }
        .font(.ui(11))
        .padding(.horizontal, 12)
        .padding(.vertical, 4)
        .background(Theme.surfacePanel)
        .onAppear { findFocused = true }
    }

    private func cycle(_ d: Int) {
        guard findCount > 0 else { return }
        findIndex = (findIndex + d + findCount) % findCount
    }

    private func arrow(_ s: String, _ help: String, action: @escaping () -> Void) -> some View {
        Button(action: action) { Text(s).foregroundStyle(Theme.gray500).padding(.horizontal, 4) }
            .buttonStyle(.plain).disabled(findCount == 0).opacity(findCount == 0 ? 0.4 : 1).help(help)
    }

    private func link(_ s: String, action: @escaping () -> Void) -> some View {
        Button(action: action) { Text(s).underline().foregroundStyle(Theme.gray300) }.buttonStyle(.plain)
    }
}
