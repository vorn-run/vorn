import AppKit
import Foundation
import Observation
import SwiftUI
import VornCore
import VornUI

/// A comment waiting on its words in the popover.
struct PendingComment: Equatable {
    var anchor: ArtifactAnchor
    var label: String
    var at: CGPoint
}

/// A doc being edited: what it was, and what it says now.
struct DocDraft: Equatable {
    var original: String
    var edited: String
}

/// A session's browser pane state that is not saved (BrowserCard's own state).
struct BrowserLive: Equatable {
    var draft: String?
    var failed: String?
    var loading = false
    var picking = false
    var manifest: Manifest?
    var values: [String: JSONValue] = [:]
    var board: String?
    /// The canvas zoom; nil fits every artboard.
    var zoom: Double?
    var commenting = false
    var sending = false
    var comparing = false
    var compareUrl: String?
    /// The `id:version` whose banner was dismissed.
    var dismissed: String?
    var editing: DocDraft?
    var saving = false
    var pending: PendingComment?
    var found: [String: Bool] = [:]
    var focus: String?
    var selection: (anchor: ArtifactAnchor, rect: CGRect, since: Date)?
    var console: [JSONValue] = []
    var network: [JSONValue] = []
    var versions = false
    var railEdit: String?
    var nav: (back: Bool, forward: Bool) = (false, false)
    var refreshed: Set<String> = []

    static func == (a: BrowserLive, b: BrowserLive) -> Bool {
        a.draft == b.draft && a.failed == b.failed && a.loading == b.loading && a.picking == b.picking && a.manifest == b.manifest
            && a.values == b.values && a.board == b.board && a.zoom == b.zoom && a.commenting == b.commenting
            && a.sending == b.sending && a.comparing == b.comparing && a.compareUrl == b.compareUrl && a.dismissed == b.dismissed
            && a.editing == b.editing && a.saving == b.saving && a.pending == b.pending && a.found == b.found && a.focus == b.focus
            && a.selection?.anchor == b.selection?.anchor && a.selection?.since == b.selection?.since
            && a.console == b.console && a.network == b.network && a.versions == b.versions && a.railEdit == b.railEdit
            && a.nav == b.nav && a.refreshed == b.refreshed
    }
}

/// A session's device pane (DeviceCard's own state).
struct DevicePaneState: Equatable {
    var error: String?
    var saving = false
    /// The frame's size against fitting the pane; nil fits.
    var zoom: Double?
}

/// The device picker over a card.
struct DevicePicker: Equatable {
    var session: String
    var fromPane: Bool
    var loading = true
    var devices: [Devices.Listed] = []
    var error: String?
}

/// The latest screen of a session's simulator.
struct DeviceFrame {
    var image: NSImage
    var pixels: CGSize
}

/// Where the simulators come from; renders and tests swap it.
public struct DeviceBackend: Sendable {
    public var list: @Sendable ([String: String]) async throws -> [Device]
    public var claim: @Sendable (String, String, [String: String]) async throws -> Devices.Claimed
    public var shutdown: @Sendable (String) async -> Void
    public var screenshot: @Sendable (String) async throws -> Data

    public static let simctl = DeviceBackend(list: { try await Devices.list(claims: $0) },
                                             claim: { try await Devices.claim($0, session: $1, claims: $2) },
                                             shutdown: { await Devices.shutdown($0) },
                                             screenshot: { try await Devices.screenshot($0) })

    public init(list: @escaping @Sendable ([String: String]) async throws -> [Device],
                claim: @escaping @Sendable (String, String, [String: String]) async throws -> Devices.Claimed,
                shutdown: @escaping @Sendable (String) async -> Void, screenshot: @escaping @Sendable (String) async throws -> Data) {
        self.list = list
        self.claim = claim
        self.shutdown = shutdown
        self.screenshot = screenshot
    }
}

/// The browser and device panes: their tabs, pages, artifacts and simulators, and the agents' bridge to them.
@MainActor
@Observable
public final class BrowserModel {
    public private(set) var panes = BrowserPanes()
    var live: [String: BrowserLive] = [:]
    var artifacts: [String: ArtifactState] = [:]
    var devices: [String: DevicePaneState] = [:]
    var claims: [String: Devices.Claimed] = [:]
    var claiming: Set<String> = []
    var picker: DevicePicker?
    var mobile: [String: Bool] = [:]
    var frames: [String: DeviceFrame] = [:]
    /// Bumped when a page view's history moves, so the arrows redraw.
    var navTick = 0

    @ObservationIgnored let host = WebHost()
    @ObservationIgnored public var client: VornClient?
    @ObservationIgnored public var session: (String) -> TerminalSession? = { _ in nil }
    @ObservationIgnored public var liveSessions: () -> [TerminalSession] = { [] }
    @ObservationIgnored public var write: (String, String) -> Void = { _, _ in }
    @ObservationIgnored public var toast: (String, Bool) -> Void = { _, _ in }
    @ObservationIgnored public var deviceBackend = DeviceBackend.simctl
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let persists: Bool
    @ObservationIgnored private var painted: [String: String] = [:]
    @ObservationIgnored private var asked: Set<String> = []
    @ObservationIgnored private var watching: [String: (path: String, modified: Date?, checked: Date)] = [:]
    @ObservationIgnored private var pollers: [String: (udid: String, task: Task<Void, Never>)] = [:]
    @ObservationIgnored private var ticker: Task<Void, Never>?
    @ObservationIgnored private var listener: Task<Void, Never>?
    /// The size of each page view, for placing the popover.
    @ObservationIgnored var areas: [String: CGSize] = [:]
    /// The size of each device stage, for fitting the frame.
    @ObservationIgnored var deviceAreas: [String: CGSize] = [:]

