import AppKit
import Foundation
import Testing
@testable import VornApp
import VornCore
@testable import VornPanes

/// The browser pane, artifacts, a design and the device pane, driven the way an agent's tools drive them, against a throwaway vornd.
@MainActor
struct BrowserPanesLiveTests {
    private func connect(_ server: TestVornd) async throws -> (VornStore, AppModel, BrowserModel, VornClient) {
        let store = VornStore(dataDir: server.dataDir, readOnly: false)
        store.start()
        #expect(await liveUntil(timeout: 15) { store.phase == .connected })
        try #require(store.phase == .connected)
        let model = AppModel()
        model.persistsSidebar = false
        model.panes = PanesModel(persists: false)
        let actions = ShellActions(store: store, model: model)
        actions.bindBrowser()
        let agent = VornClient(dataDir: server.dataDir)
        try await agent.connect()
        return (store, model, model.panes.browser, agent)
    }

    /// An agent's call, retried while the app's bridge is still identifying itself.
    private func ask(_ agent: VornClient, _ method: String, _ params: JSONValue) async throws -> JSONValue {
        for _ in 0..<40 {
            do { return try await agent.call(method, params) } catch VornError.rpc(_, let m) where m.contains("not running") {
                try await Task.sleep(for: .milliseconds(100))
            }
        }
        return try await agent.call(method, params)
    }

