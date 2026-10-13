import SwiftUI
import VornCore
import VornUI

/// What a card's controls ask the app to do.
public struct CardActions {
    public var select: (String) -> Void = { _ in }
    public var expand: (String) -> Void = { _ in }
    public var minimize: (String) -> Void = { _ in }
    public var close: (String) -> Void = { _ in }
    public var rename: (String, String) -> Void = { _, _ in }
    /// Opens the card's menu at a point in window space (top-left origin).
    public var contextMenu: (String, CGPoint) -> Void = { _, _ in }
    public var browseFiles: (String) -> Void = { _ in }
    public var addTerminal: (String) -> Void = { _ in }
    public var resume: (String) -> Void = { _ in }
    /// Writes bytes to the session's terminal.
    public var write: (String, String) -> Void = { _, _ in }
    /// Starts an agent on a prompt from a shell's composer.
    public var prompt: (String, String) -> Void = { _, _ in }
    public var showChanges: (String) -> Void = { _ in }
    /// Branch switching and opening the tree in an editor, from the status bar.
    public var git = GitActions()
    /// F2: the session whose name field should open, cleared through `endRename`.
    public var renameRequest: String?
    public var endRename: () -> Void = {}

    public init() {}
}

/// Which surface a card is drawn on.
public enum CardVariant: Sendable {
    /// A cell in the grid.
    case mini
    /// The focused stage, filling the content area.
    case focused
}

/// What the card shows besides the session: why it ended, its tree's changes, where it sits in a focus cycle.
public struct CardContext {
    public var ended: EndedInfo?
    public var diffStat: DiffStat?
    /// Another surface is showing this session expanded.
    public var shownElsewhere = false
    /// Position and count for the focused stage's nav hint.
    public var nav: (index: Int, total: Int)?
    public var onPrevious: () -> Void = {}
    public var onNext: () -> Void = {}
    /// Left inset for the traffic lights when the header stands in for the titlebar.
    public var leadingInset: CGFloat = 12
    public var headerLeading: AnyView?
    /// The session's pane column (PaneColumn), drawn beside the terminal when it has panes.
    public var paneColumn: AnyView?
    /// The terminal's share of the card while a column sits beside it.
    public var splitRatio: Double = 0.5
    public var onSplitCommit: (Double) -> Void = { _ in }
    /// One of the column's panes is maximized: it takes the card and the chrome spans it.
    public var paneMaximized = false
    /// The focused stage gives the column a fixed 420pt instead of a share.
    public var fixedColumnWidth: CGFloat?

    public init(ended: EndedInfo? = nil, diffStat: DiffStat? = nil) {
        self.ended = ended
        self.diffStat = diffStat
    }
}

/// A session's card (AgentCard.tsx / FocusedTerminal.tsx): header, terminal, composer, status bar.
public struct AgentCardView: View {
    let session: TerminalSession
    let index: Int
    let engine: TerminalEngine?
    let isSelected: Bool
    let isDimmed: Bool
    let isExpanded: Bool
    let focusRequest: Int
    let actions: CardActions
    let context: CardContext
    let variant: CardVariant
    @State private var hovering = false

    public init(session: TerminalSession, index: Int, engine: TerminalEngine?, isSelected: Bool,
                isDimmed: Bool, isExpanded: Bool = false, focusRequest: Int = 0, actions: CardActions,
                context: CardContext = CardContext(), variant: CardVariant? = nil) {
        self.session = session
        self.index = index
        self.engine = engine
        self.isSelected = isSelected
        self.isDimmed = isDimmed
        self.isExpanded = isExpanded
        self.focusRequest = focusRequest
        self.actions = actions
        self.context = context
        self.variant = variant ?? (isExpanded ? .focused : .mini)
    }

    public var body: some View {
        VStack(spacing: 0) {
            if context.paneMaximized { header }
            GeometryReader { geo in
                HStack(spacing: 0) {
                    if !context.paneMaximized {
                        VStack(spacing: 0) {
                            header
                            terminal
                                .padding(variant == .focused ? 4 : 0)
                                .padding(.top, 2)
                                .frame(maxWidth: .infinity, maxHeight: .infinity)
                                .background(Theme.surfaceSunken)
                                .onRightClick { p in actions.contextMenu(session.id, p) }
                            SessionComposer(session: session, ended: context.ended, compact: variant == .mini, actions: actions)
                            statusBar
                        }
                        .frame(width: terminalWidth(geo.size.width))
                    }
                    if let column = context.paneColumn {
                        if !context.paneMaximized {
                            if context.fixedColumnWidth == nil {
                                SplitDivider(axis: .x, label: "Resize terminal and panels", origin: { geo.frame(in: .global).minX },
                                             extent: { geo.size.width }, onChange: { dragRatio = $0 },
                                             onCommit: { context.onSplitCommit($0); dragRatio = nil })
                            } else {
                                Rectangle().fill(Theme.hairline).frame(width: 1)
                            }
                        }
                        column.frame(maxWidth: .infinity, maxHeight: .infinity)
                    }
                }
            }
            if context.paneMaximized { statusBar }
        }
        .background(Theme.surfaceRaised)
        .overlay(Rectangle().strokeBorder(borderColor, lineWidth: 1))
        .zIndex(isSelected || hovering ? 1 : 0)
        .onHover { hovering = $0 }
        .animation(.easeOut(duration: 0.2), value: hovering)
        .simultaneousGesture(TapGesture().onEnded { if !isSelected { actions.select(session.id) } })
    }

