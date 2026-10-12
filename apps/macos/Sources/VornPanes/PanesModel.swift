import AppKit
import Foundation
import Observation
import VornCore
import VornUI

/// A session's Files pane: open tabs, the one in front, the italic preview tab, the tree's visibility.
public struct FilesPaneState: Codable, Equatable, Sendable {
    public var tabs: [String] = []
    public var active: String?
    public var preview: String?
    public var treeVisible = true
    public init() {}
}

/// The shells a session holds beside its agent.
public struct TerminalsPaneState: Codable, Equatable, Sendable {
    public var terminals: [String]
    public var activeTab: Int
    public var activeIndex: Int { terminals.isEmpty ? 0 : min(max(activeTab, 0), terminals.count - 1) }
    public var activeID: String? { terminals.isEmpty ? nil : terminals[activeIndex] }
}

/// How a card divides itself: the terminal's share, and the stacked panes' weights.
public struct CardSplit: Codable, Equatable, Sendable {
    public var terminal: Double
    public var panes: [Double]
}

/// A file popped out of its session's Files pane into a card of its own.
public struct EditorCardState: Codable, Equatable, Sendable {
    public var filePath: String
    public var sessionId: String
}

/// One pane in a session's column, keyed by the id maximize matches on.
public struct ColumnEntry: Equatable, Sendable, Identifiable {
    public enum Kind: String, Sendable { case files, terminals }
    public let id: String
    public let kind: Kind
}

/// pane-id.ts: the ids panes and popped-out cards go by.
public enum PaneID {
    public static func files(_ s: String) -> String { "files:\(s)" }
    public static func terminals(_ s: String) -> String { "terminals:\(s)" }
    public static func fileTab(_ s: String, _ path: String) -> String { "tab:\(s)\u{0}\(path)" }
    public static func card(_ s: String, _ seq: Int) -> String { "card:\(s):\(seq)" }
    public static func isCard(_ id: String) -> Bool { id.hasPrefix("card:") }
}

/// The panes sessions own (files, panel terminals), the files popped out as cards, the card splits,
/// the maximized pane and the changes panel — today's `ui-slice` pane state.
@MainActor
@Observable
public final class PanesModel {
    public private(set) var filesPanes: [String: FilesPaneState] = [:]
    public private(set) var terminalsPanes: [String: TerminalsPaneState] = [:]
    public private(set) var editorCards: [String: EditorCardState] = [:]
    /// Card ids in the order they were opened, for the grid.
    public private(set) var cardOrder: [String] = []
    public private(set) var cardSplits: [String: CardSplit] = [:]
    public var maximizedPaneId: String?
    public var treeRatio: Double = 0.38 { didSet { save() } }
    /// Editors with unsaved edits, by tab key or card id (editor-dirty.ts).
    public private(set) var dirty: Set<String> = []

    /// The changes panel (RightPanel.tsx): whose diff it shows, its width, and whether it fills the window.
    public var diffSessionID: String?
    public var diffWidth: CGFloat = 480
    public var diffMaximized = false

    /// Asks before unsaved edits are thrown away; renders and tests answer for the person.
    @ObservationIgnored public var confirmDiscard: () -> Bool = PanesModel.askDiscard
    @ObservationIgnored public var persists: Bool
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private var cardSeq = 0

    public init(defaults: UserDefaults = .standard, persists: Bool = true) {
        self.defaults = defaults
        self.persists = persists
        guard persists else { return }
        let dec = JSONDecoder()
        if let d = defaults.data(forKey: "vorn.filesPanes"), let v = try? dec.decode([String: FilesPaneState].self, from: d) { filesPanes = v }
        if let d = defaults.data(forKey: "vorn.terminalPanels"), let v = try? dec.decode([String: TerminalsPaneState].self, from: d) { terminalsPanes = v }
        if let d = defaults.data(forKey: "vorn.editorCards"), let v = try? dec.decode([String: EditorCardState].self, from: d) {
            editorCards = v
            cardOrder = v.keys.sorted()
            cardSeq = v.keys.compactMap { $0.split(separator: ":").last.flatMap { Int($0) } }.max().map { $0 + 1 } ?? 0
        }
        if let d = defaults.data(forKey: "vorn.cardSplits"), let v = try? dec.decode([String: CardSplit].self, from: d) { cardSplits = v }
        let r = defaults.double(forKey: "vorn.filesTreeRatio")
        if r > 0 { treeRatio = SplitRatio.clampTree(r) }
    }

    private func save() {
        guard persists else { return }
        let enc = JSONEncoder()
        defaults.set(try? enc.encode(filesPanes), forKey: "vorn.filesPanes")
        defaults.set(try? enc.encode(terminalsPanes), forKey: "vorn.terminalPanels")
        defaults.set(try? enc.encode(editorCards), forKey: "vorn.editorCards")
        defaults.set(try? enc.encode(cardSplits), forKey: "vorn.cardSplits")
        defaults.set(treeRatio, forKey: "vorn.filesTreeRatio")
    }

