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
    public var more: (String) -> Void = { _ in }
    public var browseFiles: (String) -> Void = { _ in }

    public init() {}
}

/// A session's card in the grid (AgentCard.tsx, mini variant): header, terminal, status bar.
public struct AgentCardView: View {
    let session: TerminalSession
    let index: Int
    let engine: TerminalEngine?
    let isSelected: Bool
    let isDimmed: Bool
    let isExpanded: Bool
    let focusRequest: Int
    let actions: CardActions
    @State private var hovering = false

    public init(session: TerminalSession, index: Int, engine: TerminalEngine?, isSelected: Bool,
                isDimmed: Bool, isExpanded: Bool = false, focusRequest: Int = 0, actions: CardActions) {
        self.session = session
        self.index = index
        self.engine = engine
        self.isSelected = isSelected
        self.isDimmed = isDimmed
        self.isExpanded = isExpanded
        self.focusRequest = focusRequest
        self.actions = actions
    }

    public var body: some View {
        VStack(spacing: 0) {
            CardHeader(session: session, index: index, hovering: hovering, isExpanded: isExpanded, actions: actions)
                .opacity(isDimmed && !hovering ? 0.6 : 1)
            body(engine: engine)
                .padding(.top, 2)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Theme.surfaceSunken)
            CardStatusBar(session: session)
                .opacity(isDimmed && !hovering ? 0.6 : 1)
        }
        .background(Theme.surfaceRaised)
        .overlay(Rectangle().strokeBorder(borderColor, lineWidth: 1))
        .zIndex(isSelected || hovering ? 1 : 0)
        .onHover { hovering = $0 }
        .animation(.easeOut(duration: 0.2), value: hovering)
        .simultaneousGesture(TapGesture().onEnded { if !isSelected { actions.select(session.id) } })
    }

    @ViewBuilder private func body(engine: TerminalEngine?) -> some View {
        if let engine {
            TerminalSurface(engine: engine, sessionID: session.id, focusRequest: focusRequest, onFocus: actions.select)
        } else {
            Color.clear
        }
    }

    private var borderColor: Color {
        if isExpanded { return Theme.white(0.4) }
        return hovering ? Theme.white(0.12) : Theme.hairline
    }
}

struct CardHeader: View {
    let session: TerminalSession
    let index: Int
    let hovering: Bool
    let isExpanded: Bool
    let actions: CardActions
    @State private var renaming = false
    @State private var draft = ""
    @State private var nameHover = false

    var body: some View {
        HStack(spacing: 8) {
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
                            }
                            .onExitCommand { renaming = false }
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

            ZStack(alignment: .trailing) {
                if index < 9 {
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
                    .opacity(hovering ? 1 : 0)
            }
            .fixedSize()
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .frame(height: 41)
        .background(Theme.surfaceRaised)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.white(0.04)).frame(height: 1) }
    }
}

/// The card's hover controls (CardActionCluster.tsx).
struct CardActionCluster: View {
    let session: TerminalSession
    let isExpanded: Bool
    let actions: CardActions
    @State private var confirmClose = false

    var body: some View {
        HStack(spacing: 2) {
            button(.ellipsis, "More actions") { actions.more(session.id) }
            button(.folderOpen, "Browse files") { actions.browseFiles(session.id) }
            if session.agentType != .shell {
                button(.squareTerminal, "Add a terminal") {}
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

    private func button(_ icon: Lucide, _ help: String, action: @escaping () -> Void) -> some View {
        IconButton(icon, size: 14, padding: 4, radius: Theme.radius, color: Theme.ink, hoverColor: Theme.ink,
                   hoverFill: Theme.white(0.10), help: help, action: action)
    }
}

/// "Close this session?" (ConfirmPopover).
struct ConfirmClose: View {
    let onCancel: () -> Void
    let onConfirm: () -> Void

    var body: some View {
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

/// The card's bottom bar: branch and worktree (CardStatusBar.tsx).
struct CardStatusBar: View {
    let session: TerminalSession

    var body: some View {
        HStack(spacing: 8) {
            if let branch = session.branch, !branch.isEmpty {
                BranchChip(branch: branch)
            }
            if session.isWorktree == true, let name = session.worktreeName {
                HStack(spacing: 4) {
                    LucideIcon(.folderGit2, size: 10, strokeWidth: 1.5)
                    Text(name).font(.system(size: 10, design: .monospaced)).lineLimit(1)
                }
                .foregroundStyle(Theme.inkSecondary)
            }
            Spacer(minLength: 0)
        }
        .font(.ui(11))
        .padding(.horizontal, 8)
        .frame(height: 22)
        .background(Theme.surfaceRaised)
        .overlay(alignment: .top) { Rectangle().fill(Theme.white(0.04)).frame(height: 1) }
    }
}

/// The session's branch (BranchChip.tsx).
struct BranchChip: View {
    let branch: String

    var body: some View {
        Hovering { hover in
            HStack(spacing: 4) {
                LucideIcon(.gitBranch, size: 10).foregroundStyle(Theme.gray500)
                Text(branch)
                    .font(.system(size: 10, design: .monospaced))
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: 110, alignment: .leading)
                    .fixedSize(horizontal: true, vertical: false)
                LucideIcon(.chevronDown, size: 9).foregroundStyle(Theme.gray600)
            }
            .foregroundStyle(hover ? Theme.gray200 : Theme.gray400)
            .padding(.horizontal, 4)
            .padding(.vertical, 2)
            .background(RoundedRectangle(cornerRadius: Theme.radius).fill(hover ? Theme.white(0.06) : .clear))
        }
    }
}