    @State private var dragRatio: Double?

    private var header: some View {
        CardHeader(session: session, index: index, hovering: hovering, isExpanded: isExpanded,
                   variant: variant, context: context, actions: actions)
            .opacity(isDimmed && !hovering ? 0.6 : 1)
    }

    private var statusBar: some View {
        CardStatusBar(session: session, diffStat: context.diffStat, git: actions.git) { actions.showChanges(session.id) }
            .opacity(isDimmed && !hovering ? 0.6 : 1)
    }

    /// The terminal column's width: the whole card alone, its share (or what the fixed column leaves) beside panes.
    private func terminalWidth(_ total: CGFloat) -> CGFloat {
        guard context.paneColumn != nil else { return total }
        if let fixed = context.fixedColumnWidth { return max(0, total - fixed - 1) }
        return max(0, (total - 1) * (dragRatio ?? context.splitRatio))
    }

    @ViewBuilder private var terminal: some View {
        if context.shownElsewhere {
            Text("Expanded")
                .font(.ui(12))
                .foregroundStyle(Theme.gray600)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let engine {
            TerminalSurface(engine: engine, sessionID: session.id, focusRequest: focusRequest, onFocus: actions.select)
        } else {
            Color.clear
        }
    }

    private var borderColor: Color {
        if variant == .focused { return .clear }
        if isExpanded || context.shownElsewhere { return Theme.white(0.4) }
        return hovering ? Theme.white(0.12) : Theme.hairline
    }
}

struct CardHeader: View {
    let session: TerminalSession
    let index: Int
    let hovering: Bool
    let isExpanded: Bool
    let variant: CardVariant
    let context: CardContext
    let actions: CardActions
    @State private var renaming = false
    @State private var draft = ""
    @State private var nameHover = false

    var body: some View {
        HStack(spacing: 8) {
            if let leading = context.headerLeading {
                leading
                Rectangle().fill(Theme.hairline).frame(width: 1, height: 16)
            }
            HStack(spacing: 8) {
                AgentStatusIcon(session.agentType, status: session.status, size: 18)
                HStack(spacing: 4) {
                    if renaming {
                        TextField("", text: $draft)
                            .textFieldStyle(.plain)
                            .font(.ui(13, weight: .medium))
                            .foregroundStyle(Theme.gray200)
                            .onSubmit {
                                let name = draft.trimmingCharacters(in: .whitespaces)
                                if !name.isEmpty { actions.rename(session.id, name) }
                                renaming = false
                                actions.endRename()
                            }
                            .onExitCommand { renaming = false; actions.endRename() }
                    } else {
                        Text(session.title)
                            .font(.ui(13, weight: .medium))
                            .foregroundStyle(Theme.gray300)
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .help(session.title)
                        Button {
                            draft = session.title
                            renaming = true
                        } label: {
                            LucideIcon(.pencil, size: 10).foregroundStyle(Theme.gray500)
                        }
                        .buttonStyle(.plain)
                        .opacity(nameHover ? 1 : 0)
                        .accessibilityLabel("Rename session")
                    }
                }
                .onHover { nameHover = $0 }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
            .onTapGesture(count: 2) { actions.expand(session.id) }

            if variant == .focused, let nav = context.nav {
                FocusedNavHint(index: nav.index, total: nav.total, onPrevious: context.onPrevious, onNext: context.onNext)
                    .padding(.trailing, 4)
            }

            ZStack(alignment: .trailing) {
                if variant == .mini && index < 9 {
                    Text("⌘\(index + 1)")
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundStyle(Theme.gray600)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(RoundedRectangle(cornerRadius: Theme.radius).fill(Theme.white(0.04)))
                        .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Theme.hairline, lineWidth: 1))
                        .opacity(hovering ? 0 : 1)
                        .allowsHitTesting(false)
                }
                CardActionCluster(session: session, isExpanded: isExpanded, actions: actions)
                    .opacity(hovering || variant == .focused ? 1 : 0)
            }
            .fixedSize()
        }
        .padding(.leading, context.leadingInset)
        .padding(.trailing, 12)
        .padding(.vertical, 10)
        .frame(height: 41)
        .background(Theme.surfaceRaised)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.white(0.04)).frame(height: 1) }
        .onChange(of: actions.renameRequest == session.id, initial: true) { _, asked in
            if asked { draft = session.title; renaming = true }
        }
    }
}

