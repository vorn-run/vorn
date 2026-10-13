import CoreGraphics
import Foundation
import Testing
@testable import VornCore

/// The browser, artifact, design and device rules, against the cases the Electron app's tests pin.
struct BrowserURLTests {
    @Test func bareHostAndPortIsADevServer() {
        #expect(BrowserURL.normalize("localhost:5173") == "http://localhost:5173/")
        #expect(BrowserURL.normalize("127.0.0.1:8080/api") == "http://127.0.0.1:8080/api")
        #expect(BrowserURL.normalize("myapp.local:3000") == "http://myapp.local:3000/")
        #expect(BrowserURL.normalize("[::1]:5173") == "http://[::1]:5173/")
        #expect(BrowserURL.normalize("[::1]") == "http://[::1]/")
        #expect(BrowserURL.normalize("[2001:db8::1]:8080") == "https://[2001:db8::1]:8080/")
    }

    @Test func publicHostsGoToHttps() {
        #expect(BrowserURL.normalize("example.com") == "https://example.com/")
        #expect(BrowserURL.normalize("example.com/docs?a=1") == "https://example.com/docs?a=1")
        #expect(BrowserURL.normalize("//cdn.example.com/x") == "https://cdn.example.com/x")
        #expect(BrowserURL.normalize("  example.com  ") == "https://example.com/")
        #expect(BrowserURL.normalize("https://x.com/a?b=1") == "https://x.com/a?b=1")
        #expect(BrowserURL.normalize("http://localhost:3000") == "http://localhost:3000/")
        #expect(BrowserURL.normalize("about:blank") == "about:blank")
    }

    @Test func refusesWhatAPaneShouldNotLoad() {
        for bad in ["file:///etc/passwd", "javascript:alert(1)", "data:text/html,<h1>x", "ftp://h.com", "", "   ", "not a url"] {
            #expect(BrowserURL.normalize(bad) == nil, "\(bad)")
        }
        #expect(BrowserURL.normalize("file:///proj/index.html", allowFile: true) == "file:///proj/index.html")
        #expect(BrowserURL.normalize("javascript:alert(1)", allowFile: true) == nil)
        #expect(BrowserURL.normalize("file://evil.example/share/secret", allowFile: true) == nil)
        #expect(BrowserURL.normalize("file:///proj/a.html?x=1#frag", allowFile: true) == "file:///proj/a.html")
        #expect(BrowserURL.normalize("file:1234", allowFile: true) == nil)
    }

    @Test func displayHost() {
        #expect(BrowserURL.displayHost("http://localhost:5173/some/deep/path") == "localhost:5173")
        #expect(BrowserURL.displayHost("https://example.com/a/b") == "example.com")
        #expect(BrowserURL.displayHost("file:///Users/j/dev/vorn/design/true-black.dc.html") == "true-black.dc.html")
        #expect(BrowserURL.displayHost("file:///repo/a%20b.dc.html") == "a b.dc.html")
        #expect(BrowserURL.displayHost("file:///repo/%ZZ.dc.html") == "%ZZ.dc.html")
        #expect(BrowserURL.displayHost("about:blank") == "New tab")
        #expect(BrowserURL.displayHost("nonsense") == "nonsense")
    }

    @Test func fileLoadsStayInsideTheSessionRoot() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("vorn-root-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let page = root.appendingPathComponent("index.html")
        try "<p>hi</p>".write(to: page, atomically: true, encoding: .utf8)
        let real = page.resolvingSymlinksInPath().path
        #expect(BrowserURL.loadable("file://\(real)", root: root.path) != nil)
        #expect(BrowserURL.loadable("file:///etc/hosts", root: root.path) == nil)
        #expect(BrowserURL.loadable("file://\(real)", root: nil) == nil)
        #expect(BrowserURL.loadable("localhost:3000", root: nil) == "http://localhost:3000/")
    }

    @Test func cursors() {
        #expect(BrowserURL.parseCursor("3:40", generation: 3) == 40)
        #expect(BrowserURL.parseCursor("2:40", generation: 3) == 0)
        #expect(BrowserURL.parseCursor(nil, generation: 3) == 0)
        #expect(BrowserURL.encodeURIComponent("a b/é") == "a%20b%2F%C3%A9")
    }
}

