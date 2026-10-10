import SwiftUI
import VornCore
import VornUI

/// The sessions grid in smart-auto mode (GridView.tsx): cards edge to edge,
/// the composer when there are none.
public struct SessionGridView<Empty: View>: View {
    let sessions: [TerminalSession]
    let engine: TerminalEngine?
    let selectedID: String?
    let focusRequest: [String: Int]
    let actions: CardActions
    let onDoubleClickEmpty: () -> Void
    let empty: () -> Empty

    public init(sessions: [TerminalSession], engine: TerminalEngine?, selectedID: String?,
                focusRequest: [String: Int] = [:], actions: CardActions,
                onDoubleClickEmpty: @escaping () -> Void = {}, @ViewBuilder empty: @escaping () -> Empty) {
        self.sessions = sessions
        self.engine = engine
        self.selectedID = selectedID
        self.focusRequest = focusRequest
        self.actions = actions
        self.onDoubleClickEmpty = onDoubleClickEmpty
        self.empty = empty
    }

    public var body: some View {
        if sessions.isEmpty {
            empty().frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            GeometryReader { geo in
                let layout = AutoLayout.pick(sessions.count, width: geo.size.width, height: geo.size.height)
                let cardW = geo.size.width / CGFloat(layout.cols)
                let cardH = layout.mode == .fit
                    ? geo.size.height / CGFloat(layout.rows)
                    : AutoLayout.scrollRowHeight(geo.size.height)
                let grid = cards(layout: layout, cardW: cardW, cardH: cardH)
                ZStack(alignment: .topLeading) {
                    Color.clear
                        .contentShape(Rectangle())
                        .onTapGesture(count: 2, perform: onDoubleClickEmpty)
                    if layout.mode == .scroll {
                        ScrollView { grid }
                    } else {
                        grid
                    }
                }
            }
        }
    }

    private func cards(layout: AutoLayout, cardW: CGFloat, cardH: CGFloat) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(0..<layout.rows, id: \.self) { row in
                HStack(spacing: 0) {
                    ForEach(Array(sessions.enumerated().dropFirst(row * layout.cols).prefix(layout.cols)), id: \.element.id) { i, s in
                        AgentCardView(session: s, index: i, engine: engine, isSelected: s.id == selectedID,
                                      isDimmed: selectedID != nil && s.id != selectedID,
                                      focusRequest: focusRequest[s.id] ?? 0, actions: actions)
                            .frame(width: cardW, height: cardH)
                    }
                }
            }
        }
    }
}
