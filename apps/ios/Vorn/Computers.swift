import Foundation
import Security

/// A computer this phone is paired with; its token lives in the Keychain.
struct Computer: Codable, Identifiable, Equatable {
    let id: String
    var name: String
    /// `host:port`, or a `wss://host:port` base when it was paired over https.
    var host: String
}

/// The paired computers and which one is in use.
final class Computers: ObservableObject {
    @Published private(set) var list: [Computer] = []
    @Published private(set) var active: String?

    private let defaults = UserDefaults.standard
    private static let listKey = "vorn.devices"
    private static let activeKey = "vorn.activeDevice"

    init() {
        if let data = defaults.data(forKey: Self.listKey),
           let saved = try? JSONDecoder().decode([Computer].self, from: data) {
            list = saved
        }
        active = defaults.string(forKey: Self.activeKey)
        migrate()
        if current == nil { active = list.first?.id }
    }

    var current: Computer? { list.first { $0.id == active } }

    func token(_ c: Computer) -> String? { Keychain.get("vorn.token.\(c.id)") }

    /// Saves a pairing and makes it the one in use; the same address replaces its token.
    func add(name: String, host: String, token: String) {
        let id = list.first { $0.host == host }?.id ?? UUID().uuidString
        Keychain.set("vorn.token.\(id)", token)
        if let i = list.firstIndex(where: { $0.id == id }) {
            list[i].name = name
        } else {
            list.append(Computer(id: id, name: name, host: host))
        }
        select(id)
    }

    func select(_ id: String) {
        active = id
        save()
    }

    func remove(_ id: String) {
        Keychain.delete("vorn.token.\(id)")
        list.removeAll { $0.id == id }
        if active == id { active = list.first?.id }
        save()
    }

    private func save() {
        defaults.set(try? JSONEncoder().encode(list), forKey: Self.listKey)
        defaults.set(active, forKey: Self.activeKey)
    }

    /// Moves the single computer an earlier build kept in settings into the list.
    private func migrate() {
        guard let host = defaults.string(forKey: "vornHost"), !host.isEmpty else { return }
        let token = defaults.string(forKey: "vornToken") ?? ""
        defaults.removeObject(forKey: "vornHost")
        defaults.removeObject(forKey: "vornToken")
        if !token.isEmpty { add(name: Pairing.hostOf(host), host: host, token: token) }
    }

    /// The id half of a `vorn_<id>_<secret>` token, which is not secret; the secret may contain `_`.
    static func tokenId(_ token: String) -> String? {
        guard token.hasPrefix("vorn_") else { return nil }
        let rest = token.dropFirst(5)
        guard let sep = rest.firstIndex(of: "_"), sep > rest.startIndex,
              rest.index(after: sep) < rest.endIndex else { return nil }
        return String(rest[..<sep])
    }
}

enum Keychain {
    private static let service = "run.vorn.phone"

    private static func query(_ key: String) -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: service,
         kSecAttrAccount as String: key]
    }

    static func set(_ key: String, _ value: String) {
        delete(key)
        var q = query(key)
        q[kSecValueData as String] = Data(value.utf8)
        q[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        SecItemAdd(q as CFDictionary, nil)
    }

    static func get(_ key: String) -> String? {
        var q = query(key)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: AnyObject?
        guard SecItemCopyMatching(q as CFDictionary, &out) == errSecSuccess, let data = out as? Data else { return nil }
        return String(decoding: data, as: UTF8.self)
    }

    static func delete(_ key: String) {
        SecItemDelete(query(key) as CFDictionary)
    }
}