    public init(defaults: UserDefaults = .standard, persists: Bool = true, ephemeral: Bool = false) {
        self.defaults = defaults
        self.persists = persists
        host.ephemeral = ephemeral || !persists
        if persists, let d = defaults.data(forKey: "vorn.browserPanes"), let v = try? JSONDecoder().decode(BrowserPanes.self, from: d) {
            panes = v
        }
        host.onEvent = { [weak self] e in self?.webEvent(e) }
        host.onNav = { [weak self] key in
            guard let self, key == WebHost.sessionOf(key) else { return }
            let nav = host.nav(key)
            if let old = live[key]?.nav, old == nav {} else { live[key, default: BrowserLive()].nav = nav }
        }
    }

    private func save() {
        guard persists else { return }
        defaults.set(try? JSONEncoder().encode(panes.intent), forKey: "vorn.browserPanes")
    }

    /// Starts answering the agents' browser and device tools, and listening for artifact changes.
    public func start(client: VornClient, answerAgents: Bool) {
        self.client = client
        listener?.cancel()
        listener = Task { [weak self] in
            for await event in await client.events(["artifact:published", "artifact:commentsChanged"]) {
                guard case .notification(let method, let params) = event else { continue }
                self?.notified(method, params)
            }
        }
        if answerAgents {
            Task { await client.serveRequests { [weak self] method, params in
                guard let self else { return .failure(VornRequestError("Vorn app is not running")) }
                return await self.bridge(method, params)
            } }
        }
        startTicking()
        syncViews()
    }

    private func startTicking() {
        guard ticker == nil else { return }
        ticker = Task { [weak self] in
            while !Task.isCancelled {
                self?.pump()
                try? await Task.sleep(for: .milliseconds(100))
            }
        }
    }

    // MARK: Reading

    func liveState(_ s: String) -> BrowserLive { live[s] ?? BrowserLive() }

    private func mutate(_ s: String, _ f: (inout BrowserLive) -> Void) {
        var l = live[s] ?? BrowserLive()
        f(&l)
        if live[s] != l { live[s] = l }
    }

    func root(_ s: String) -> String? {
        guard let info = session(s) else { return nil }
        return info.worktreePath ?? info.projectPath
    }

    func art(_ s: String) -> TabArtifact? { panes[s]?.active?.artifact }

    func artState(_ s: String) -> ArtifactState? { art(s).flatMap { artifacts[$0.id] } }

    /// The design manifest with artboards the session's page declared.
    func boards(_ s: String) -> [Artboard]? {
        guard let m = live[s]?.manifest, m.kind == "design", !m.artboards.isEmpty else { return nil }
        return m.artboards
    }

    func board(_ s: String) -> Artboard? {
        guard let boards = boards(s) else { return nil }
        return boards.first { $0.id == live[s]?.board } ?? boards.first
    }

    func canvasZoom(_ s: String, area: CGSize?) -> Double {
        if let z = live[s]?.zoom { return z }
        if let b = boards(s), let area { return DesignCanvas.fitZoom(b, area: area) }
        return 0.5
    }

    /// The session's unsent comments and the last batch sent.
    func comments(_ s: String) -> (drafts: [ArtifactComment], sent: [ArtifactComment]) {
        guard let st = artState(s) else { return ([], []) }
        return (st.comments.filter(\.isDraft), Artifacts.latestSentBatch(st.comments))
    }

    func holders() -> [String: String] { Dictionary(claims.map { ($0.value.udid, $0.key) }, uniquingKeysWith: { a, _ in a }) }

    func deviceScale(_ s: String, area: CGSize?) -> (scale: Double, points: CGSize) {
        let points = frames[s].map { f -> CGSize in
            let k = Devices.pointScale(width: Int(f.pixels.width), height: Int(f.pixels.height))
            return CGSize(width: f.pixels.width / k, height: f.pixels.height / k)
        } ?? .zero
        let scale = devices[s]?.zoom ?? area.map { Devices.fitScale(points: points, area: $0) } ?? 1
        return (scale, points)
    }

    /// Every view key the session's browser has.
    private func sessionKeys(_ s: String) -> [String] {
        [s, "\(s)#compare"] + (boards(s) ?? []).map { "\(s)#board:\($0.id)" }
    }

    // MARK: The views

    /// Creates and loads each pane's views, and drops those whose pane went.
    func syncViews() {
        for (s, pane) in panes.open {
            guard let tab = pane.active else { continue }
            let root = root(s)
            host.page(s, url: tab.url, root: root)
            if let boards = boards(s), live[s]?.editing == nil {
                for b in boards { host.page("\(s)#board:\(b.id)", url: DesignCanvas.artboardURL(tab.url, id: b.id), root: root) }
            }
            if let l = live[s], l.comparing, l.editing == nil, let url = l.compareUrl {
                host.page("\(s)#compare", url: url, root: root)
            }
        }
        host.close { key in
            let s = WebHost.sessionOf(key)
            guard panes[s] != nil else { return true }
            if key.hasSuffix("#compare") { return !(live[s]?.comparing ?? false) || live[s]?.compareUrl == nil }
            if let r = key.range(of: "#board:") { return !(boards(s) ?? []).contains { $0.id == String(key[r.upperBound...]) } }
            return false
        }
        if !panes.open.isEmpty || !devices.isEmpty { startTicking() }
    }

    /// What arrives with the clock: settled selections, marks, watched files, artifact refreshes and frame pollers.
    func pump() {
        settleSelections()
        paintMarks()
        watchFiles()
        refreshArtifactTabs()
        syncPollers()
    }

    // MARK: Pane controls

    public func isOpen(_ s: String) -> Bool { panes[s] != nil }
    public func hasDevicePane(_ s: String) -> Bool { devices[s] != nil }

    public func toggle(_ s: String) {
        panes.toggle(s)
        save()
        syncViews()
    }

    public func close(_ s: String) {
        _ = panes.close(s)
        save()
        syncViews()
    }

    public func open(_ s: String, url: String?) {
        _ = panes.open(s, url: url)
        save()
        syncViews()
    }

