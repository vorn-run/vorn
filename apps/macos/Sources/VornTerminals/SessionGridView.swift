import SwiftUI
import VornCore
import VornUI

/// The sessions grid (GridView.tsx): smart-auto or fixed columns, cards edge to edge,
/// the composer when there are none.
public struct SessionGridView<Empty: View>: View {
    let sessions: [TerminalSession]
    let engine: TerminalEngine?
    let selectedID: String?
    let focusRequest: [String: Int]
    let actions: CardActions
    /// 0 auto (and flexible), 1-4 fixed.
    let columns: Int
    let isFiltered: Bool
    let context: (TerminalSession) -> CardContext
    let onDoubleClickEmpty: () -> Void
    let onContextMenu: (CGPoint) -> Void
    let empty: () -> Empty
    /// Cards popped out of a session (a file, today), drawn as cells right after it.
    var cardsAfter: (TerminalSession) -> [(id: String, view: AnyView)] = { _ in [] }

    /// `rowHeight` (208) plus the card's header and status bar, for fixed columns.
    static var fixedRowHeight: CGFloat { 250 }

    public init(sessions: [TerminalSession], engine: TerminalEngine?, selectedID: String?,
                focusRequest: [String: Int] = [:], actions: CardActions, columns: Int = 0, isFiltered: Bool = false,
                context: @escaping (TerminalSession) -> CardContext = { _ in CardContext() },
                onDoubleClickEmpty: @escaping () -> Void = {}, onContextMenu: @escaping (CGPoint) -> Void = { _ in },
                @ViewBuilder empty: @escaping () -> Empty) {
        self.sessions = sessions
        self.engine = engine
        self.selectedID = selectedID
        self.focusRequest = focusRequest
        self.actions = actions
        self.columns = columns
        self.isFiltered = isFiltered
        self.context = context
        self.onDoubleClickEmpty = onDoubleClickEmpty
        self.onContextMenu = onContextMenu
        self.empty = empty
    }

    public func cards(after: @escaping (TerminalSession) -> [(id: String, view: AnyView)]) -> Self {
        var copy = self
        copy.cardsAfter = after
        return copy
    }

    private enum Cell: Identifiable {
        case session(Int, TerminalSession)
        case card(String, AnyView)
        var id: String {
            switch self {
            case .session(_, let s): s.id
            case .card(let id, _): id
            }
        }
    }

    private var cells: [Cell] {
        sessions.enumerated().flatMap { i, s in [Cell.session(i, s)] + cardsAfter(s).map { Cell.card($0.id, $0.view) } }
    }

    public var body: some View {
        if sessions.isEmpty {
            Group {
                if isFiltered { NoMatchingAgents() } else { empty() }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color.clear.contentShape(Rectangle()).onRightClick(onContextMenu))
        } else {
            GeometryReader { geo in
                let layout = Self.layout(cells.count, columns: columns, size: geo.size)
                let cardW = geo.size.width / CGFloat(layout.cols)
                let cardH = columns > 0 ? Self.fixedRowHeight
                    : layout.mode == .fit ? geo.size.height / CGFloat(layout.rows) : AutoLayout.scrollRowHeight(geo.size.height)
                let grid = cards(layout: layout, cardW: cardW, cardH: cardH)
                ZStack(alignment: .topLeading) {
                    Color.clear
                        .contentShape(Rectangle())
                        .onTapGesture(count: 2, perform: onDoubleClickEmpty)
                        .onRightClick(onContextMenu)
                    if layout.mode == .scroll {
                        ScrollView { grid }
                    } else {
                        grid
                    }
                }
            }
        }
    }

    static func layout(_ count: Int, columns: Int, size: CGSize) -> AutoLayout {
        guard columns > 0 else { return AutoLayout.pick(count, width: size.width, height: size.height) }
        let rows = (count + columns - 1) / columns
        let fits = CGFloat(rows) * fixedRowHeight <= size.height
        return AutoLayout(cols: columns, rows: rows, mode: fits ? .fit : .scroll)
    }

    private func cards(layout: AutoLayout, cardW: CGFloat, cardH: CGFloat) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(0..<layout.rows, id: \.self) { row in
                HStack(spacing: 0) {
                    ForEach(Array(cells.dropFirst(row * layout.cols).prefix(layout.cols))) { cell in
                        switch cell {
                        case .session(let i, let s):
                            AgentCardView(session: s, index: i, engine: engine, isSelected: s.id == selectedID,
                                          isDimmed: selectedID != nil && s.id != selectedID,
                                          focusRequest: focusRequest[s.id] ?? 0, actions: actions, context: context(s))
                                .frame(width: cardW, height: cardH)
                        case .card(_, let view):
                            view.frame(width: cardW, height: cardH)
                        }
                    }
                }
            }
        }
    }
}

/// The grid with a status filter that matches nothing.
public struct NoMatchingAgents: View {
    static let monitor = LucideGlyph([
        .rect(x: 2, y: 3, width: 20, height: 14, rx: 2), .path("M8 21h8M12 17v4"), .path("M7 8l3 3-3 3M12 14h4"),
    ])

    public init() {}

    public var body: some View {
        VStack(spacing: 0) {
            LucideIcon(Self.monitor, size: 64, strokeWidth: 1)
                .foregroundStyle(Theme.white(0.2))
                .padding(.bottom, 24)
            Text("No matching agents")
                .font(.ui(24, weight: .semibold))
                .foregroundStyle(.white)
                .padding(.bottom, 8)
            Text("Try changing the status filter")
                .font(.ui(14))
                .foregroundStyle(Theme.gray500)
                .padding(.bottom, 24)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
