import Foundation
import SwiftUI

/// The Rust app, its connection to vornd, and a counter bumped whenever vornd sends something.
final class Core: ObservableObject {
    let handle: OpaquePointer
    @Published var revision = 0
    let launched = ProcessInfo.processInfo.systemUptime

    private let box = WakeBox()

    init(host: String, token: String) {
        let wake: VpWake = { ctx in
            guard let ctx else { return }
            let box = Unmanaged<WakeBox>.fromOpaque(ctx).takeUnretainedValue()
            DispatchQueue.main.async { box.core?.woke() }
        }
        handle = vp_new(host, token, wake, Unmanaged.passRetained(box).toOpaque())
        box.core = self
    }

    deinit { vp_free(handle) }

    func woke() {
        _ = vp_pump(handle)
        revision += 1
    }

    func tree(_ screen: String) -> Node? {
        guard let p = vp_native_tree(handle, screen) else { return nil }
        defer { vp_free_string(p) }
        return try? JSONDecoder().decode(Node.self, from: Data(String(cString: p).utf8))
    }

    func act(_ screen: String, _ action: [String: Any]) {
        guard let data = try? JSONSerialization.data(withJSONObject: action),
              let json = String(data: data, encoding: .utf8) else { return }
        vp_native_action(handle, screen, json)
        revision += 1
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
}

extension Core {
    static func fromSettings() -> Core {
        let d = UserDefaults.standard
        return Core(host: d.string(forKey: "vornHost") ?? "", token: d.string(forKey: "vornToken") ?? "")
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