    func addTab(_ s: String) {
        if panes.addTab(s) { tabChanged(s); save() }
        syncViews()
    }

    func closeTab(_ s: String, _ i: Int) {
        let was = panes[s]?.activeTab
        if panes.closeTab(s, index: i) {
            if panes[s]?.activeTab != was || was == i { tabChanged(s) }
            save()
        }
        syncViews()
    }

    func selectTab(_ s: String, _ i: Int) {
        if panes.selectTab(s, index: i) { tabChanged(s); save() }
        syncViews()
    }

    func back(_ s: String) { _ = host.step(s, back: true) }
    func forward(_ s: String) { _ = host.step(s, back: false) }

    func reloadOrStop(_ s: String) {
        if live[s]?.loading == true {
            host.stop(s)
            mutate(s) { $0.loading = false }
        } else {
            host.reload(s)
        }
    }

    func setDraft(_ s: String, _ text: String?) { mutate(s) { $0.draft = text } }

    func submitAddress(_ s: String, _ text: String) {
        guard let url = BrowserURL.normalize(text, allowFile: false) else {
            mutate(s) { $0.failed = "That does not look like a web address" }
            return
        }
        mutate(s) {
            $0.failed = nil
            $0.draft = nil
        }
        open(s, url: url)
        host.load(s, url, root: root(s))
    }

    func pick(_ s: String) {
        mutate(s) { $0.picking.toggle() }
        host.run(s, "window.__vorn.pins(\(live[s]?.picking ?? false), [])")
        painted[s] = nil
    }

    func ink(_ s: String) { mutate(s) { $0.failed = BrowserBridge.noInk } }

    func toggleVersions(_ s: String) { mutate(s) { $0.versions.toggle() } }

    func chooseVersion(_ s: String, _ v: Int) {
        mutate(s) { $0.versions = false }
        if let i = panes[s]?.activeTab, let a = art(s) { showVersion(s, index: i, id: a.id, version: v) }
    }

    func toggleComments(_ s: String) {
        if live[s]?.commenting == true { dropSelection(s) }
        mutate(s) { $0.commenting.toggle() }
    }

    func send(_ s: String) {
        guard let a = art(s), let client else { return }
        mutate(s) { $0.sending = true }
        Task {
            do { _ = try await client.call("artifact:send", ["artifactId": .string(a.id)]) } catch {
                mutate(s) { $0.failed = Self.fail(error, "Could not send the comments") }
            }
            mutate(s) { $0.sending = false }
            await refreshArtifact(a.id)
        }
    }

    func edit(_ s: String) {
        guard let a = art(s), let client else { return }
        Task {
            let v = try? await client.call("artifact:readSource", ["artifactId": .string(a.id), "version": .number(Double(a.version))])
            if let body = v?["body"]?.stringValue {
                mutate(s) {
                    $0.commenting = false
                    $0.editing = DocDraft(original: body, edited: body)
                }
                dropSelection(s)
            } else {
                mutate(s) { $0.failed = "Could not read the doc" }
            }
            syncViews()
        }
    }

    func setEdited(_ s: String, _ text: String) { mutate(s) { $0.editing?.edited = text } }

    func discard(_ s: String) {
        mutate(s) { $0.editing = nil }
        syncViews()
    }

    func saveDoc(_ s: String, send: Bool) {
        guard let a = art(s), let index = panes[s]?.activeTab, let draft = live[s]?.editing, let client else { return }
        let (body, edits) = Artifacts.mergeDocEdit(original: draft.original, edited: draft.edited)
        guard !edits.isEmpty else { return }
        mutate(s) { $0.saving = true }
        Task {
            do {
                let v = try await client.call("artifact:saveUserVersion", ["artifactId": .string(a.id), "body": .string(body),
                                                                           "edits": .array(edits.map(\.json)), "send": .bool(send)])
                mutate(s) {
                    $0.saving = false
                    $0.editing = nil
                }
                if let n = v["version"]?["version"]?.numberValue.map({ Int($0) }) {
                    await showVersionNow(s, index: index, id: a.id, version: n)
                    if let e = v["sendError"]?.stringValue { mutate(s) { $0.failed = "Saved v\(n), not sent: \(e)" } }
                }
                await refreshArtifact(a.id)
            } catch {
                mutate(s) {
                    $0.saving = false
                    $0.failed = Self.fail(error, "Could not save the doc")
                }
            }
            syncViews()
        }
    }

    func openLatest(_ s: String) {
        guard let i = panes[s]?.activeTab, let st = artState(s) else { return }
        showVersion(s, index: i, id: st.artifact.id, version: st.artifact.latestVersion)
    }

    func toggleCompare(_ s: String) {
        mutate(s) {
            $0.comparing.toggle()
            $0.compareUrl = nil
        }
        syncViews()
        guard live[s]?.comparing == true, let a = art(s), let on = artifacts[a.id]?.answered(a.version).on, let client else { return }
        Task {
            let url = try? await client.call("artifact:versionUrl", ["artifactId": .string(a.id), "version": .number(Double(on))])
            mutate(s) { $0.compareUrl = url?["url"]?.stringValue }
            syncViews()
        }
    }

    func dismissBanner(_ s: String) {
        if let a = art(s) { mutate(s) { $0.dismissed = "\(a.id):\(a.version)" } }
    }

    /// `applyTweak`: shown at once, kept for the design's file, and written into every view of the page.
    func applyTweak(_ s: String, key: String, value: JSONValue) {
        if let path = panes[s]?.active.flatMap({ BrowserURL.filePath($0.current) }) {
            panes.tweaks[path, default: [:]][key] = value
            save()
        }
        mutate(s) { $0.values[key] = value }
        let script = "window.__vorn.tweak(\(BrowserBridge.js([key: value])))"
        for k in sessionKeys(s) { host.run(k, script) }
    }

    func selectBoard(_ s: String, _ id: String) { mutate(s) { $0.board = id } }

