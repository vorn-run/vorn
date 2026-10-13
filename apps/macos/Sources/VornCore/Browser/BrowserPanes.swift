import Foundation

/// The artifact a tab shows, when it shows one.
public struct TabArtifact: Codable, Equatable, Sendable {
    public var id: String
    public var kind: String
    public var version: Int
    public var title: String
    public init(id: String, kind: String, version: Int, title: String) {
        self.id = id; self.kind = kind; self.version = version; self.title = title
    }
}

/// One tab: the address it was asked for, and what its page reported.
public struct BrowserTab: Codable, Equatable, Sendable {
    public var url: String
    public var liveUrl: String?
    public var title: String?
    public var artifact: TabArtifact?

    public init(url: String, liveUrl: String? = nil, title: String? = nil, artifact: TabArtifact? = nil) {
        self.url = url; self.liveUrl = liveUrl; self.title = title; self.artifact = artifact
    }

    /// Where the tab is: its page's last report, else where it was sent.
    public var current: String { liveUrl ?? url }
}

/// A session's browser pane.
public struct BrowserPane: Codable, Equatable, Sendable {
    public var tabs: [BrowserTab]
    public var activeTab: Int

    public init(tabs: [BrowserTab], activeTab: Int = 0) { self.tabs = tabs; self.activeTab = activeTab }

    public var active: BrowserTab? { tabs.indices.contains(activeTab) ? tabs[activeTab] : nil }
}

/// Every session's pane, and the tabs of the ones closed, for reopening.
public struct BrowserPanes: Codable, Equatable, Sendable {
    public var open: [String: BrowserPane] = [:]
    public var memory: [String: BrowserPane] = [:]
    /// A design's controls as last set, by the file it was loaded from.
    public var tweaks: [String: [String: JSONValue]] = [:]

    public init() {}

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        open = try c.decodeIfPresent([String: BrowserPane].self, forKey: .open) ?? [:]
        memory = try c.decodeIfPresent([String: BrowserPane].self, forKey: .memory) ?? [:]
        tweaks = try c.decodeIfPresent([String: [String: JSONValue]].self, forKey: .tweaks) ?? [:]
    }

    /// Only what was asked for is kept: what a page reported is seen again next run.
    public var intent: BrowserPanes {
        var out = self
        func strip(_ p: BrowserPane) -> BrowserPane {
            var p = p
            for i in p.tabs.indices { p.tabs[i].liveUrl = nil; p.tabs[i].title = nil }
            p.activeTab = min(p.activeTab, max(p.tabs.count - 1, 0))
            return p
        }
        out.open = open.mapValues(strip).filter { !$0.value.tabs.isEmpty }
        out.memory = memory.mapValues(strip)
        return out
    }

    public subscript(session: String) -> BrowserPane? { open[session] }

    /// With a vetted `url`, the active tab goes there; without, the pane opens as it was left, or blank.
    @discardableResult
    public mutating func open(_ session: String, url: String? = nil) -> Bool {
        switch (url, open[session]) {
        case (let url?, var pane?):
            if pane.tabs.indices.contains(pane.activeTab) { pane.tabs[pane.activeTab] = BrowserTab(url: url) }
            open[session] = pane
        case (let url?, nil):
            open[session] = BrowserPane(tabs: [BrowserTab(url: url)])
        case (nil, nil):
            open[session] = memory[session] ?? BrowserPane(tabs: [BrowserTab(url: BrowserURL.blank)])
        case (nil, _?):
            return false
        }
        return true
    }

    /// Its tabs are kept for the next open.
    @discardableResult
    public mutating func close(_ session: String) -> Bool {
        guard let pane = open.removeValue(forKey: session) else { return false }
        memory[session] = pane
        return true
    }

    public mutating func toggle(_ session: String) {
        if !close(session) { open(session) }
    }

    @discardableResult
    public mutating func addTab(_ session: String, url: String? = nil) -> Bool {
        guard var pane = open[session] else { return false }
        pane.tabs.append(BrowserTab(url: url ?? BrowserURL.blank))
        pane.activeTab = pane.tabs.count - 1
        open[session] = pane
        return true
    }

    /// The last tab takes the pane with it, and leaves nothing to reopen.
    @discardableResult
    public mutating func closeTab(_ session: String, index: Int) -> Bool {
        guard var pane = open[session], pane.tabs.indices.contains(index) else { return false }
        if pane.tabs.count == 1 {
            open[session] = nil
            memory[session] = nil
            return true
        }
        pane.tabs.remove(at: index)
        let active = index <= pane.activeTab ? max(pane.activeTab - 1, 0) : pane.activeTab
        pane.activeTab = min(active, pane.tabs.count - 1)
        open[session] = pane
        return true
    }

    @discardableResult
    public mutating func selectTab(_ session: String, index: Int) -> Bool {
        guard var pane = open[session], pane.tabs.indices.contains(index), pane.activeTab != index else { return false }
        pane.activeTab = index
        open[session] = pane
        return true
    }

    /// What the page in tab `index` reported. A new address drops the old page's title.
    @discardableResult
    public mutating func syncTab(_ session: String, index: Int, url: String?, title: String?) -> Bool {
        guard var pane = open[session], pane.tabs.indices.contains(index) else { return false }
        var tab = pane.tabs[index]
        let movedOn = url.map { $0 != tab.current } ?? false
        let liveUrl = url ?? tab.liveUrl
        let newTitle = title.flatMap { $0.isEmpty ? nil : $0 } ?? (movedOn ? nil : tab.title)
        if liveUrl == tab.liveUrl && newTitle == tab.title { return false }
        tab.liveUrl = liveUrl.flatMap { $0.isEmpty ? nil : $0 }
        tab.title = newTitle
        pane.tabs[index] = tab
        open[session] = pane
        return true
    }

    /// The artifact's tab is replaced in place, or added.
    public mutating func openArtifact(_ session: String, url: String, artifact: TabArtifact) {
        let tab = BrowserTab(url: url, artifact: artifact)
        guard var pane = open.removeValue(forKey: session) ?? memory[session] else {
            open[session] = BrowserPane(tabs: [tab])
            return
        }
        if let i = pane.tabs.firstIndex(where: { $0.artifact?.id == artifact.id }) {
            pane.tabs[i] = tab
            pane.activeTab = i
        } else {
            pane.tabs.append(tab)
            pane.activeTab = pane.tabs.count - 1
        }
        open[session] = pane
    }

    public mutating func setArtifactVersion(_ session: String, index: Int, url: String, version: Int) {
        guard var pane = open[session], pane.tabs.indices.contains(index), var artifact = pane.tabs[index].artifact else { return }
        artifact.version = version
        pane.tabs[index] = BrowserTab(url: url, artifact: artifact)
        open[session] = pane
    }

    /// The tabs as the agent's `list_tabs` reads them.
    public func tabInfos(_ session: String) -> [JSONValue] {
        guard let pane = open[session] else { return [] }
        return pane.tabs.enumerated().map { i, t in
            var o: [String: JSONValue] = ["index": .number(Double(i)), "url": .string(t.current), "active": .bool(i == pane.activeTab)]
            if let title = t.title { o["title"] = .string(title) }
            return .object(o)
        }
    }
}