struct BrowserPanesTests {
    @Test func tabsOpenSelectAndClose() {
        var p = BrowserPanes()
        let ok556 = p.open("s", url: "http://localhost:3000/")
        #expect(ok556)
        #expect(p["s"]?.tabs.map(\.url) == ["http://localhost:3000/"])
        let ok917 = p.addTab("s", url: "https://example.com/")
        #expect(ok917)
        #expect(p["s"]?.activeTab == 1)
        let ok920 = p.selectTab("s", index: 0)
        #expect(ok920)
        #expect(p["s"]?.activeTab == 0)
        let ok236 = p.closeTab("s", index: 0)
        #expect(ok236)
        #expect(p["s"]?.tabs.map(\.url) == ["https://example.com/"])
        #expect(p.tabInfos("s").count == 1)
        let ok793 = p.close("s")
        #expect(ok793)
        #expect(p["s"] == nil)
    }

    @Test func survivesAnEncodeRoundTrip() throws {
        var p = BrowserPanes()
        _ = p.open("s", url: "http://localhost:3000/")
        p.openArtifact("s", url: "http://127.0.0.1:9/artifact/a1/1", artifact: TabArtifact(id: "a1", kind: "doc", version: 1, title: "Plan"))
        let back = try JSONDecoder().decode(BrowserPanes.self, from: JSONEncoder().encode(p.intent))
        #expect(back["s"]?.tabs.count == p["s"]?.tabs.count)
        #expect(back["s"]?.tabs.last?.artifact?.id == "a1")
    }
}

struct ArtifactTests {
    static let doc = [
        "# Fine-tuning a small model",
        "That works, but it always bothered me a little: cost, privacy and accuracy.",
        "So in this experiment I wanted to see how far a small model can go.",
        "* one\n* two",
    ].joined(separator: "\n\n")

    @Test func markdownSpellingIsNoEdit() {
        let r = Artifacts.mergeDocEdit(original: Self.doc, edited: Self.doc.replacingOccurrences(of: "* one\n* two", with: "- one\n- two"))
        #expect(r.edits.isEmpty)
        #expect(r.body.contains("* one"))
    }

    @Test func changedParagraphIsBeforeAndAfter() {
        let edited = Self.doc
            .replacingOccurrences(of: "it always bothered me a little", with: "a few things kept me wondering whether it really scales")
            .replacingOccurrences(of: "* one\n* two", with: "- one\n- two")
        let r = Artifacts.mergeDocEdit(original: Self.doc, edited: edited)
        #expect(r.edits == [Artifacts.DocEdit(
            before: "That works, but it always bothered me a little: cost, privacy and accuracy.",
            after: "That works, but a few things kept me wondering whether it really scales: cost, privacy and accuracy.")])
        #expect(Artifacts.paragraphs(r.body).count == 4)
        #expect(r.body.contains("* one"))
    }

    @Test func addedAndRemovedParagraphs() {
        #expect(Artifacts.mergeDocEdit(original: Self.doc, edited: Self.doc + "\n\nA new closing line.").edits
            == [Artifacts.DocEdit(before: "", after: "A new closing line.")])
        let removed = Artifacts.mergeDocEdit(original: Self.doc, edited: Self.doc.replacingOccurrences(
            of: "\n\nSo in this experiment I wanted to see how far a small model can go.", with: ""))
        #expect(removed.edits == [Artifacts.DocEdit(before: "So in this experiment I wanted to see how far a small model can go.", after: "")])
        #expect(Artifacts.paragraphs(removed.body).count == 3)
    }

    @Test func latestBatchAndMarks() {
        let a = ArtifactComment(id: "a", version: 1, anchor: .quote(quote: "x", prefix: "", suffix: ""), body: "1", state: "sent", batchId: "b1", sentAt: "2024-01-01")
        let b = ArtifactComment(id: "b", version: 1, anchor: nil, body: "2", state: "sent", batchId: "b2", sentAt: "2024-02-01")
        let c = ArtifactComment(id: "c", version: 1, anchor: .quote(quote: "y", prefix: "p", suffix: "s"), body: "3", state: "draft")
        #expect(Artifacts.latestSentBatch([a, b, c]).map(\.id) == ["b"])
        let marks = Artifacts.marksFor(drafts: [c], sent: [a], focus: "c")
        #expect(marks.map(\.id) == ["a", "c"])
        #expect(marks.map(\.state) == ["sent", "focus"])
    }

    @Test func popoverStaysInTheArea() {
        let below = Artifacts.placePopover(CGRect(x: 380, y: 40, width: 24, height: 24), area: CGSize(width: 420, height: 320))
        #expect(below == CGPoint(x: 420 - 260 - 8, y: 72))
        let above = Artifacts.placePopover(CGRect(x: 10, y: 250, width: 24, height: 24), area: CGSize(width: 420, height: 320))
        #expect(above == CGPoint(x: 10, y: 92))
    }
}

