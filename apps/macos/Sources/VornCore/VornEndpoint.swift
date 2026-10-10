import Foundation

/// Where a running vornd listens, and the credential to reach it, as found in
/// its data directory (`ws-port`, `local-token`, `run/vornd-grid-<pid>.sock`).
public struct VornEndpoint: Sendable, Hashable {
    public var dataDir: URL
    public var host: String
    public var port: Int
    /// vornd's process id, which names its grid socket; absent from a bare `ws-port`.
    public var pid: Int?
    public var token: String

    public init(
        dataDir: URL = VornEndpoint.defaultDataDir(), host: String = "127.0.0.1", port: Int, pid: Int? = nil,
        token: String
    ) {
        self.dataDir = dataDir
        self.host = host
        self.port = port
        self.pid = pid
        self.token = token
    }

    public var webSocketURL: URL { URL(string: "ws://\(host):\(port)/ws")! }

    /// The grid endpoint's Unix socket.
    public var gridSocket: String? {
        pid.map { dataDir.appendingPathComponent("run/vornd-grid-\($0).sock").path }
    }

    public enum DiscoveryError: Error, Equatable, LocalizedError {
        case missing(String)
        case malformed(String)

        public var errorDescription: String? {
            switch self {
            case .missing(let path): "vornd is not running (no \(path))"
            case .malformed(let path): "Could not read \(path)"
            }
        }
    }

    /// The data directory: `VORN_DATA_DIR`, else `VORN_HOME`, else `~/.vorn`.
    public static func defaultDataDir(environment: [String: String] = ProcessInfo.processInfo.environment) -> URL {
        for key in ["VORN_DATA_DIR", "VORN_HOME"] {
            if let dir = environment[key], !dir.isEmpty {
                return URL(fileURLWithPath: (dir as NSString).expandingTildeInPath, isDirectory: true)
            }
        }
        return FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".vorn", isDirectory: true)
    }

    /// Reads the endpoint files vornd writes into `dataDir`.
    public static func discover(dataDir: URL = defaultDataDir()) throws -> VornEndpoint {
        let portFile = dataDir.appendingPathComponent("ws-port")
        let tokenFile = dataDir.appendingPathComponent("local-token")
        guard let portData = try? Data(contentsOf: portFile) else { throw DiscoveryError.missing(portFile.path) }
        guard let tokenData = try? Data(contentsOf: tokenFile) else { throw DiscoveryError.missing(tokenFile.path) }
        guard let (port, pid) = parsePort(portData) else { throw DiscoveryError.malformed(portFile.path) }
        let token = String(decoding: tokenData, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !token.isEmpty else { throw DiscoveryError.malformed(tokenFile.path) }
        return VornEndpoint(dataDir: dataDir, port: port, pid: pid, token: token)
    }

    /// Reads `{"port":N,"pid":P}`, or a bare port number.
    static func parsePort(_ data: Data) -> (port: Int, pid: Int?)? {
        if let obj = try? JSONDecoder().decode([String: JSONValue].self, from: data),
           let n = obj["port"]?.numberValue, n > 0, n < 65536 {
            return (Int(n), obj["pid"]?.numberValue.map { Int($0) })
        }
        let text = String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        if let n = Int(text), n > 0, n < 65536 { return (n, nil) }
        return nil
    }
}

public enum VornError: Error, Equatable, LocalizedError {
    case notConnected
    case disconnected
    case rpc(code: Int, message: String)
    case badResponse(String)
    case refused(String)

    public var errorDescription: String? {
        switch self {
        case .notConnected: "Not connected to the Vorn server"
        case .disconnected: "The connection to the Vorn server closed"
        case .rpc(_, let message): message
        case .badResponse(let detail): "Unexpected answer from the server: \(detail)"
        case .refused(let detail): detail
        }
    }
}