    /// ±1 steps the canvas zoom by a tenth; 0 fits.
    func zoom(_ s: String, _ dir: Int, area: CGSize?) {
        let z = canvasZoom(s, area: area)
        mutate(s) { $0.zoom = dir == 0 ? nil : DesignCanvas.clampZoom(z + 0.1 * Double(dir)) }
        syncViews()
    }

    func railEdit(_ s: String, _ id: String?) { mutate(s) { $0.railEdit = id } }

    func railSave(_ s: String, _ body: String) {
        let id = live[s]?.railEdit
        mutate(s) { $0.railEdit = nil }
        let body = body.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let id, let a = art(s), !body.isEmpty else { return }
        commentCall(s, a.id, "artifact:updateComment", ["commentId": .string(id), "body": .string(body)], error: nil)
    }

    func railDelete(_ s: String, _ id: String) {
        guard let a = art(s) else { return }
        commentCall(s, a.id, "artifact:deleteComment", ["commentId": .string(id)], error: nil)
    }

    func addNote(_ s: String, _ body: String) {
        let body = body.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let a = art(s), !body.isEmpty else { return }
        saveComment(s, a, anchor: nil, body: body)
    }

    /// Escape closes the versions menu.
    func closeMenus(_ s: String) { mutate(s) { $0.versions = false } }

    // MARK: Artifacts

    private static func fail(_ error: Error, _ fallback: String) -> String {
        if case VornError.rpc(_, let m) = error, !m.isEmpty { return m }
        return fallback
    }

    private func saveComment(_ s: String, _ a: TabArtifact, anchor: ArtifactAnchor?, body: String) {
        commentCall(s, a.id, "artifact:saveComment",
                    ["artifactId": .string(a.id), "version": .number(Double(a.version)), "anchor": anchor?.json ?? .null, "body": .string(body)],
                    error: anchor == nil ? "Could not save the note" : "Could not save the comment")
    }

    private func commentCall(_ s: String, _ id: String, _ method: String, _ params: JSONValue, error failure: String?) {
        guard let client else { return }
        Task {
            do {
                _ = try await client.call(method, params)
                await refreshArtifact(id)
            } catch {
                if let failure { mutate(s) { $0.failed = failure } } else { await refreshArtifact(id) }
            }
        }
    }

    func refreshArtifact(_ id: String) async {
        guard let client, let v = try? await client.call("artifact:get", ["artifactId": .string(id)]), let st = ArtifactState(v) else { return }
        if artifacts[id] != st { artifacts[id] = st }
    }

    /// `showVersion`: tab `index` goes to `version` once its address is known.
    func showVersion(_ s: String, index: Int, id: String, version: Int) {
        Task { await showVersionNow(s, index: index, id: id, version: version) }
    }

    private func showVersionNow(_ s: String, index: Int, id: String, version: Int) async {
        guard let client,
              let url = try? await client.call("artifact:versionUrl", ["artifactId": .string(id), "version": .number(Double(version))])["url"]?.stringValue
        else { return }
        let before = panes[s]
        panes.setArtifactVersion(s, index: index, url: url, version: version)
        if let a = art(s) { mutate(s) { $0.refreshed.insert("\(a.id):\(a.version)") } }
        if panes[s] != before {
            if panes[s]?.activeTab == index { tabChanged(s) }
            save()
        }
        syncViews()
    }

    func notified(_ method: String, _ params: JSONValue) {
        let id = method == "artifact:published" ? params["artifact"]?["id"]?.stringValue : params["artifactId"]?.stringValue
        if let id, artifacts[id] != nil { Task { await refreshArtifact(id) } }
    }

    /// An artifact's address carries the server's port: each tab's version is asked for again once, and each artifact fetched.
    private func refreshArtifactTabs() {
        for (s, pane) in panes.open {
            for (i, t) in pane.tabs.enumerated() {
                guard let a = t.artifact else { continue }
                let tag = "\(a.id):\(a.version)"
                if !(live[s]?.refreshed.contains(tag) ?? false) {
                    mutate(s) { $0.refreshed.insert(tag) }
                    showVersion(s, index: i, id: a.id, version: a.version)
                }
                if asked.insert(a.id).inserted { Task { await refreshArtifact(a.id) } }
            }
        }
    }

    /// A tab switch or another version: what the old page showed goes.
    private func tabChanged(_ s: String) {
        mutate(s) {
            $0.manifest = nil
            $0.values = [:]
            $0.editing = nil
            $0.comparing = false
            $0.compareUrl = nil
            $0.pending = nil
            $0.selection = nil
            $0.picking = false
            $0.draft = nil
            $0.failed = nil
            $0.versions = false
        }
        watching[s] = nil
    }

    // MARK: Page events

    func webEvent(_ e: WebEvent) {
        switch e {
        case .page(let key, let url, let finished):
            painted[key] = nil
            let s = WebHost.sessionOf(key)
            if key == s {
                if !url.isEmpty, let i = panes[s]?.activeTab, panes.syncTab(s, index: i, url: url, title: nil) { save() }
                let nav = host.nav(key)
                mutate(s) {
                    $0.loading = !finished
                    $0.draft = nil
                    if finished {
                        $0.nav = nav
                        $0.found = [:]
                    } else {
                        $0.failed = nil
                        $0.manifest = nil
                        $0.values = [:]
                    }
                }
                if finished {
                    Task {
                        let read = await host.eval(key, "JSON.stringify(window.__vorn.manifest())")
                        manifestRead(s, key: key, read: try? read.get())
                    }
                }
            } else if finished, key.contains("#board:") {
                host.run(key, "window.__vorn.tweak(\(BrowserBridge.js(.object(live[s]?.values ?? [:]))))")
            }
        case .title(let key, let title):
            let s = WebHost.sessionOf(key)
            if key == s, let i = panes[s]?.activeTab, panes.syncTab(s, index: i, url: nil, title: title) { save() }
        case .message(let key, let body):
            if let data = body.data(using: .utf8), let msg = try? JSONDecoder().decode(JSONValue.self, from: data) {
                pageMessage(key, msg)
            }
        case .newWindow(let key, let url):
            let s = WebHost.sessionOf(key)
            if let url = BrowserURL.loadable(url, root: root(s)), panes.addTab(s, url: url) {
                tabChanged(s)
                save()
                syncViews()
            }
        case .failed(let key, let message):
            if key == WebHost.sessionOf(key) { mutate(key) { $0.failed = message } }
        }
    }