/// The card's hover controls (CardActionCluster.tsx).
struct CardActionCluster: View {
    let session: TerminalSession
    let isExpanded: Bool
    let actions: CardActions
    @State private var confirmClose = false
    @State private var moreFrame: CGRect = .zero

    var body: some View {
        HStack(spacing: 2) {
            button(.ellipsis, "More actions") {
                actions.contextMenu(session.id, CGPoint(x: moreFrame.minX, y: moreFrame.maxY + 4))
            }
            .reportFrame($moreFrame)
            button(.folderOpen, "Browse files") { actions.browseFiles(session.id) }
            if session.agentType != .shell {
                button(.squareTerminal, "Add a terminal") { actions.addTerminal(session.id) }
                button(.globe, "Open browser") {}
            }
            if !isExpanded {
                button(.minus, "Minimize") { actions.minimize(session.id) }
            }
            button(isExpanded ? .minimize2 : .maximize2, isExpanded ? "Collapse to grid" : "Expand") {
                actions.expand(session.id)
            }
            IconButton(.x, size: 14, padding: 4, radius: Theme.radius, color: Theme.ink, hoverColor: Theme.danger,
                       hoverFill: Theme.white(0.10), help: "Close session") { confirmClose = true }
                .popover(isPresented: $confirmClose, arrowEdge: .bottom) {
                    ConfirmClose { confirmClose = false } onConfirm: {
                        confirmClose = false
                        actions.close(session.id)
                    }
                }
        }
    }

    private func button(_ icon: LucideGlyph, _ help: String, action: @escaping () -> Void) -> some View {
        IconButton(icon, size: 14, padding: 4, radius: Theme.radius, color: Theme.ink, hoverColor: Theme.ink,
                   hoverFill: Theme.white(0.10), help: help, action: action)
    }
}


/// "Close this session?" (ConfirmPopover).
public struct ConfirmClose: View {
    let onCancel: () -> Void
    let onConfirm: () -> Void

    public init(onCancel: @escaping () -> Void, onConfirm: @escaping () -> Void) {
        self.onCancel = onCancel
        self.onConfirm = onConfirm
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Close this session?").font(.ui(12)).foregroundStyle(Theme.gray200)
            HStack(spacing: 6) {
                Spacer()
                Button("Cancel", action: onCancel).keyboardShortcut(.cancelAction)
                Button("Close", role: .destructive, action: onConfirm).keyboardShortcut(.defaultAction)
            }
            .controlSize(.small)
        }
        .padding(12)
        .frame(width: 220)
    }
}

/// The card's bottom bar: worktree and branch, the tree's uncommitted changes, open in an editor (CardStatusBar.tsx).
struct CardStatusBar: View {
    let session: TerminalSession
    var diffStat: DiffStat?
    var git = GitActions()
    var onChanges: () -> Void = {}

    var body: some View {
        HStack(spacing: 8) {
            BranchChip(session: session, git: git)
            Spacer(minLength: 0)
            if let diffStat, diffStat.insertions > 0 || diffStat.deletions > 0 {
                GitChangesIndicator(stat: diffStat, action: onChanges)
            }
            OpenInButton(projectPath: session.projectPath, git: git)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .clipped()
        .font(.ui(11))
        .padding(.horizontal, 8)
        .frame(height: 22)
        .background(Theme.surfaceRaised)
        .overlay(alignment: .top) { Rectangle().fill(Theme.white(0.04)).frame(height: 1) }
    }
}

/// A session without its header: what the tabs layout shows under the active tab.
public struct SessionStage: View {
    let session: TerminalSession
    let engine: TerminalEngine?
    let focusRequest: Int
    let actions: CardActions
    let context: CardContext
    var showsStatusBar = true

    public init(session: TerminalSession, engine: TerminalEngine?, focusRequest: Int = 0, actions: CardActions,
                context: CardContext = CardContext(), showsStatusBar: Bool = true) {
        self.session = session
        self.engine = engine
        self.focusRequest = focusRequest
        self.actions = actions
        self.context = context
        self.showsStatusBar = showsStatusBar
    }

    public var body: some View {
        VStack(spacing: 0) {
            Group {
                if let engine {
                    TerminalSurface(engine: engine, sessionID: session.id, focusRequest: focusRequest, onFocus: actions.select)
                } else {
                    Color.clear
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .onRightClick { p in actions.contextMenu(session.id, p) }
            SessionComposer(session: session, ended: context.ended, compact: !showsStatusBar, actions: actions)
            if showsStatusBar {
                CardStatusBar(session: session, diffStat: context.diffStat, git: actions.git) { actions.showChanges(session.id) }
            }
        }
        .background(Theme.surfaceSunken)
    }
}
