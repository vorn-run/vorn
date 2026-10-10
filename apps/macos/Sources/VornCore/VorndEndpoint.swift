import Foundation

/// Where a vornd listens and the token it takes, read from its data directory.
public struct VorndEndpoint: Sendable, Hashable {
    public var url: URL
    public var token: String

    public init(url: URL, token: String) {
        self.url = url
        self.token = token
    }

    public init(host: String = "127.0.0.1", port: Int, token: String) {
        self.url = URL(string: "ws://\(host):\(port)/ws")!
        self.token = token
    }

    /// The app's own data directory, ~/.vorn.
    public static var defaultDataDirectory: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".vorn", isDirectory: true)
    }

    public enum DiscoveryError: Error, Equatable, CustomStringConvertible {
        case missing(String)
        case malformed(String)

        public var description: String {
            switch self {
            case .missing(let p): return "vornd is not running (no \(p))"
            case .malformed(let p): return "could not read \(p)"
            }
        }
    }

    /// Reads `ws-port` ({"port":N,"pid":P}) and `local-token` from a data directory.
    public static func discover(dataDirectory: URL = defaultDataDirectory) throws -> VorndEndpoint {
        let portFile = dataDirectory.appendingPathComponent("ws-port")
        let tokenFile = dataDirectory.appendingPathComponent("local-token")
        guard let portData = try? Data(contentsOf: portFile) else {
            throw DiscoveryError.missing(portFile.path)
        }
        guard let tokenData = try? Data(contentsOf: tokenFile) else {
            throw DiscoveryError.missing(tokenFile.path)
        }
        guard let port = parsePort(portData) else { throw DiscoveryError.malformed(portFile.path) }
        let token = String(decoding: tokenData, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !token.isEmpty else { throw DiscoveryError.malformed(tokenFile.path) }
        return VorndEndpoint(port: port, token: token)
    }

    /// Accepts the JSON form or a bare port number.
    static func parsePort(_ data: Data) -> Int? {
        if let obj = try? JSONDecoder().decode([String: JSONValue].self, from: data),
           let n = obj["port"]?.numberValue, n > 0, n < 65536 {
            return Int(n)
        }
        let text = String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        if let n = Int(text), n > 0, n < 65536 { return n }
        return nil
    }
}