    /// What the page declares itself to be: a design's controls are put back as they were set for its file.
    private func manifestRead(_ s: String, key: String, read: JSONValue?) {
        let declared = read?["declared"]?.stringValue ?? ""
        let held = read?["live"]?.objectValue
        guard let m = Manifest.parse(declared) else {
            mutate(s) {
                $0.manifest = nil
                $0.values = [:]
            }
            watching[s] = nil
            return
        }
        let path = panes[s]?.active.flatMap { BrowserURL.filePath($0.current) }
        let stored = path.flatMap { panes.tweaks[$0] } ?? [:]
        let merged = Manifest.mergeTweaks(m.tweaks, stored: stored, live: held)
        let push = merged.filter { held?[$0.key] != $0.value }
        if !push.isEmpty { host.run(key, "window.__vorn.tweak(\(BrowserBridge.js(.object(push))))") }
        if let path, m.kind == "design" {
            watching[s] = (path, Self.modified(path), Date())
        } else {
            watching[s] = nil
        }
        mutate(s) {
            $0.values = merged
            $0.manifest = m
        }
        syncViews()
    }

    private static func modified(_ path: String) -> Date? {
        (try? FileManager.default.attributesOfItem(atPath: path))?[.modificationDate] as? Date
    }

    /// What the page script posted: its console and requests, the selection, the popover, a pin, a pick.
    private func pageMessage(_ key: String, _ msg: JSONValue) {
        let s = WebHost.sessionOf(key)
        let isPage = key == s
        switch msg["t"]?.stringValue ?? "" {
        case "console" where isPage, "network" where isPage:
            var entry = msg.objectValue ?? [:]
            let console = entry.removeValue(forKey: "t") == "console"
            mutate(s) {
                if console {
                    $0.console.append(.object(entry))
                    if $0.console.count > 500 { $0.console.removeFirst($0.console.count - 500) }
                } else {
                    $0.network.append(.object(entry))
                    if $0.network.count > 500 { $0.network.removeFirst($0.network.count - 500) }
                }
            }
        case "selection" where isPage:
            let l = liveState(s)
            guard l.commenting, l.pending == nil, boards(s) == nil else { return }
            let quote = msg["quote"]?.stringValue ?? ""
            guard !quote.isEmpty else {
                mutate(s) { $0.selection = nil }
                return
            }
            let anchor = ArtifactAnchor.quote(quote: quote, prefix: msg["prefix"]?.stringValue ?? "", suffix: msg["suffix"]?.stringValue ?? "")
            func n(_ k: String) -> Double { msg["rect"]?[k]?.numberValue ?? 0 }
            let rect = CGRect(x: n("x"), y: n("y"), width: n("width"), height: n("height"))
            let since = l.selection.flatMap { $0.anchor == anchor ? $0.since : nil } ?? Date()
            mutate(s) { $0.selection = (anchor, rect, since) }
        case "comment":
            let body = (msg["body"]?.stringValue ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            let pending = live[s]?.pending
            mutate(s) { $0.pending = nil }
            dropSelection(s)
            if let p = pending, let a = art(s), !body.isEmpty { saveComment(s, a, anchor: p.anchor, body: body) }
        case "cancel":
            dropSelection(s)
        case "pin":
            if let id = msg["id"]?.stringValue { reveal(s, id) }
        case "point" where isPage:
            guard live[s]?.picking == true else { return }
            mutate(s) { $0.picking = false }
            host.run(key, "window.__vorn.pins(false, [])")
            write(s, BrowserBridge.pickMessage(msg))
        case "point":
            pinPoint(s, key: key, msg)
        default:
            break
        }
    }

    /// A click on an artboard while commenting: the popover asks what to say.
    private func pinPoint(_ s: String, key: String, _ msg: JSONValue) {
        guard let r = key.range(of: "#board:") else { return }
        let board = String(key[r.upperBound...])
        let l = liveState(s)
        guard l.commenting, l.pending == nil else { return }
        let x = msg["x"]?.numberValue ?? 0, y = msg["y"]?.numberValue ?? 0
        let element = msg["element"]?.stringValue ?? "element"
        let found = boards(s)?.first { $0.id == board }
        let label = "\(found?.label ?? board) \u{b7} \(element)"
        let area = found.map { CGSize(width: $0.width, height: $0.height) } ?? CGSize(width: CGFloat.greatestFiniteMagnitude, height: .greatestFiniteMagnitude)
        let at = Artifacts.placePopover(CGRect(x: x, y: y, width: 0, height: 0), area: area)
        host.run(key, popoverScript(label, at))
        mutate(s) { $0.pending = PendingComment(anchor: .point(artboard: board, x: x, y: y, element: element), label: label, at: at) }
    }

    private func popoverScript(_ label: String, _ at: CGPoint) -> String {
        "window.__vorn.popover(\(BrowserBridge.js(.string(label))), \(Self.num(at.x)), \(Self.num(at.y)), \(Self.num(Artifacts.popoverWidth)))"
    }

    private static func num(_ v: Double) -> String { v == v.rounded() ? String(Int(v)) : String(v) }

    /// A selection that held still for 400 ms opens the popover over it.
    private func settleSelections() {
        let now = Date()
        for (s, l) in live where l.pending == nil {
            guard let sel = l.selection, now.timeIntervalSince(sel.since) >= 0.4, case .quote(let quote, _, _) = sel.anchor else { continue }
            let at = Artifacts.placePopover(sel.rect, area: areas[s] ?? CGSize(width: 800, height: 600))
            mutate(s) {
                $0.selection = nil
                $0.pending = PendingComment(anchor: sel.anchor, label: quote, at: at)
            }
            host.run(s, popoverScript(quote, at))
        }
    }

    /// `dropSelection`: the popover goes, and the page's own selection.
    func dropSelection(_ s: String) {
        mutate(s) {
            $0.pending = nil
            $0.selection = nil
        }
        for k in sessionKeys(s) { host.run(k, "window.__vorn.clear()") }
        painted = painted.filter { WebHost.sessionOf($0.key) != s }
    }

    func reveal(_ s: String, _ id: String) {
        let (drafts, sent) = comments(s)
        let point: String? = (drafts + sent).first { $0.id == id }.flatMap {
            if case .point(let board, _, _, _) = $0.anchor { return board }
            return nil
        }
        mutate(s) {
            $0.focus = id
            if let point { $0.board = point }
        }
        if point == nil { host.run(s, "window.__vorn.reveal(\(BrowserBridge.js(.string(id))))") }
    }

    /// The comments' marks on an artifact page, and pins on artboards, sent whenever they change.
    private func paintMarks() {
        for s in panes.open.keys where art(s) != nil {
            let l = liveState(s)
            let pinning = l.commenting && l.pending == nil
            let (drafts, sent) = comments(s)
            if let boards = boards(s) {
                var pins: [(String, JSONValue)] = []
                var n = 0
                for c in drafts + sent {
                    guard case .point(let board, let x, let y, _) = c.anchor else { continue }
                    n += 1
                    let state = l.focus == c.id ? "focus" : c.isDraft ? "draft" : "sent"
                    pins.append((board, ["id": .string(c.id), "n": .number(Double(n)), "x": .number(x), "y": .number(y), "state": .string(state)]))
                }
                for b in boards {
                    let key = "\(s)#board:\(b.id)"
                    let list = pins.filter { $0.0 == b.id && l.commenting }.map(\.1)
                    let script = "window.__vorn.pins(\(pinning), \(BrowserBridge.js(.array(list))))"
                    if host.has(key), painted[key] != script {
                        host.run(key, script)
                        painted[key] = script
                    }
                }
            } else {
                let marks = l.commenting ? Artifacts.marksFor(drafts: drafts, sent: sent, focus: l.focus) : []
                let script = "JSON.stringify(window.__vorn.marks(\(BrowserBridge.js(.array(marks.map(\.json))))))"
                guard host.has(s), painted[s] != script else { continue }
                painted[s] = script
                Task {
                    if case .success(let v) = await host.eval(s, script), let o = v.objectValue {
                        let found = o.mapValues { $0.boolValue ?? false }
                        mutate(s) { $0.found = found }
                    }
                }
            }
        }
    }

    /// A design that changed on disk is shown again.
    private func watchFiles() {
        let now = Date()
        for (s, w) in watching where now.timeIntervalSince(w.checked) >= 0.5 {
            let m = Self.modified(w.path)
            watching[s] = (w.path, m, now)
            if m != w.modified { for k in sessionKeys(s) { host.reload(k) } }
        }
    }

    // MARK: Devices

    /// The card's phone button: closes the device pane, else opens the picker.
    public func deviceButton(_ s: String) {
        if devices.removeValue(forKey: s) != nil { return }
        openPicker(s, fromPane: false)
    }

    func switchDevice(_ s: String) {
        if picker != nil { picker = nil } else { openPicker(s, fromPane: true) }
    }

    func closeDevicePane(_ s: String) { devices[s] = nil }

    /// Whether the picker is open for `s` from the pane, or from the card's button.
    public func pickerBinding(_ s: String, fromPane: Bool) -> Binding<Bool> {
        Binding(get: { [weak self] in self?.picker.map { $0.session == s && $0.fromPane == fromPane } ?? false },
                set: { [weak self] open in if !open, self?.picker?.session == s, self?.picker?.fromPane == fromPane { self?.picker = nil } })
    }

    func openPicker(_ s: String, fromPane: Bool) {
        picker = DevicePicker(session: s, fromPane: fromPane)
        let holders = holders(), backend = deviceBackend
        Task {
            let result: Result<[Device], Error>
            do { result = .success(try await backend.list(holders)) } catch { result = .failure(error) }
            guard picker?.session == s else { return }
            picker?.loading = false
            switch result {
            case .success(let d): picker?.devices = Devices.order(d, session: s)
            case .failure(let e): picker?.error = (e as? DeviceError)?.message ?? e.localizedDescription
            }
        }
    }

    func pickDevice(_ s: String, _ udid: String) {
        picker = nil
        if claims[s]?.udid == udid {
            if devices[s] == nil { devices[s] = DevicePaneState() }
        } else {
            Task { _ = await claim(s, udid, open: true, reply: false) }
        }
    }

    /// Takes `udid` for `s`; `open` shows its pane once it is held. With `reply`, the bridge's answer.
    func claim(_ s: String, _ udid: String, open: Bool, reply: Bool) async -> Result<JSONValue, VornRequestError> {
        let holders = holders()
        if let holder = holders[udid], holder != s {
            let message = "\(udid) is in use by session \(holder)."
            if !reply { toast(message, true) }
            return open ? .failure(VornRequestError(message))
                : .success(["ok": false, "reason": "held-by-session", "message": .string(message), "holder": .string(holder)])
        }
        claiming.insert(s)
        defer { claiming.remove(s) }
        do {
            let c = try await deviceBackend.claim(udid, s, holders)
            if let old = claims[s], old.udid != c.udid, old.bootedHere {
                let backend = deviceBackend
                Task.detached { await backend.shutdown(old.udid) }
            }
            claims[s] = c
            if open, devices[s] == nil { devices[s] = DevicePaneState() }
            syncPollers()
            return .success(open ? ["udid": .string(c.udid)] : ["ok": true, "udid": .string(c.udid), "name": .string(c.name), "booted": true])
        } catch {
            let message = (error as? DeviceError)?.message ?? error.localizedDescription
            if !reply {
                if devices[s] != nil { devices[s]?.error = message } else { toast(message, true) }
            }
            return open ? .failure(VornRequestError(message)) : .success(BrowserBridge.claimFailure(message))
        }
    }

    /// Lets the session's simulator go, shutting it down when the app booted it.
    func release(_ s: String) -> Bool {
        devices[s] = nil
        guard let c = claims.removeValue(forKey: s) else { return false }
        if c.bootedHere {
            let backend = deviceBackend
            Task.detached { await backend.shutdown(c.udid) }
        }
        syncPollers()
        return true
    }

    func dismissDeviceError(_ s: String) { devices[s]?.error = nil }

    /// A device pane control: zoom, fit, actual size and save are here; the rest need the companion.
    func deviceControl(_ s: String, _ op: String, area: CGSize?) {
        guard devices[s] != nil else { return }
        let scale = deviceScale(s, area: area).scale
        switch op {
        case "zoom-in": devices[s]?.zoom = Devices.steppedZoom(scale, inward: true)
        case "zoom-out": devices[s]?.zoom = Devices.steppedZoom(scale, inward: false)
        case "fit": devices[s]?.zoom = nil
        case "actual": devices[s]?.zoom = 1
        case "save":
            guard let c = claims[s] else {
                devices[s]?.error = BrowserBridge.noClaim
                return
            }
            devices[s]?.saving = true
            let backend = deviceBackend
            Task {
                do {
                    let png = try await backend.screenshot(c.udid)
                    let home = URL(fileURLWithPath: NSHomeDirectory())
                    let down = home.appendingPathComponent("Downloads")
                    var isDir: ObjCBool = false
                    let dir = FileManager.default.fileExists(atPath: down.path, isDirectory: &isDir) && isDir.boolValue ? down : home
                    let iso = ISO8601DateFormatter.vornMillis.string(from: Date())
                    try png.write(to: dir.appendingPathComponent(Devices.screenshotName(c.name, iso: iso)))
                    devices[s]?.saving = false
                    toast("Screenshot saved", false)
                } catch {
                    devices[s]?.saving = false
                    devices[s]?.error = (error as? DeviceError)?.message ?? error.localizedDescription
                }
            }
        default:
            devices[s]?.error = BrowserBridge.noCompanion
        }
    }

    /// One frame poller per shown device; a pane that closed or switched simulators stops its own.
    private func syncPollers() {
        var wanted: [String: String] = [:]
        for s in devices.keys { if let c = claims[s] { wanted[s] = c.udid } }
        for (s, p) in pollers where wanted[s] != p.udid {
            p.task.cancel()
            pollers[s] = nil
            if wanted[s] == nil { frames[s] = nil }
        }
        for (s, udid) in wanted where pollers[s] == nil {
            let backend = deviceBackend
            let task = Task { [weak self] in
                while !Task.isCancelled {
                    if let png = try? await backend.screenshot(udid), let size = Devices.pngSize(png), let image = NSImage(data: png) {
                        guard !Task.isCancelled else { return }
                        self?.frames[s] = DeviceFrame(image: image, pixels: CGSize(width: size.width, height: size.height))
                    }
                    try? await Task.sleep(for: .milliseconds(250))
                }
            }
            pollers[s] = (udid, task)
        }
    }

    /// Asks whether each project is a mobile app, once; a failure reads as not mobile.
    public func detectMobile(_ paths: [String]) {
        guard let client else { return }
        for p in Set(paths) where !p.isEmpty && mobile[p] == nil {
            mobile[p] = false
            Task {
                let v = try? await client.call("project:detectMobile", ["projectPath": .string(p)])
                mobile[p] = v?["isMobile"]?.boolValue ?? false
            }
        }
    }

    public func showsDeviceButton(_ s: TerminalSession) -> Bool {
        Devices.shouldShowButton(mobile: mobile[s.projectPath], hasPane: devices[s.id] != nil, shell: s.agentType == .shell)
    }

    /// Drops what belongs to sessions that are gone.
    public func prune(liveSessions: Set<String>) {
        var changed = false
        for s in panes.open.keys where !liveSessions.contains(s) { changed = panes.close(s) || changed }
        for s in claims.keys where !liveSessions.contains(s) { _ = release(s) }
        for s in live.keys where !liveSessions.contains(s) && panes[s] == nil { live[s] = nil }
        if changed { save() }
        syncViews()
    }

    // MARK: The agents' bridge

    /// Answers a `browser:*` or `device:*` call an agent's tool made.
    public func bridge(_ method: String, _ params: JSONValue) async -> Result<JSONValue, VornRequestError> {
        let s = params["sessionId"]?.stringValue ?? ""
        if session(s) == nil && method != "device:chrome" { return .failure(VornRequestError("No session \(s) in this app.")) }
        let limit = { (d: Int) in params["limit"]?.numberValue.map { Int($0) } ?? d }
        let gen = host.generation(s)
        func noPane() -> Result<JSONValue, VornRequestError> { .failure(VornRequestError(BrowserBridge.noPane)) }
        switch method {
        case "browser:openPane":
            guard let raw = params["url"]?.stringValue else {
                if panes.open(s, url: nil) { save() }
                syncViews()
                return .success(["url": ""])
            }
            guard let url = BrowserURL.loadable(raw, root: root(s)) else { return .failure(VornRequestError(BrowserBridge.refusing(raw, verb: "open"))) }
            if let a = params["artifact"], let data = try? JSONEncoder().encode(a), let art = try? JSONDecoder().decode(TabArtifact.self, from: data) {
                panes.openArtifact(s, url: url, artifact: art)
            } else {
                _ = panes.open(s, url: url)
            }
            save()
            syncViews()
            return .success(["url": .string(url)])
        case "browser:navigate":
            guard panes[s] != nil else { return noPane() }
            let raw = params["url"]?.stringValue ?? ""
            guard let url = BrowserURL.loadable(raw, root: root(s)) else { return .failure(VornRequestError(BrowserBridge.refusing(raw, verb: "navigate to"))) }
            _ = panes.open(s, url: url)
            syncViews()
            host.load(s, url, root: root(s))
            save()
            return .success(["url": .string(url)])
        case "browser:tabs":
            guard panes[s] != nil else { return noPane() }
            let action = params["action"]?.stringValue ?? ""
            let index = params["index"]?.numberValue.map { Int($0) }
            switch (action, index) {
            case ("add", _):
                var url: String?
                if let raw = params["url"]?.stringValue {
                    guard let u = BrowserURL.loadable(raw, root: root(s)) else { return .failure(VornRequestError(BrowserBridge.refusing(raw, verb: "open"))) }
                    url = u
                }
                _ = panes.addTab(s, url: url)
            case ("close", let i?): _ = panes.closeTab(s, index: i)
            case ("select", let i?): _ = panes.selectTab(s, index: i)
            default: return .failure(VornRequestError("A tab index is required to \(action) a tab."))
            }
            tabChanged(s)
            save()
            syncViews()
            return .success(["ok": true])
        case "browser:listTabs":
            guard panes[s] != nil else { return noPane() }
            return .success(["tabs": .array(panes.tabInfos(s))])
        case "browser:history":
            guard panes[s] != nil else { return noPane() }
            return host.step(s, back: params["direction"]?.stringValue != "forward").map { .object(["url": .string($0)]) }
        case "browser:readPage":
            guard let gen else { return noPane() }
            let start = BrowserURL.parseCursor(params["cursor"]?.stringValue, generation: gen)
            let all = params["filter"]?.stringValue == "all"
            let answer = await host.eval(s, "JSON.stringify(window.__vorn.read(\(gen), \(all), \(start), \(limit(200))))")
            return answer.flatMap { v in
                host.generation(s) != gen ? .failure(VornRequestError("The page navigated while it was being read. Call read_page again."))
                    : .success(BrowserBridge.readResult(v, generation: gen))
            }
        case "browser:getText":
            guard let gen else { return noPane() }
            let start = BrowserURL.parseCursor(params["cursor"]?.stringValue, generation: gen)
            return await host.eval(s, "JSON.stringify(window.__vorn.text())").map { BrowserBridge.textResult($0, generation: gen, start: start) }
        case "browser:find":
            guard let gen else { return noPane() }
            let text = BrowserBridge.js(.string(params["text"]?.stringValue ?? ""))
            let answer = await host.eval(s, "JSON.stringify(window.__vorn.find(\(gen), \(text), \(limit(20))))")
            return answer.flatMap { v in
                host.generation(s) != gen ? .failure(VornRequestError("The page navigated while it was being searched. Call find again.")) : .success(v)
            }
        case "browser:interact":
            return await host.eval(s, "JSON.stringify(window.__vorn.act(\(BrowserBridge.js(params))))").map { _ in ["ok": true] }
        case "browser:consoleMessages", "browser:networkRequests":
            guard panes[s] != nil else { return noPane() }
            let l = liveState(s)
            return .success(BrowserBridge.tail(method == "browser:consoleMessages" ? l.console : l.network, limit: limit(50)))
        case "browser:screenshot":
            return .failure(VornRequestError(BrowserBridge.noScreenshot))
        case "device:list":
            do { return .success(.array(try await deviceBackend.list(holders()).map(\.json))) } catch {
                return .failure(VornRequestError((error as? DeviceError)?.message ?? error.localizedDescription))
            }
        case "device:claim":
            return await claim(s, params["udid"]?.stringValue ?? "", open: false, reply: true)
        case "device:release":
            return .success(["released": .bool(release(s))])
        case "device:openPane":
            if let udid = params["udid"]?.stringValue { return await claim(s, udid, open: true, reply: true) }
            guard let c = claims[s] else {
                return .failure(VornRequestError("No device is claimed for this session. Call device_claim first, or pass a udid."))
            }
            if devices[s] == nil { devices[s] = DevicePaneState() }
            syncPollers()
            return .success(["udid": .string(c.udid)])
        case "device:launch", "device:terminate", "device:openUrl", "device:install":
            guard let udid = claims[s]?.udid else { return .failure(VornRequestError(BrowserBridge.noClaim)) }
            let arg = { (k: String) in params[k]?.stringValue ?? "" }
            do {
                switch method {
                case "device:launch": try await Devices.launch(udid, bundle: arg("bundleId"))
                case "device:terminate": try await Devices.terminate(udid, bundle: arg("bundleId"))
                case "device:openUrl": try await Devices.openURL(udid, url: arg("url"))
                default: try await Devices.install(udid, path: arg("path"))
                }
                return .success(["ok": true])
            } catch {
                return .failure(VornRequestError((error as? DeviceError)?.message ?? error.localizedDescription))
            }
        case "device:screenshot":
            guard let udid = claims[s]?.udid else { return .failure(VornRequestError(BrowserBridge.noClaim)) }
            do {
                let png = try await deviceBackend.screenshot(udid)
                let size = Devices.pngSize(png) ?? (0, 0)
                return .success(["data": .string(png.base64EncodedString()), "scale": 1,
                                 "screen": ["width": .number(Double(size.width)), "height": .number(Double(size.height))], "orientation": "portrait"])
            } catch {
                return .failure(VornRequestError((error as? DeviceError)?.message ?? error.localizedDescription))
            }
        case "device:chrome":
            return .success(.null)
        case let m where m.hasPrefix("device:"):
            return .failure(VornRequestError(BrowserBridge.noCompanion))
        default:
            return .failure(VornRequestError("\(method) is not available in the native app yet."))
        }
    }
}

extension ISO8601DateFormatter {
    static let vornMillis: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()
}