    static func askDiscard() -> Bool {
        let alert = NSAlert()
        alert.messageText = "Discard unsaved changes?"
        alert.addButton(withTitle: "OK")
        alert.addButton(withTitle: "Cancel")
        return alert.runModal() == .alertFirstButtonReturn
    }

    // MARK: Dirty state

    public func setDirty(_ key: String, _ on: Bool) {
        if on { dirty.insert(key) } else { dirty.remove(key) }
    }

    public func isDirty(_ key: String) -> Bool { dirty.contains(key) }

    /// confirmDiscard / confirmDiscardAll: asks only when something is dirty, and clears what it gave up.
    public func confirmDiscard(_ keys: [String]) -> Bool {
        let d = keys.filter(dirty.contains)
        guard !d.isEmpty else { return true }
        guard confirmDiscard() else { return false }
        for k in d { dirty.remove(k) }
        return true
    }

    // MARK: Column

    /// usePaneColumnEntries: what a session's column draws, in order.
    public func columnEntries(_ sessionID: String) -> [ColumnEntry] {
        var out: [ColumnEntry] = []
        if filesPanes[sessionID] != nil { out.append(ColumnEntry(id: PaneID.files(sessionID), kind: .files)) }
        if let t = terminalsPanes[sessionID], !t.terminals.isEmpty {
            out.append(ColumnEntry(id: PaneID.terminals(sessionID), kind: .terminals))
        }
        return out
    }

    public func setCardSplit(_ sessionID: String, terminal: Double? = nil, panes: [Double]? = nil) {
        let old = cardSplits[sessionID]
        cardSplits[sessionID] = CardSplit(terminal: terminal ?? old?.terminal ?? SplitRatio.default, panes: panes ?? old?.panes ?? [])
        save()
    }

    public func toggleMaximized(_ paneID: String) {
        maximizedPaneId = maximizedPaneId == paneID ? nil : paneID
    }

    // MARK: Files pane

    public func toggleFilesPane(_ s: String) {
        if let open = filesPanes[s] {
            if confirmDiscard(open.tabs.map { PaneID.fileTab(s, $0) }) { closeFilesPane(s) }
        } else {
            filesPanes[s] = FilesPaneState()
            save()
        }
    }

    public func closeFilesPane(_ s: String) {
        guard let closing = filesPanes.removeValue(forKey: s) else { return }
        for p in closing.tabs { dirty.remove(PaneID.fileTab(s, p)) }
        if maximizedPaneId == PaneID.files(s) { maximizedPaneId = nil }
        save()
    }

    private func write(_ s: String, _ pane: FilesPaneState) {
        filesPanes[s] = pane
        save()
    }

    /// openFileTab: a click reuses the clean preview tab; a double click (`pin`) keeps one.
    public func openFileTab(_ s: String, _ path: String, pin: Bool = false) {
        var pane = filesPanes[s] ?? FilesPaneState()
        if pane.tabs.contains(path) {
            if pin && pane.preview == path { pane.preview = nil }
        } else if pin {
            pane.tabs.append(path)
        } else {
            let reusable = pane.preview.map { !isDirty(PaneID.fileTab(s, $0)) } ?? false
            pane.tabs = reusable ? pane.tabs.map { $0 == pane.preview ? path : $0 } : pane.tabs + [path]
            pane.preview = path
        }
        pane.active = path
        write(s, pane)
    }

    public func pinFileTab(_ s: String, _ path: String) {
        guard var pane = filesPanes[s], pane.preview == path else { return }
        pane.preview = nil
        write(s, pane)
    }

    public func setActiveFileTab(_ s: String, _ path: String) {
        guard var pane = filesPanes[s], pane.active != path, pane.tabs.contains(path) else { return }
        pane.active = path
        write(s, pane)
    }

    public func closeFileTab(_ s: String, _ path: String) {
        guard var pane = filesPanes[s], let i = pane.tabs.firstIndex(of: path) else { return }
        dirty.remove(PaneID.fileTab(s, path))
        pane.tabs.remove(at: i)
        if pane.active == path { pane.active = pane.tabs[safe: i] ?? pane.tabs[safe: i - 1] }
        if pane.preview == path { pane.preview = nil }
        write(s, pane)
    }

    /// The tab's ✕: asks first when its edits are unsaved.
    public func requestCloseFileTab(_ s: String, _ path: String) {
        if confirmDiscard([PaneID.fileTab(s, path)]) { closeFileTab(s, path) }
    }

    public func closeSavedFileTabs(_ s: String) {
        guard var pane = filesPanes[s] else { return }
        let tabs = pane.tabs.filter { isDirty(PaneID.fileTab(s, $0)) }
        guard tabs.count != pane.tabs.count else { return }
        pane.active = pane.active.flatMap { tabs.contains($0) ? $0 : nil } ?? tabs.first
        pane.preview = pane.preview.flatMap { tabs.contains($0) ? $0 : nil }
        pane.tabs = tabs
        write(s, pane)
    }

