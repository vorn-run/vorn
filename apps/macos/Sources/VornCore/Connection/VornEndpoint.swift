import Foundation

/// Where a vornd listens and the credential it accepts, read from its data directory.
public struct VornEndpoint: Sendable, Equatable {
    public var host: String
    public var port: Int
    public var token: String

    public init(host: String = "127.0.0.1", port: Int, token: String) {
        self.host = host
        self.port = port
        self.token = token
    }

    public var socketURL: URL {
        URL(string: "ws://\(host):\(port)/ws")!
    }

    /// The running app's data directory.
    public static var defaultDataDirectory: URL {
        if let home = ProcessInfo.processInfo.environment["VORN_HOME"], !home.isEmpty {
            return URL(fileURLWithPath: home, isDirectory: true)
        }
        return URL(fileURLWithPath: NSHomeDirectory(), isDirectory: true).appendingPathComponent(".vorn", isDirectory: true)
    }

    /// Reads `ws-port` (`{"port":N,"pid":P}`) and `local-token` from `dataDirectory`.
    public static func read(dataDirectory: URL) throws -> VornEndpoint {
        struct PortFile: Decodable { let port: Int }
        let portData = try Data(contentsOf: dataDirectory.appendingPathComponent("ws-port"))
        let port = try JSONDecoder().decode(PortFile.self, from: portData).port
        let token = try String(contentsOf: dataDirectory.appendingPathComponent("local-token"), encoding: .utf8)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !token.isEmpty else { throw VornError.missingCredential }
        return VornEndpoint(port: port, token: token)
    }
}

public enum VornError: Error, Equatable, LocalizedError {
    case missingCredential
    case notConnected
    case disconnected
    case rpc(code: Int, message: String)
    case badResponse(String)
    case refused(String)

    public var errorDescription: String? {
        switch self {
        case .missingCredential: "The server's credential is missing"
        case .notConnected: "Not connected to the Vorn server"
        case .disconnected: "The connection to the Vorn server closed"
        case .rpc(_, let message): message
        case .badResponse(let detail): "Unexpected answer from the server: \(detail)"
        case .refused(let detail): detail
        }
    }
}
