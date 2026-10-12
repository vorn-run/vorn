import SwiftUI
import VornCore
import VornTerminals
import VornUI

/// FilesCard.tsx: a session's tree, and the files open from it as tabs.
public struct FilesCard: View {
    @Bindable var panes: PanesModel
    let sessionID: String
    let services: PaneServices
    @State private var width: CGFloat = 0
    @State private var dragRatio: Double?

    public init(panes: PanesModel, sessionID: String, services: PaneServices) {
        self.panes = panes
        self.sessionID = sessionID
        self.services = services
    }

    private static func base(_ p: String) -> String { (p as NSString).lastPathComponent }
    private static func parent(_ p: String) -> String { ((p as NSString).deletingLastPathComponent as NSString).lastPathComponent }

    public var body: some View {
        if let session = services.session(sessionID), let pane = panes.filesPanes[sessionID] {
            let cwd = session.worktreePath ?? session.projectPath
            let paneID = PaneID.files(sessionID)
            let hasTabs = !pane.tabs.isEmpty
            let narrow = width > 0 && width < 480
            let treeShown = !hasTabs || pane.treeVisible
            let fileShown = hasTabs && !(narrow && pane.treeVisible)
            let ratio = dragRatio ?? panes.treeRatio
            VStack(spacing: 0) {
                PaneTabStrip(tabs: tabs(pane), activeID: pane.active, emptyTitle: "Files",
                             onSelect: { p in
                                 panes.setActiveFileTab(sessionID, p)
                                 if narrow { panes.setFileTreeVisible(sessionID, false) }
                             },
                             onClose: { panes.requestCloseFileTab(sessionID, $0) },
                             onDoubleClickTab: { panes.pinFileTab(sessionID, $0) },
                             menuFooter: [("Close saved tabs", { panes.closeSavedFileTabs(sessionID) })],
                             onDoubleClick: { panes.toggleMaximized(paneID) }) {
                    PaneIconButton(icon: .listTree, help: treeShown ? "Hide file tree" : "Show file tree", active: treeShown,
                                   disabled: !hasTabs, dimsWhenDisabled: false) { panes.toggleFileTree(sessionID) }
                } trailing: {
                    PaneControls(panes: panes, paneID: paneID, title: "Files",
                                 onPopOut: pane.active.map { a in { panes.popOutTab(sessionID, a) } }, popOutLabel: "this file",
                                 onClose: close)
                }
                GeometryReader { geo in
                    HStack(spacing: 0) {
                        if treeShown {
                            FileTree(model: FileTreeModel(client: services.client, cwd: cwd, remoteHostId: session.remoteHostId),
                                     selectedFile: pane.active,
                                     onSelect: { open($0, pin: false, narrow: narrow) },
                                     onPin: { open($0, pin: true, narrow: narrow) },
                                     onPopOut: { panes.promoteFile(sessionID, $0) })
                                .id(cwd)
                                .frame(width: fileShown ? geo.size.width * ratio : geo.size.width)
                        }
                        if treeShown && fileShown {
                            SplitDivider(axis: .x, label: "Resize file tree", origin: { geo.frame(in: .global).minX },
                                         extent: { geo.size.width },
                                         onChange: { dragRatio = $0 }, onCommit: { panes.treeRatio = $0; dragRatio = nil })
                        }
                        ZStack {
                            ForEach(pane.tabs, id: \.self) { path in
                                let key = PaneID.fileTab(sessionID, path)
                                FilePanel(cwd: cwd, filePath: path, remoteHostId: session.remoteHostId, draftKey: key,
                                          services: services, onDirty: { panes.setDirty(key, $0) }) { EmptyView() }
                                    .opacity(fileShown && path == pane.active ? 1 : 0)
                                    .allowsHitTesting(fileShown && path == pane.active)
                            }
                        }
                        .frame(maxWidth: fileShown ? .infinity : 0)
                        .clipped()
                    }
                    .onAppear { width = geo.size.width }
                    .onChange(of: geo.size.width) { _, w in width = w }
                }
                .padding(.top, 2)
            }
            .background(Theme.surfacePanel)
            .onChange(of: pane.preview.map { panes.isDirty(PaneID.fileTab(sessionID, $0)) } ?? false) { _, dirty in
                if dirty, let p = pane.preview { panes.pinFileTab(sessionID, p) }
            }
        }
    }

    private func open(_ path: String, pin: Bool, narrow: Bool) {
        panes.openFileTab(sessionID, path, pin: pin)
        if narrow { panes.setFileTreeVisible(sessionID, false) }
    }

    private func close() {
        if let pane = panes.filesPanes[sessionID], panes.confirmDiscard(pane.tabs.map { PaneID.fileTab(sessionID, $0) }) {
            panes.closeFilesPane(sessionID)
        }
    }

    private func tabs(_ pane: FilesPaneState) -> [PaneTab] {
        let names = pane.tabs.map(Self.base)
        return pane.tabs.enumerated().map { i, path in
            let shared = names.filter { $0 == names[i] }.count > 1
            return PaneTab(id: path, name: names[i], detail: shared ? Self.parent(path) : nil, title: path,
                           icon: AnyView(FileTypeIcon(name: names[i], size: 12)), italic: pane.preview == path,
                           dirty: panes.isDirty(PaneID.fileTab(sessionID, path)))
        }
    }
}

/// EditorCard.tsx: a file popped out of its session, as a card of its own in the grid.
public struct EditorCard: View {
    @Bindable var panes: PanesModel
    let cardID: String
    let services: PaneServices
    @State private var hover = false

    public init(panes: PanesModel, cardID: String, services: PaneServices) {
        self.panes = panes
        self.cardID = cardID
        self.services = services
    }

