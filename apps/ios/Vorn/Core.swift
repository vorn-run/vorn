import Foundation
import SwiftUI

/// The Rust app, its connection to vornd, and a counter bumped whenever vornd sends something.
final class Core: ObservableObject {
    let handle: OpaquePointer
    @Published var revision = 0
    /// Ctrl latched on the key bar: the next key or letter typed goes with it.
    @Published var ctrl = false
    let launched = ProcessInfo.processInfo.systemUptime
    /// The stack that acted last: where an answer that arrives later goes.
    weak var front: Nav?

    private let box = WakeBox()
    /// The core's screen revision `revision` last caught up with.
    private var seen: UInt64 = 0
    private var trees: [String: (revision: Int, node: Node?)] = [:]
    private var tick: Timer?
    private static let prefsKey = "vorn.prefs"
    /// The view options as last saved, to write only when they change.
    private var prefs: String?

    init(host: String, token: String) {
        let wake: VpWake = { ctx in
            guard let ctx else { return }
            Unmanaged<WakeBox>.fromOpaque(ctx).takeUnretainedValue().ring()
        }
        handle = vp_new(host, token, wake, Unmanaged.passRetained(box).toOpaque())
        box.core = self
        if let saved = UserDefaults.standard.string(forKey: Self.prefsKey) {
            vp_set_prefs(handle, saved)
        }
        prefs = currentPrefs()
        // Relative times ("2m ago") move on without vornd saying anything.
        tick = Timer.scheduledTimer(withTimeInterval: 30, repeats: true) { [weak self] _ in self?.revision += 1 }
    }

    deinit {
        tick?.invalidate()
        vp_free(handle)
    }

    func woke() {
        _ = vp_pump(handle)
        let r = vp_revision(handle)
        if r != seen {
            seen = r
            revision += 1
        }
        if let go = takeGo() { front?.apply(go) }
    }

    /// Where the core says to go next, once.
    func takeGo() -> String? {
        guard let p = vp_take_go(handle) else { return nil }
        defer { vp_free_string(p) }
        return String(cString: p)
    }

    /// `screen`'s tree, built once per revision however many views read it.
    func tree(_ screen: String) -> Node? {
        if let t = trees[screen], t.revision == revision { return t.node }
        let node = build(screen)
        trees[screen] = (revision, node)
        if trees.count > 64 { trees = trees.filter { $0.value.revision == revision } }
        return node
    }

    /// `screen`'s tree fresh from the core, decoded.
    func build(_ screen: String) -> Node? {
        guard let p = vp_native_tree(handle, screen) else { return nil }
        defer { vp_free_string(p) }
        return try? JSONDecoder().decode(Node.self, from: Data(String(cString: p).utf8))
    }

    func act(_ screen: String, _ action: [String: Any]) {
        guard let data = try? JSONSerialization.data(withJSONObject: action),
              let json = String(data: data, encoding: .utf8) else { return }
        vp_native_action(handle, screen, json)
        revision += 1
        if let now = currentPrefs(), now != prefs {
            prefs = now
            UserDefaults.standard.set(now, forKey: Self.prefsKey)
        }
    }

    private func currentPrefs() -> String? {
        guard let p = vp_prefs(handle) else { return nil }
        defer { vp_free_string(p) }
        return String(cString: p)
    }

    func metrics() -> [String: Any] {
        guard let p = vp_metrics(handle) else { return [:] }
        defer { vp_free_string(p) }
        var m = (try? JSONSerialization.jsonObject(with: Data(String(cString: p).utf8))) as? [String: Any] ?? [:]
        m["memoryMB"] = Memory.footprintMB()
        return m
    }
}

/// What the wake callback points at: lives as long as the core and finds it once it exists.
final class WakeBox {
    weak var core: Core?
    private let lock = NSLock()
    private var queued = false

    /// Wakes the core on the main thread, once however many times vornd rings before it runs.
    func ring() {
        lock.lock()
        let first = !queued
        queued = true
        lock.unlock()
        guard first else { return }
        DispatchQueue.main.async {
            self.lock.lock()
            self.queued = false
            self.lock.unlock()
            self.core?.woke()
        }
    }
}

/// One node of a screen's semantic tree.
struct Node: Decodable, Identifiable {
    let k: String
    let id_: String?
    let label: String?
    let value: String?
    let style: String?
    let on: Bool?
    let secret: Bool?
    let confirm: String?
    let c: [Node]?

    var id: String { id_ ?? "\(k)-\(label ?? "")-\(value ?? "")-\(c?.count ?? 0)" }
    var kids: [Node] { c ?? [] }

    enum CodingKeys: String, CodingKey {
        case k, label, value, style, on, secret, confirm, c
        case id_ = "id"
    }

    /// Every node under this one, depth first, itself first.
    func walk() -> [Node] { [self] + kids.flatMap { $0.walk() } }
}

enum Memory {
    static func footprintMB() -> Double {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
        let kr = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        return kr == KERN_SUCCESS ? Double(info.phys_footprint) / 1_048_576 : 0
    }
}
