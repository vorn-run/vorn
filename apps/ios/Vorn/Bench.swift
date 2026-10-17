import Foundation

/// With `-vornBench 1`, opens a session, scrolls the terminal and types, timing each part.
final class Bench {
    private let core: Core
    private let label: String
    private let open: (String) -> Void
    private var results: [String: Any] = [:]

    static var wanted: Bool { UserDefaults.standard.bool(forKey: "vornBench") }

    init(core: Core, label: String, open: @escaping (String) -> Void) {
        self.core = core
        self.label = label
        self.open = open
    }

    func start() { Task { @MainActor in await run() } }

    @MainActor private func run() async {
        var session: String?
        for _ in 0..<200 where session == nil {
            try? await Task.sleep(for: .milliseconds(100))
            session = core.tree("sessions")?.walk().first { $0.k == "link" && $0.value?.hasPrefix("session:") == true }?.value?
                .split(separator: ":", maxSplits: 1).last.map(String.init)
        }
        guard let session else {
            Report.write(core, label: label, extra: ["bench": "no session to open"])
            return
        }
        screens(["sessions", "tasks", "workflows", "launch", "session:\(session)"])
        await idle()
        try? await Task.sleep(for: .seconds(1))
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

    /// Building and decoding each screen's tree, best of five: what opening it costs before SwiftUI draws.
    private func screens(_ routes: [String]) {
        var out: [String: Double] = [:]
        for r in routes {
            var best = Double.infinity
            for _ in 0..<5 {
                let t = ProcessInfo.processInfo.systemUptime
                _ = core.build(r)
                best = min(best, (ProcessInfo.processInfo.systemUptime - t) * 1000)
            }
            out[r.split(separator: ":").first.map(String.init) ?? r] = best
        }
        results["screenMs"] = out
    }

    /// The share of one core the app uses over five seconds of nothing happening.
    @MainActor private func idle() async {
        func cpu() -> Double {
            var u = rusage()
            getrusage(RUSAGE_SELF, &u)
            return Double(u.ru_utime.tv_sec + u.ru_stime.tv_sec) + Double(u.ru_utime.tv_usec + u.ru_stime.tv_usec) / 1e6
        }
        let c0 = cpu(), t0 = ProcessInfo.processInfo.systemUptime
        try? await Task.sleep(for: .seconds(5))
        results["idleCpuPct"] = (cpu() - c0) / (ProcessInfo.processInfo.systemUptime - t0) * 100
    }

    private func snap(_ name: String) {
        var m = core.metrics()
        m.removeValue(forKey: "firstFrameMs")
        results[name] = m
    }
}