    public var body: some View {
        if let card = panes.editorCards[cardID], let session = services.session(card.sessionId) {
            let name = (card.filePath as NSString).lastPathComponent
            FilePanel(cwd: session.worktreePath ?? session.projectPath, filePath: card.filePath, remoteHostId: session.remoteHostId,
                      draftKey: cardID, services: services, onDirty: { panes.setDirty(cardID, $0) }) {
                Text("·").foregroundStyle(Theme.inkGhost)
                Text(session.projectName).font(.ui(11)).foregroundStyle(Theme.inkSecondary).lineLimit(1).frame(maxWidth: 140)
                    .fixedSize()
                BranchChip(session: session, git: services.cardActions.git)
                PromotedCardControls(title: name, onMinimize: { services.minimize(cardID) },
                                     onReturn: { panes.returnCard(cardID) }, onClose: { panes.closeCard(cardID) })
            }
            .background(Theme.surfacePanel)
            .overlay(Rectangle().strokeBorder(hover ? Theme.white(0.12) : Theme.hairline, lineWidth: 1))
            .onHover { hover = $0 }
        }
    }
}

/// TerminalsCard.tsx: the shells a session holds beside its agent, one tab in front.
public struct TerminalsCard: View {
    @Bindable var panes: PanesModel
    let sessionID: String
    let services: PaneServices
    @State private var focusTarget: String?

    public init(panes: PanesModel, sessionID: String, services: PaneServices) {
        self.panes = panes
        self.sessionID = sessionID
        self.services = services
    }

    public var body: some View {
        if services.session(sessionID) != nil, let pane = panes.terminalsPanes[sessionID], let activeID = pane.activeID {
            let paneID = PaneID.terminals(sessionID)
            let names = pane.terminals.map { id in (id, services.session(id)?.title ?? id) }
            VStack(spacing: 0) {
                PaneTabStrip(tabs: names.map { PaneTab(id: $0.0, name: $0.1) }, activeID: activeID,
                             onSelect: { id in
                                 if let i = pane.terminals.firstIndex(of: id) { panes.setActivePanelTerminal(sessionID, index: i) }
                                 focusTarget = id
                             },
                             onClose: { services.closeTerminal($0) },
                             tabAction: { tab in
                                 AnyView(Button { panes.releaseTerminal(tab.id) } label: {
                                     LucideIcon(.squareArrowOutUpRight, size: 10, strokeWidth: 2.5).foregroundStyle(Theme.gray600).padding(2)
                                 }
                                 .buttonStyle(.plain)
                                 .help("Open as its own terminal")
                                 .accessibilityLabel("Open \(tab.name) as its own terminal"))
                             },
                             onAdd: { services.addPanelTerminal(sessionID) }, addLabel: "New terminal in this session") {
                    PaneControls(panes: panes, paneID: paneID, title: "Terminals", onClose: closeAll)
                }
                .background(Theme.surfacePanel)
                Group {
                    if let engine = services.engine, let s = services.session(activeID) {
                        SessionStage(session: s, engine: engine, focusRequest: focusTarget == activeID ? 1 : 0,
                                     actions: services.cardActions, context: CardContext(), showsStatusBar: false)
                            .id(activeID)
                    } else {
                        Theme.surfaceSunken
                    }
                }
                .padding(.top, 2)
                .background(Theme.surfaceSunken)
            }
        }
    }

    /// closeTerminalsPanel: every shell in it ends with it.
    private func closeAll() {
        let held = panes.terminalsPanes[sessionID]?.terminals ?? []
        panes.closeTerminalsPane(sessionID)
        for id in held { services.closeTerminal(id) }
    }
}

/// PaneColumn.tsx: the panes a session owns, stacked, with dividers; a maximized one takes the column.
public struct PaneColumn: View {
    @Bindable var panes: PanesModel
    let sessionID: String
    let services: PaneServices
    @State private var dragWeights: [Double]?

    public init(panes: PanesModel, sessionID: String, services: PaneServices) {
        self.panes = panes
        self.sessionID = sessionID
        self.services = services
    }

    public var body: some View {
        let entries = panes.columnEntries(sessionID)
        let maxIndex = entries.firstIndex { $0.id == panes.maximizedPaneId }
        let weights = dragWeights ?? SplitRatio.weights(panes.cardSplits[sessionID]?.panes, count: entries.count)
        GeometryReader { geo in
            VStack(spacing: 0) {
                ForEach(Array(entries.enumerated()), id: \.element.id) { i, entry in
                    if maxIndex == nil || maxIndex == i {
                        if i > 0 && maxIndex == nil {
                            SplitDivider(axis: .y, label: "Resize \(entry.kind.rawValue) panel", origin: { geo.frame(in: .global).minY },
                                         extent: { geo.size.height },
                                         onChange: { dragWeights = SplitRatio.resize(weights, index: i - 1, ratio: $0) },
                                         onCommit: {
                                             panes.setCardSplit(sessionID, panes: SplitRatio.resize(weights, index: i - 1, ratio: $0))
                                             dragWeights = nil
                                         })
                        }
                        view(entry)
                            .frame(height: maxIndex != nil ? geo.size.height
                                : max(0, (geo.size.height - CGFloat(entries.count - 1)) * weights[i]))
                    }
                }
            }
        }
    }

    @ViewBuilder private func view(_ e: ColumnEntry) -> some View {
        switch e.kind {
        case .files: FilesCard(panes: panes, sessionID: sessionID, services: services)
        case .terminals: TerminalsCard(panes: panes, sessionID: sessionID, services: services)
        }
    }
}
