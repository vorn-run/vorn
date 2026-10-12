import SwiftUI
import VornCore
import VornUI

/// icon-button.ts: `p-1 rounded text-ink hover:bg-white/[0.10]`, 14px glyphs.
struct PaneIconButton: View {
    let icon: LucideGlyph
    let help: String
    var size: CGFloat = 14
    var strokeWidth: CGFloat = 2
    var active = false
    var disabled = false
    var dimsWhenDisabled = true
    let action: () -> Void

    var body: some View {
        Hovering { hover in
            Button(action: action) {
                LucideIcon(icon, size: size, strokeWidth: strokeWidth)
                    .foregroundStyle(Theme.ink)
                    .padding(4)
                    .background(RoundedRectangle(cornerRadius: Theme.radius).fill(active || (hover && !disabled) ? Theme.white(0.10) : .clear))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(disabled)
            .opacity(disabled && dimsWhenDisabled ? 0.4 : 1)
        }
        .help(help)
        .accessibilityLabel(help)
    }
}

/// PaneControls: pop out, maximize or restore, close.
struct PaneControls: View {
    @Bindable var panes: PanesModel
    let paneID: String
    let title: String
    var onPopOut: (() -> Void)?
    var popOutLabel: String?
    let onClose: () -> Void

    var body: some View {
        let max = panes.maximizedPaneId == paneID
        HStack(spacing: 2) {
            if let onPopOut {
                PaneIconButton(icon: .squareArrowOutUpRight, help: "Open as its own card", action: onPopOut)
                    .accessibilityLabel("Open \(popOutLabel ?? title) as its own card")
            }
            PaneIconButton(icon: max ? .minimize2 : .maximize2, help: max ? "Restore" : "Maximize") { panes.toggleMaximized(paneID) }
            PaneIconButton(icon: .x, help: "Close", action: onClose)
        }
    }
}

/// PromotedCardControls: minimize to the dock, put back in the session, close.
struct PromotedCardControls: View {
    let title: String
    let onMinimize: () -> Void
    let onReturn: () -> Void
    let onClose: () -> Void

    var body: some View {
        HStack(spacing: 2) {
            PaneIconButton(icon: .minus, help: "Minimize", action: onMinimize)
            PaneIconButton(icon: .cornerDownLeft, help: "Put back in its session card", action: onReturn)
            PaneIconButton(icon: .x, help: "Close", action: onClose)
        }
    }
}

/// PaneTabStrip.tsx: the tab strip a pane uses as its title bar.
struct PaneTab: Identifiable {
    let id: String
    let name: String
    var detail: String?
    var title: String?
    var icon: AnyView?
    var italic = false
    var dirty = false
}

struct PaneTabStrip<Leading: View, Trailing: View>: View {
    let tabs: [PaneTab]
    let activeID: String?
    var emptyTitle: String?
    let onSelect: (String) -> Void
    let onClose: (String) -> Void
    var onDoubleClickTab: ((String) -> Void)?
    var tabAction: ((PaneTab) -> AnyView)?
    var onAdd: (() -> Void)?
    var addLabel = "New tab"
    var menuFooter: [(String, () -> Void)] = []
    var onDoubleClick: (() -> Void)?
    @ViewBuilder var leading: () -> Leading
    @ViewBuilder var trailing: () -> Trailing
    @State private var menuOpen = false
    @State private var contentWidth: CGFloat = 0
    @State private var viewport: CGFloat = 0

    var body: some View {
        HStack(spacing: 4) {
            leading()
            ScrollView(.horizontal) {
                HStack(spacing: 2) {
                    if tabs.isEmpty, let emptyTitle {
                        Text(emptyTitle).font(.ui(12, weight: .medium)).foregroundStyle(Theme.ink).padding(.leading, 4)
                            .frame(maxHeight: .infinity)
                    }
                    ForEach(tabs) { tab in tabView(tab) }
                }
                .background(GeometryReader { g in Color.clear.onAppear { contentWidth = g.size.width }
                    .onChange(of: g.size.width) { _, w in contentWidth = w } })
            }
            .scrollIndicators(.never)
            .background(GeometryReader { g in Color.clear.onAppear { viewport = g.size.width }
                .onChange(of: g.size.width) { _, w in viewport = w } })
            .frame(maxWidth: .infinity, alignment: .leading)
            if let onAdd {
                IconButton(.plus, size: 13, padding: 4, radius: Theme.radius, color: Theme.gray600, hoverColor: Theme.gray200,
                           hoverFill: Theme.white(0.06), help: addLabel, action: onAdd)
            }
            if contentWidth > viewport + 1 {
                PaneIconButton(icon: .chevronDown, help: "All open tabs", size: 13, active: menuOpen) { menuOpen.toggle() }
                    .popover(isPresented: $menuOpen, arrowEdge: .bottom) { menu }
            }
            trailing()
        }
        .padding(.leading, 6)
        .padding(.trailing, 4)
        .padding(.top, 4)
        .frame(height: 30)
        .contentShape(Rectangle())
        .onTapGesture(count: 2) { onDoubleClick?() }
    }