    public func setFileTreeVisible(_ s: String, _ visible: Bool) {
        guard var pane = filesPanes[s], pane.treeVisible != visible else { return }
        pane.treeVisible = visible
        write(s, pane)
    }

    public func toggleFileTree(_ s: String) {
        if let pane = filesPanes[s] { setFileTreeVisible(s, !pane.treeVisible) }
    }

    // MARK: Cards

    public func cards(ownedBy s: String) -> [String] { cardOrder.filter { editorCards[$0]?.sessionId == s } }

    /// promoteFile: one card per file per session; an existing one is surfaced instead.
    @discardableResult
    public func promoteFile(_ s: String, _ path: String) -> String {
        if let existing = cardOrder.first(where: { editorCards[$0] == EditorCardState(filePath: path, sessionId: s) }) {
            onSurfaceCard(existing)
            return existing
        }
        let id = PaneID.card(s, cardSeq)
        cardSeq += 1
        editorCards[id] = EditorCardState(filePath: path, sessionId: s)
        cardOrder.append(id)
        save()
        return id
    }

    /// The tab's pop-out: asks about unsaved edits, then moves the file into a card.
    public func popOutTab(_ s: String, _ path: String) {
        guard confirmDiscard([PaneID.fileTab(s, path)]) else { return }
        promoteFile(s, path)
        closeFileTab(s, path)
    }

    /// Shows a minimized card again; the shell sets it.
    @ObservationIgnored public var onSurfaceCard: (String) -> Void = { _ in }

    public func closeCard(_ id: String) {
        guard editorCards[id] != nil, confirmDiscard([id]) else { return }
        dirty.remove(id)
        editorCards[id] = nil
        cardOrder.removeAll { $0 == id }
        if maximizedPaneId == id { maximizedPaneId = nil }
        save()
    }

    /// returnCardToSession: the file goes back as a pinned tab; the card's buffer does not travel.
    public func returnCard(_ id: String) {
        guard let card = editorCards[id], confirmDiscard([id]) else { return }
        dirty.remove(id)
        openFileTab(card.sessionId, card.filePath, pin: true)
        editorCards[id] = nil
        cardOrder.removeAll { $0 == id }
        save()
    }

    // MARK: Panel terminals

    /// Terminals held in some session's panel: drawn there and nowhere else (claimedTerminalIds).
    public var claimedTerminalIDs: Set<String> { Set(terminalsPanes.values.flatMap(\.terminals)) }

    public func openTerminalsPane(_ s: String, terminal id: String) {
        var terminals = terminalsPanes[s]?.terminals ?? []
        if !terminals.contains(id) { terminals.append(id) }
        terminalsPanes[s] = TerminalsPaneState(terminals: terminals, activeTab: terminals.firstIndex(of: id) ?? 0)
        save()
    }

    public func closeTerminalsPane(_ s: String) {
        guard terminalsPanes.removeValue(forKey: s) != nil else { return }
        if maximizedPaneId == PaneID.terminals(s) { maximizedPaneId = nil }
        save()
    }

    public func setActivePanelTerminal(_ s: String, index: Int) {
        guard var pane = terminalsPanes[s], pane.terminals.indices.contains(index), pane.activeTab != index else { return }
        pane.activeTab = index
        terminalsPanes[s] = pane
        save()
    }

    /// Releases a terminal from whichever panel holds it; an emptied panel goes.
    public func releaseTerminal(_ id: String) {
        for (owner, var pane) in terminalsPanes where pane.terminals.contains(id) {
            pane.terminals.removeAll { $0 == id }
            if pane.terminals.isEmpty {
                closeTerminalsPane(owner)
            } else {
                pane.activeTab = min(pane.activeTab, pane.terminals.count - 1)
                terminalsPanes[owner] = pane
            }
        }
        save()
    }

    /// Drops what belongs to sessions that are gone.
    public func prune(liveSessions: Set<String>) {
        for id in filesPanes.keys where !liveSessions.contains(id) { closeFilesPane(id) }
        for (owner, pane) in terminalsPanes {
            if !liveSessions.contains(owner) { closeTerminalsPane(owner); continue }
            for t in pane.terminals where !liveSessions.contains(t) { releaseTerminal(t) }
        }
        for id in cardOrder where editorCards[id].map({ !liveSessions.contains($0.sessionId) }) ?? true {
            dirty.remove(id)
            editorCards[id] = nil
            cardOrder.removeAll { $0 == id }
        }
        if let d = diffSessionID, !liveSessions.contains(d) { diffSessionID = nil }
        save()
    }
}

extension SplitRatio {
    static func clampTree(_ r: Double) -> Double { clamp(r) }
}

extension Array {
    subscript(safe i: Int) -> Element? { indices.contains(i) ? self[i] : nil }
}