struct DesignCanvasTests {
    static let boards = [Artboard(id: "desktop", label: "Desktop", width: 1440, height: 900),
                         Artboard(id: "phone", label: "Phone", width: 390, height: 844)]

    @Test func laysBoardsSideBySide() {
        let (placed, width, height) = DesignCanvas.place(Self.boards, zoom: 0.5)
        #expect(placed == [CGPoint(x: 24, y: 44), CGPoint(x: 24 * 2 + 720, y: 44)])
        #expect(width == 24 * 3 + 720 + 195)
        #expect(height == 24 * 2 + 20 + 450)
    }

    @Test func fitsEveryBoard() {
        #expect(DesignCanvas.fitZoom(Self.boards, area: CGSize(width: 1000, height: 800)) == 0.5)
        #expect(DesignCanvas.fitZoom(Self.boards, area: CGSize(width: 300, height: 800)) == DesignCanvas.minZoom)
        #expect(DesignCanvas.fitZoom(Self.boards, area: CGSize(width: 9000, height: 9000)) == 1)
    }

    @Test func boardURLs() {
        #expect(DesignCanvas.artboardURL("http://127.0.0.1:9/artifact/a1/2?t=tok", id: "phone") == "http://127.0.0.1:9/artifact/a1/2?t=tok#artboard=phone")
        #expect(DesignCanvas.artboardURL("file:///repo/hero.dc.html#board-2", id: "dark") == "file:///repo/hero.dc.html#artboard=dark")
    }