    private func tabView(_ tab: PaneTab) -> some View {
        let active = tab.id == activeID
        return Hovering { hover in
            HStack(spacing: 4) {
                if let icon = tab.icon { icon }
                (Text(tab.name) + (tab.detail.map { Text(" " + $0).foregroundColor(Theme.inkFaint) } ?? Text("")))
                    .font(.ui(11)).italic(tab.italic).lineLimit(1)
                if let a = tabAction { a(tab) }
                ZStack {
                    if tab.dirty && !hover {
                        Circle().fill(Theme.amber400).frame(width: 6, height: 6).help("Unsaved changes")
                    }
                    Button { onClose(tab.id) } label: {
                        LucideIcon(.x, size: 10, strokeWidth: 2.5).foregroundStyle(Theme.gray600).padding(2)
                    }
                    .buttonStyle(.plain)
                    .opacity(hover ? 1 : 0)
                    .accessibilityLabel("Close \(tab.name)")
                }
                .frame(width: 14, height: 14)
            }
            .foregroundStyle(active ? Theme.gray200 : hover ? Theme.gray300 : Theme.gray500)
            .padding(.leading, 10)
            .padding(.trailing, 4)
            .padding(.vertical, 4)
            .frame(maxWidth: 170)
            .fixedSize(horizontal: true, vertical: false)
            .background(RoundedRectangle(cornerRadius: Theme.radius).fill(active ? Theme.white(0.06) : hover ? Theme.white(0.03) : .clear))
            .contentShape(Rectangle())
            .onTapGesture(count: 2) { onDoubleClickTab?(tab.id) }
            .simultaneousGesture(TapGesture().onEnded { onSelect(tab.id) })
            .help(tab.title ?? tab.name)
        }
    }

    private var menu: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(tabs) { tab in
                Button {
                    onSelect(tab.id)
                    menuOpen = false
                } label: {
                    HStack(spacing: 6) {
                        if let icon = tab.icon { icon }
                        Text(tab.name).font(.ui(12)).lineLimit(1)
                        Spacer(minLength: 0)
                        if tab.dirty { Circle().fill(Theme.amber400).frame(width: 6, height: 6) }
                    }
                    .foregroundStyle(tab.id == activeID ? Theme.ink : Theme.inkSecondary)
                    .padding(.horizontal, 8)
                    .frame(height: 26)
                    .background(RoundedRectangle(cornerRadius: Theme.radius).fill(tab.id == activeID ? Theme.white(0.08) : .clear))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            if !menuFooter.isEmpty {
                Rectangle().fill(Theme.white(0.07)).frame(height: 1).padding(.vertical, 4)
                ForEach(Array(menuFooter.enumerated()), id: \.offset) { _, item in
                    Button {
                        item.1()
                        menuOpen = false
                    } label: {
                        Text(item.0).font(.ui(12)).foregroundStyle(Theme.inkSecondary)
                            .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 8).frame(height: 26)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .padding(4)
        .frame(width: 220)
        .background(Theme.surfaceOverlay)
    }
}

extension PaneTabStrip where Leading == EmptyView {
    init(tabs: [PaneTab], activeID: String?, emptyTitle: String? = nil, onSelect: @escaping (String) -> Void,
         onClose: @escaping (String) -> Void, tabAction: ((PaneTab) -> AnyView)? = nil, onAdd: (() -> Void)? = nil,
         addLabel: String = "New tab", @ViewBuilder trailing: @escaping () -> Trailing) {
        self.init(tabs: tabs, activeID: activeID, emptyTitle: emptyTitle, onSelect: onSelect, onClose: onClose,
                  tabAction: tabAction, onAdd: onAdd, addLabel: addLabel, leading: { EmptyView() }, trailing: trailing)
    }
}
