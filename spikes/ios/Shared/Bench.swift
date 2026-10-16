import Foundation

/// With `-vornBench 1`, scrolls the list and terminal and types, timing each part the same in both apps.
final class Bench {
    private let core: Core
    private let label: String
    private let open: (String) -> Void
    /// Whether the list is drawn and so can be scrolled with synthetic touches.
    private let drawnList: Bool
    private var results: [String: Any] = [:]

    static var wanted: Bool { UserDefaults.standard.bool(forKey: "vornBench") }

    init(core: Core, label: String, drawnList: Bool, open: @escaping (String) -> Void) {
        self.core = core
        self.label = label
        self.drawnList = drawnList
        self.open = open
    }

    func start() { Task { @MainActor in await run() } }

    @MainActor private func run() async {
        var session: String?
        for _ in 0..<200 where session == nil {
            try? await Task.sleep(for: .milliseconds(100))
            session = core.tree("sessions")?.walk().first { $0.k == "link" }?.value?
                .split(separator: ":", maxSplits: 1).last.map(String.init)
        }
        guard let session else {
            Report.write(core, label: label, extra: ["bench": "no session to open"])
            return
        }
        try? await Task.sleep(for: .seconds(1))
        if drawnList {
            await drag(from: 620, to: 220)
            snap("list")
        }
        open(session)
        try? await Task.sleep(for: .seconds(2))
        await drag(from: 200, to: 600)
        snap("terminal")
        vp_metrics_reset(core.handle)
        for ch in "echo vorn phone" {
            vp_text(core.handle, String(ch))
            try? await Task.sleep(for: .milliseconds(180))
        }
        vp_key(core.handle, "Enter", 0)
        try? await Task.sleep(for: .seconds(1))
        snap("keys")
        Report.write(core, label: label, extra: ["bench": results])
    }

    /// A finger flick from `y0` to `y1` at the screen's middle, then the fling settling.
    @MainActor private func drag(from y0: Float, to y1: Float) async {
        vp_metrics_reset(core.handle)
        let steps = 24
        vp_touch(core.handle, 0, 200, y0)
        for i in 1...steps {
            try? await Task.sleep(for: .milliseconds(8))
            vp_touch(core.handle, 1, 200, y0 + (y1 - y0) * Float(i) / Float(steps))
        }
        vp_touch(core.handle, 2, 200, y1)
        try? await Task.sleep(for: .milliseconds(1500))
    }

    private func snap(_ name: String) {
        var m = core.metrics()
        m.removeValue(forKey: "firstFrameMs")
        results[name] = m
    }
}