    private func project(_ server: TestVornd, _ name: String, files: [String: String]) throws -> URL {
        let dir = server.root.appendingPathComponent("home/\(name)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        for (path, text) in files { try text.write(to: dir.appendingPathComponent(path), atomically: true, encoding: .utf8) }
        return dir.resolvingSymlinksInPath()
    }

    @Test func agentDrivesTheBrowser() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let (store, model, browser, agent) = try await connect(server)
        defer { store.stop() }
        let dir = try project(server, "site", files: [
            "index.html": "<html><head><title>Hello page</title></head><body><h1>Welcome</h1><button id=go>Go</button><a href=\"next.html\">Next</a></body></html>",
            "next.html": "<html><body><p>Second page</p></body></html>",
        ])
        try await store.addProject(Project(name: "site", path: dir.path))
        let shell = try await store.createShell(cwd: dir.path, project: store.project(named: "site"))
        let s = shell.id

        // The card's globe button opens the pane in the session's column.
        model.panes.browser.toggle(s)
        #expect(browser.isOpen(s))
        #expect(model.panes.columnEntries(s).contains { $0.kind == .browser })
        browser.toggle(s)
        #expect(!browser.isOpen(s))

        let file = dir.appendingPathComponent("index.html").absoluteString
        let opened = try await ask(agent, "browser:openPane", ["sessionId": .string(s), "url": .string(file)])
        #expect(opened["url"]?.stringValue == file)
        #expect(browser.isOpen(s))

        // Outside the session's tree stays refused.
        await #expect(throws: VornError.self) { try await agent.call("browser:navigate", ["sessionId": .string(s), "url": "file:///etc/hosts"]) }

        #expect(await liveUntil(timeout: 20) {
            let text = try? await agent.call("browser:getText", ["sessionId": .string(s)])
            return text?["text"]?.stringValue?.contains("Welcome") == true
        })
        let read = try await agent.call("browser:readPage", ["sessionId": .string(s)])
        #expect("\(read)".contains("Go"))
        let found = try await agent.call("browser:find", ["sessionId": .string(s), "text": "Next"])
        #expect("\(found)".contains("Next"))

        _ = try await agent.call("browser:tabs", ["sessionId": .string(s), "action": "add", "url": "localhost:9"])
        let tabs = try await agent.call("browser:listTabs", ["sessionId": .string(s)])
        #expect(tabs["tabs"]?.arrayValue?.count == 2)
        #expect(browser.panes[s]?.activeTab == 1)
        _ = try await agent.call("browser:tabs", ["sessionId": .string(s), "action": "close", "index": 1])
        #expect(browser.panes[s]?.tabs.count == 1)

        _ = try await agent.call("browser:navigate", ["sessionId": .string(s), "url": .string(dir.appendingPathComponent("next.html").absoluteString)])
        #expect(await liveUntil(timeout: 20) {
            let text = try? await agent.call("browser:getText", ["sessionId": .string(s)])
            return text?["text"]?.stringValue?.contains("Second page") == true
        })
        #expect(await liveUntil(timeout: 10) { browser.liveState(s).nav.back == true })
        let back = try await agent.call("browser:history", ["sessionId": .string(s), "direction": "back"])
        #expect(back["url"]?.stringValue?.hasSuffix("index.html") == true)

        // The address bar: a typed host goes to http, junk says why.
        browser.submitAddress(s, "localhost:9")
        #expect(browser.panes[s]?.active?.url == "http://localhost:9/")
        await #expect(throws: VornError.self) { try await agent.call("browser:screenshot", ["sessionId": .string(s)]) }

        // A closed pane answers that it is closed; a session that ended takes its pane with it.
        browser.close(s)
        await #expect(throws: VornError.self) { try await agent.call("browser:listTabs", ["sessionId": .string(s)]) }
        browser.open(s, url: nil)
        model.panes.prune(liveSessions: [])
        #expect(!browser.isOpen(s))
    }

    @Test func artifactsCommentsAndDocEdits() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let (store, _, browser, agent) = try await connect(server)
        defer { store.stop() }
        let dir = try project(server, "docs", files: ["README.md": "hi\n"])
        try await store.addProject(Project(name: "docs", path: dir.path))
        let shell = try await store.createShell(cwd: dir.path, project: store.project(named: "docs"))
        let s = shell.id
        _ = try? await ask(agent, "browser:listTabs", ["sessionId": .string(s)])

        let doc = "# Plan\n\nFirst paragraph.\n\nSecond paragraph.\n"
        let published = try await agent.call("artifact:publish", ["sessionId": .string(s), "title": "Plan", "kind": "doc", "content": .string(doc)])
        #expect(published["opened"]?.boolValue == true)
        let id = try #require(published["artifact"]?["id"]?.stringValue)
        #expect(browser.art(s)?.id == id)
        #expect(browser.art(s)?.kind == "doc")
        #expect(await liveUntil(timeout: 10) { browser.artState(s)?.versions.count == 1 })
        #expect(await liveUntil(timeout: 20) {
            let text = try? await agent.call("browser:getText", ["sessionId": .string(s)])
            return text?["text"]?.stringValue?.contains("First paragraph") == true
        })

        // A note from the rail, then an edit and delete.
        browser.addNote(s, "Tighten the intro")
        #expect(await liveUntil(timeout: 10) { browser.comments(s).drafts.map(\.body) == ["Tighten the intro"] })
        let note = try #require(browser.comments(s).drafts.first)
        browser.railEdit(s, note.id)
        browser.railSave(s, "Tighten the intro a lot")
        #expect(await liveUntil(timeout: 10) { browser.comments(s).drafts.first?.body == "Tighten the intro a lot" })
        browser.addNote(s, "Second note")
        #expect(await liveUntil(timeout: 10) { browser.comments(s).drafts.count == 2 })
        browser.railDelete(s, note.id)
        #expect(await liveUntil(timeout: 10) { browser.comments(s).drafts.map(\.body) == ["Second note"] })

        // A shell is never at an agent's prompt, so Send holds the batch until it is.
        browser.send(s)
        #expect(await liveUntil(timeout: 15) { browser.artState(s)?.queued == true })
        #expect(browser.comments(s).drafts.count == 1)

        // Edit the doc and save: a new version, with the changed paragraph as the edit.
        browser.edit(s)
        #expect(await liveUntil(timeout: 10) { browser.liveState(s).editing?.original == doc })
        browser.setEdited(s, doc.replacingOccurrences(of: "Second paragraph.", with: "A better second paragraph."))
        browser.saveDoc(s, send: false)
        #expect(await liveUntil(timeout: 15) { browser.art(s)?.version == 2 && browser.liveState(s).editing == nil })
        let source = try await agent.call("artifact:readSource", ["artifactId": .string(id), "version": 2])
        #expect(source["body"]?.stringValue?.contains("A better second paragraph.") == true)
        #expect(await liveUntil(timeout: 10) { browser.artState(s)?.versions.count == 2 })

        // Back to v1 through the version menu.
        browser.chooseVersion(s, 1)
        #expect(await liveUntil(timeout: 10) { browser.art(s)?.version == 1 })
        browser.openLatest(s)
        #expect(await liveUntil(timeout: 10) { browser.art(s)?.version == 2 })
    }

    @Test func designCanvasAndTweaks() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let (store, _, browser, agent) = try await connect(server)
        defer { store.stop() }
        let design = """
        <html><head><script type="application/json" id="artifact">
        {"kind":"design","title":"Hero","tweaks":{"dark":{"type":"boolean","default":false,"label":"Dark"},
         "size":{"type":"number","default":16,"min":10,"max":40}},
         "artboards":[{"id":"desktop","label":"Desktop","width":1280,"height":800},{"id":"phone","label":"Phone","width":390,"height":844}]}
        </script></head><body><h1>Hero</h1></body></html>
        """
        let dir = try project(server, "design", files: ["hero.dc.html": design])
        try await store.addProject(Project(name: "design", path: dir.path))
        let shell = try await store.createShell(cwd: dir.path, project: store.project(named: "design"))
        let s = shell.id
        let url = dir.appendingPathComponent("hero.dc.html").absoluteString
        _ = try await ask(agent, "browser:openPane", ["sessionId": .string(s), "url": .string(url)])

        #expect(await liveUntil(timeout: 20) { browser.boards(s)?.map(\.id) == ["desktop", "phone"] })
        #expect(browser.liveState(s).manifest?.kind == "design")
        #expect(browser.liveState(s).values["dark"] == false)
        browser.syncViews()
        #expect(browser.host.pages["\(s)#board:phone"] != nil)

        browser.applyTweak(s, key: "dark", value: true)
        #expect(browser.liveState(s).values["dark"] == true)
        #expect(browser.panes.tweaks[dir.appendingPathComponent("hero.dc.html").path]?["dark"] == true)

        let fit = browser.canvasZoom(s, area: CGSize(width: 1000, height: 700))
        browser.zoom(s, 1, area: CGSize(width: 1000, height: 700))
        #expect(browser.canvasZoom(s, area: CGSize(width: 1000, height: 700)) == DesignCanvas.clampZoom(fit + 0.1))
        browser.zoom(s, 0, area: nil)
        #expect(browser.liveState(s).zoom == nil)
    }

    @Test func devicePaneWithAFakeSimulator() async throws {
        guard let server = try TestVornd.launch() else { return }
        defer { server.stop() }
        let (store, _, browser, agent) = try await connect(server)
        defer { store.stop() }
        let png = Self.png(width: 1206, height: 2622)
        browser.deviceBackend = DeviceBackend(
            list: { claims in [Device(udid: "U1", name: "iPhone 17 Pro", runtime: "iOS 26.2", booted: true, claimedBy: claims["U1"]),
                               Device(udid: "U2", name: "iPad Air", runtime: "iOS 26.2", booted: false, claimedBy: claims["U2"])] },
            claim: { udid, _, _ in Devices.Claimed(udid: udid, name: udid == "U1" ? "iPhone 17 Pro" : "iPad Air", bootedHere: false) },
            shutdown: { _ in },
            screenshot: { _ in png })
        let dir = try project(server, "app", files: ["app.json": "{\"expo\":{}}", "package.json": "{\"dependencies\":{\"expo\":\"1\",\"react-native\":\"1\"}}"])
        try await store.addProject(Project(name: "app", path: dir.path))
        let a = try await store.createShell(cwd: dir.path, project: store.project(named: "app"))
        let b = try await store.createShell(cwd: dir.path, project: store.project(named: "app"))

        let listed = try await ask(agent, "device:list", ["sessionId": .string(a.id)])
        #expect(listed.arrayValue?.count == 2)
        let opened = try await agent.call("device:openPane", ["sessionId": .string(a.id), "udid": "U1"])
        #expect(opened["udid"]?.stringValue == "U1")
        #expect(browser.hasDevicePane(a.id))
        #expect(await liveUntil(timeout: 10) { browser.frames[a.id]?.pixels == CGSize(width: 1206, height: 2622) })

        // Another session cannot take a held device.
        let refused = try await agent.call("device:claim", ["sessionId": .string(b.id), "udid": "U1"])
        #expect(refused["reason"]?.stringValue == "held-by-session")
        browser.openPicker(b.id, fromPane: false)
        #expect(await liveUntil(timeout: 5) { browser.picker?.loading == false })
        #expect(browser.picker?.devices.map(\.device.udid) == ["U2", "U1"])
        #expect(browser.picker?.devices.last?.selectable == false)
        browser.picker = nil

        // Controls: zoom steps, fit, and the companion-only ones say so.
        let area = CGSize(width: 400, height: 700)
        browser.deviceControl(a.id, "zoom-in", area: area)
        #expect(browser.devices[a.id]?.zoom != nil)
        browser.deviceControl(a.id, "fit", area: area)
        #expect(browser.devices[a.id]?.zoom == nil)
        browser.deviceControl(a.id, "home", area: area)
        #expect(browser.devices[a.id]?.error == BrowserBridge.noCompanion)
        browser.dismissDeviceError(a.id)

        let shot = try await agent.call("device:screenshot", ["sessionId": .string(a.id)])
        #expect(shot["screen"]?["width"]?.numberValue == 1206)

        // The card's phone button hides the pane; release lets the device go.
        browser.deviceButton(a.id)
        #expect(!browser.hasDevicePane(a.id))
        let released = try await agent.call("device:release", ["sessionId": .string(a.id)])
        #expect(released["released"]?.boolValue == true)
        #expect(browser.frames[a.id] == nil)
    }

    static func png(width: Int, height: Int) -> Data {
        let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height, bitsPerSample: 8, samplesPerPixel: 4,
                                   hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        return rep.representation(using: .png, properties: [:])!
    }
}
