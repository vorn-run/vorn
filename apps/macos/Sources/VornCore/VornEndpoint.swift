import Foundation

/// Where a running vornd listens, and the credential to reach it, as found in
/// its data directory (`ws-port`, `local-token`, `run/vornd-grid-<pid>.sock`).
public struct VornEndpoint: Sendable, Equatable {
    public var dataDir: URL
    public var port: Int
    public var pid: Int
    public var token: String

    public init(dataDir: URL, port: Int, pid: Int, token: String) {
        self.dataDir = dataDir
        self.port = port
        self.pid = pid
        self.token = token
    }

    public var webSocketURL: URL { URL(string: "ws://127.0.0.1:\(port)/ws")! }

    /// The grid endpoint's Unix socket.
    public var gridSocket: String {
        dataDir.appendingPathComponent("run/vornd-grid-\(pid).sock").path
    }

    public enum DiscoveryError: Error, LocalizedError {
        case notRunning(URL)
        case unreadable(URL)

        public var errorDescription: String? {
            switch self {
            case .notRunning(let dir): "No vornd is running for \(dir.path)"
            case .unreadable(let file): "Could not read \(file.path)"
            }
        }
    }

    /// The data directory: `VORN_DATA_DIR` if set, else `~/.vorn`.
    public static func defaultDataDir(environment: [String: String] = ProcessInfo.processInfo.environment) -> URL {
        if let dir = environment["VORN_DATA_DIR"], !dir.isEmpty {
            return URL(fileURLWithPath: (dir as NSString).expandingTildeInPath, isDirectory: true)
        }
        return FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".vorn", isDirectory: true)
    }

    /// Reads the endpoint files vornd writes into `dataDir`.
    public static func discover(dataDir: URL) throws -> VornEndpoint {
        struct PortFile: Decodable { let port: Int; let pid: Int }
        let portURL = dataDir.appendingPathComponent("ws-port")
        let tokenURL = dataDir.appendingPathComponent("local-token")
        guard let portData = try? Data(contentsOf: portURL) else { throw DiscoveryError.notRunning(dataDir) }
        guard let file = try? JSONDecoder().decode(PortFile.self, from: portData) else {
            throw DiscoveryError.unreadable(portURL)
        }
        guard let token = try? String(contentsOf: tokenURL, encoding: .utf8) else {
            throw DiscoveryError.unreadable(tokenURL)
        }
        return VornEndpoint(
            dataDir: dataDir, port: file.port, pid: file.pid,
            token: token.trimmingCharacters(in: .whitespacesAndNewlines)
        )
    }
}