    @Test func manifestValidatesTweaksAndBoards() throws {
        let m = try #require(Manifest.parse("""
        {"kind":"design","title":"  Hero  ","tweaks":{"plan":{"type":"number","default":10,"min":0},
         "bad name":{"type":"number","default":1},"tone":{"type":"select","default":"x","options":["a","b"]},
         "dark":{"type":"boolean","default":true}},
         "artboards":[{"id":"desktop","width":1440,"height":900},{"id":"tiny","width":10,"height":10},{"id":"desktop","width":400,"height":400}]}
        """))
        #expect(m.kind == "design")
        #expect(m.title == "Hero")
        #expect(m.tweaks.map(\.0) == ["plan", "tone", "dark"])
        #expect(m.tweaks[1].1 == .select(label: nil, default: "a", options: ["a", "b"]))
        #expect(m.artboards == [Artboard(id: "desktop", label: "desktop", width: 1440, height: 900)])
        #expect(Manifest.parse("{\"kind\":\"other\"}") == nil)
        #expect(Manifest.parse("not json") == nil)
        let merged = Manifest.mergeTweaks(m.tweaks, stored: ["plan": 9000, "dark": "no"], live: ["tone": "b"])
        #expect(merged == ["plan": 9000, "tone": "b", "dark": true])
        #expect(Manifest.hexColor("#abc") == 0xAABBCC)
        #expect(Manifest.hexColor("red") == nil)
    }
}

struct DeviceTests {
    static func device(_ udid: String = "u1", name: String = "iPhone 17", runtime: String = "iOS 26.2", booted: Bool = false, claimedBy: String? = nil) -> Device {
        Device(udid: udid, name: name, runtime: runtime, booted: booted, claimedBy: claimedBy)
    }

    @Test func buttonGate() {
        #expect(Devices.shouldShowButton(mobile: true, hasPane: false, shell: false))
        #expect(!Devices.shouldShowButton(mobile: false, hasPane: false, shell: false))
        #expect(!Devices.shouldShowButton(mobile: nil, hasPane: false, shell: false))
        #expect(Devices.shouldShowButton(mobile: false, hasPane: true, shell: false))
        #expect(!Devices.shouldShowButton(mobile: true, hasPane: false, shell: true))
    }

    @Test func pickerOrderAndSelectability() {
        #expect(Devices.isSelectable(Self.device(), session: "s1"))
        #expect(Devices.isSelectable(Self.device(claimedBy: "s1"), session: "s1"))
        #expect(!Devices.isSelectable(Self.device(claimedBy: "s2"), session: "s1"))
        let listed = Devices.order([
            Self.device("a", name: "iPhone 17", claimedBy: "s2"),
            Self.device("b", name: "iPad Air"),
            Self.device("c", name: "iPhone 17", runtime: "iOS 18.0", booted: true),
        ], session: "s1")
        #expect(listed.map(\.device.udid) == ["c", "b", "a"])
        #expect(listed[0].subtitle == "iOS 18.0")
        #expect(listed[1].subtitle == nil)
        #expect(!listed[2].selectable)
    }

    @Test func runtimesAndSimctl() {
        #expect(Devices.formatRuntime("com.apple.CoreSimulator.SimRuntime.iOS-26-2") == "iOS 26.2")
        let json: JSONValue = ["devices": [
            "com.apple.CoreSimulator.SimRuntime.iOS-26-2": [
                ["udid": "u1", "name": "iPhone 17", "state": "Booted", "isAvailable": true],
                ["udid": "u2", "name": "iPhone 16", "state": "Shutdown", "isAvailable": false],
            ],
            "com.apple.CoreSimulator.SimRuntime.watchOS-11-0": [["udid": "w", "name": "Watch", "state": "Shutdown", "isAvailable": true]],
        ]]
        #expect(Devices.parseSimctl(json, claims: ["u1": "s"]) == [Self.device("u1", booted: true, claimedBy: "s")])
    }

    @Test func zoomAndFrame() {
        #expect(Devices.steppedZoom(1, inward: true) > 1)
        #expect(Devices.steppedZoom(1, inward: false) < 1)
        #expect(Devices.steppedZoom(Devices.zoomMax, inward: true) == Devices.zoomMax)
        #expect(Devices.steppedZoom(Devices.zoomMin, inward: false) == Devices.zoomMin)
        #expect(Devices.fitScale(points: CGSize(width: 402, height: 874), area: .zero) == 1)
        let area = CGSize(width: 500, height: 900), points = CGSize(width: 402, height: 874)
        let scale = Devices.fitScale(points: points, area: area)
        let frame = Devices.bezelThickness(area: area, points: points)
        #expect(points.width * scale + 2 * frame <= area.width)
        #expect(points.height * scale + 2 * frame <= area.height)
        #expect(Devices.bezelThickness(area: CGSize(width: 40, height: 40), points: points) >= 6)
        #expect(Devices.pointScale(width: 1206, height: 2622) == 3)
        #expect(Devices.pointScale(width: 2048, height: 2732) == 2)
    }

    @Test func screenshotNamesAndPNGs() {
        #expect(Devices.screenshotName("iPhone 17 Pro", iso: "2026-01-02T03:04:05.678Z") == "iPhone-17-Pro 2026-01-02 03-04-05.png")
        #expect(Devices.screenshotName("  ", iso: "2026-01-02T03:04:05.678Z") == "device 2026-01-02 03-04-05.png")
        var png = Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52])
        png.append(contentsOf: [0, 0, 1, 0x92, 0, 0, 3, 0x6A])
        #expect(Devices.pngSize(png)! == (402, 874))
        #expect(Devices.pngSize(Data([1, 2, 3])) == nil)
    }
}

struct BrowserBridgeTests {
    @Test func messagesAndPaging() {
        #expect(BrowserBridge.refusing("file:///etc/passwd", verb: "open").contains("file:///etc/passwd"))
        let tail = BrowserBridge.tail((0..<5).map { .number(Double($0)) }, limit: 2)
        #expect(tail.arrayValue?.count == 2 || tail["entries"]?.arrayValue?.count == 2)
        #expect(BrowserBridge.js(.string("a\"b")) == "\"a\\\"b\"")
    }
}
