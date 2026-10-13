import Foundation

/// extension-copy.ts: how an extension's asks, reach and surfaces read in a sentence.
public enum ExtensionCopy {
    public enum Bucket: String, Hashable, Sendable, CaseIterable {
        case reads = "Reads", sends = "Sends", renames = "Renames"
    }

    public struct PermissionRow: Hashable, Sendable {
        public var label: Bucket
        public var items: [String]
    }

    /// EXTENSION_PERMISSION.
    public static func permission(_ p: ExtensionPermission) -> (bucket: Bucket, phrase: String) {
        switch p {
        case .gitRead: (.reads, "the worktree's diff and status")
        case .terminalRead: (.reads, "the session's recent terminal output")
        case .terminalSelection: (.reads, "the text selected in the terminal")
        case .agentUsage: (.reads, "the agent's context and provider allowance")
        case .terminalSend: (.sends, "text into the session's terminal")
        case .cardRename: (.renames, "the session card")
        }
    }

    static let permissionOrder: [ExtensionPermission] = [.gitRead, .terminalRead, .terminalSelection, .agentUsage, .terminalSend, .cardRename]

    public static func permissionRows(_ permissions: [ExtensionPermission]?) -> [PermissionRow] {
        let asked = Set(permissions ?? [])
        return Bucket.allCases.map { bucket in
            PermissionRow(label: bucket, items: permissionOrder.filter { asked.contains($0) && permission($0).bucket == bucket }
                .map { permission($0).phrase })
        }.filter { !$0.items.isEmpty }
    }

    public static func notAsked(_ permissions: [ExtensionPermission]?) -> [String] {
        let asked = Set(permissions ?? [])
        return permissionOrder.filter { !asked.contains($0) }.map { permission($0).phrase }
    }

    static func anyOf(_ values: [String]) -> String {
        guard values.count > 1 else { return values.first ?? "" }
        return "\(values.dropLast().joined(separator: ", ")) or \(values[values.count - 1])"
    }

    static func agentLabel(_ agent: String) -> String {
        agent == "shell" ? "shell" : AgentType(rawValue: agent).displayName
    }

    public static func describeActivation(_ activates: ExtensionActivation?) -> [String] {
        var clauses: [String] = []
        let contains = activates?.workspaceContains.filter { !$0.isEmpty } ?? []
        if !contains.isEmpty { clauses.append("projects with \(anyOf(contains))") }
        let hostLabels = ["github.com": "GitHub", "gitlab.com": "GitLab", "bitbucket.org": "Bitbucket"]
        let hosts = activates?.remoteHost.filter { !$0.isEmpty } ?? []
        if !hosts.isEmpty { clauses.append("\(anyOf(hosts.map { hostLabels[$0] ?? $0 })) remotes") }
        let agents = activates?.agent ?? []
        if !agents.isEmpty { clauses.append("\(anyOf(agents.map(agentLabel))) sessions") }
        let platformLabels = ["darwin": "macOS", "linux": "Linux", "win32": "Windows"]
        let platforms = activates?.platform ?? []
        if !platforms.isEmpty { clauses.append(anyOf(platforms.map { platformLabels[$0] ?? $0 })) }
        return clauses.isEmpty ? ["every session"] : clauses
    }

    public static func describeContributions(_ contributes: ExtensionContributions?) -> String {
        [(contributes?.panes.count ?? 0, "pane"), (contributes?.footers.count ?? 0, "footer"),
         (contributes?.linkHandlers.count ?? 0, "link handler")]
            .filter { $0.0 > 0 }.map { count($0.0, $0.1) }.joined(separator: ", ")
    }

    public static func describePaneKind(_ pane: ExtensionContribution) -> String {
        if !pane.command.isEmpty { return "runs \(pane.command.joined(separator: " "))" }
        if pane.web != nil { return "a page it ships" }
        return "a pane"
    }

    public static func describeFooterInterval(_ seconds: Int) -> String {
        if seconds >= 60 && seconds % 60 == 0 {
            let minutes = seconds / 60
            return "every \(minutes == 1 ? "minute" : "\(minutes) minutes")"
        }
        return "every \(seconds)s"
    }

    public static func count(_ n: Int, _ noun: String) -> String { "\(n) \(noun)\(n == 1 ? "" : "s")" }

    /// cron-text.ts humanCron: only the shapes connector seeding writes; anything else verbatim.
    public static func humanCron(_ cron: String) -> String {
        if cron == "* * * * *" { return "every minute" }
        let parts = cron.split(separator: " ", omittingEmptySubsequences: false)
        if parts.count == 5, parts[0].hasPrefix("*/"), parts[1...].allSatisfy({ $0 == "*" }) {
            let n = parts[0].dropFirst(2)
            if !n.isEmpty, n.allSatisfy({ $0.isASCII && $0.isNumber }) { return "every \(n) minute\(n == "1" ? "" : "s")" }
        }
        return cron
    }

    /// connector-origins.ts originLabel.
    public static func originLabel(_ origin: String) -> String {
        origin.hasPrefix("https://") ? String(origin.dropFirst(8)) : origin
    }

    static let sensitiveEnvPrefixes = [
        "AWS_SECRET", "AWS_SESSION", "GITHUB_TOKEN", "GH_TOKEN", "OPENAI_API", "ANTHROPIC_API", "GOOGLE_API", "STRIPE_",
        "DATABASE_URL", "DB_PASSWORD", "SECRET_", "PRIVATE_KEY", "NPM_TOKEN", "NODE_AUTH_TOKEN",
    ]

    /// types.ts borrowableFromManifest: what a CLI-signed connector takes from the signed-in tool.
    public static func borrowable(auth: SdkAuth?, env: [ConnectorEnvVar]) -> [String] {
        let declared = Dictionary(env.map { ($0.name.uppercased(), $0.name) }, uniquingKeysWith: { _, last in last })
        return (auth?.borrowEnv ?? []).compactMap { declared[$0.uppercased()] }.filter { name in
            let upper = name.uppercased()
            let never = upper == "CLAUDECODE" || upper.hasPrefix("CLAUDE_CODE_")
            return !never && !sensitiveEnvPrefixes.contains { upper.hasPrefix($0) }
        }
    }

    /// types.ts sdkConnectionFilters: what a package connection records about its package.
    public static func sdkFilters(id: String, version: String, icon: ConnectorIconData?, trigger: String?) -> [String: JSONValue] {
        var filters: [String: JSONValue] = [SdkFilterKey.connectorId: .string(id), SdkFilterKey.version: .string(version)]
        if let icon {
            let json: JSONValue = .object(["viewBox": .string(icon.viewBox), "paths": .array(icon.paths.map { .string($0) })])
            filters[SdkFilterKey.icon] = .string(json.compactJSON)
        }
        if let trigger, !trigger.isEmpty { filters[SdkFilterKey.trigger] = .string(trigger) }
        return filters
    }
}

extension JSONValue {
    /// Compact JSON text, the way JSON.stringify writes it into a filter.
    public var compactJSON: String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        return (try? encoder.encode(self)).flatMap { String(data: $0, encoding: .utf8) } ?? "null"
    }
}
