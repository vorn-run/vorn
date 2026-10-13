import SwiftUI
import VornCore
import VornUI

/// agent-definitions.ts and agent-defaults.ts: what the settings pages say about each agent.
public struct AgentDefinition {
    public let type: AgentType
    let color: UInt32
    let bgAlpha: Double
    let description: String
    public let installCommand: String
    let installUrl: String
    let command: String
    let headlessArgs: [String]

    var tint: Color { Color(hex: color) }
    public var bg: Color { Color(hex: color, opacity: bgAlpha) }

    public static let all: [AgentDefinition] = [
        AgentDefinition(type: .claude, color: 0xD97757, bgAlpha: 0x20 / 255, description: "Anthropic Claude Code CLI",
                        installCommand: "npm install -g @anthropic-ai/claude-code",
                        installUrl: "https://docs.anthropic.com/en/docs/claude-code",
                        command: "claude", headlessArgs: ["--dangerously-skip-permissions"]),
        AgentDefinition(type: .copilot, color: 0x6CC644, bgAlpha: 0x20 / 255, description: "GitHub Copilot CLI",
                        installCommand: "brew install copilot-cli",
                        installUrl: "https://docs.github.com/en/copilot/using-github-copilot/using-github-copilot-agent-mode",
                        command: "copilot", headlessArgs: ["--allow-all"]),
        AgentDefinition(type: .codex, color: 0x7A9DFF, bgAlpha: 0x20 / 255, description: "OpenAI Codex CLI",
                        installCommand: "npm install -g @openai/codex", installUrl: "https://github.com/openai/codex",
                        command: "codex", headlessArgs: ["-a", "never"]),
        AgentDefinition(type: .opencode, color: 0xFFFFFF, bgAlpha: 0x15 / 255, description: "OpenCode CLI",
                        installCommand: "npm install -g opencode-ai", installUrl: "https://opencode.ai",
                        command: "opencode", headlessArgs: []),
        AgentDefinition(type: .gemini, color: 0x3186FF, bgAlpha: 0x20 / 255, description: "Google Gemini CLI",
                        installCommand: "npm install -g @google/gemini-cli",
                        installUrl: "https://github.com/google-gemini/gemini-cli",
                        command: "gemini", headlessArgs: ["-y"]),
    ]

    public static func of(_ type: AgentType) -> AgentDefinition? { all.first { $0.type == type } }

    /// The default `AgentCommandConfig` as JSON, as `DEFAULT_AGENT_COMMANDS` holds it.
    var defaultCommand: JSONValue {
        var o: [String: JSONValue] = ["command": .string(command), "args": .array([])]
        if !headlessArgs.isEmpty { o["headlessArgs"] = .array(headlessArgs.map { .string($0) }) }
        return .object(o)
    }
}

/// mcp-data.ts: how each agent installs the Vorn plugin.
public struct AgentMcpSetup {
    public let agent: AgentType
    public let commands: [String]
    public var inAgent = false
    public var note: String?

    static let repo = "vorn-run/plugin"
    public static let all: [AgentMcpSetup] = {
        let url = "https://github.com/\(repo)"
        let mcpOnly = "mcp add vorn -- npx -y @vornrun/mcp@latest"
        return [
            AgentMcpSetup(agent: .claude, commands: ["/plugin marketplace add \(repo)", "/plugin install vorn"], inAgent: true),
            AgentMcpSetup(agent: .copilot, commands: ["copilot plugin marketplace add \(repo)", "copilot plugin install vorn@vorn"]),
            AgentMcpSetup(agent: .codex, commands: ["codex plugin marketplace add \(repo)", "codex plugin add vorn@vorn"],
                          note: "Restart, or start a fresh session — skills load at session start."),
            AgentMcpSetup(agent: .opencode, commands: ["opencode \(mcpOnly)", "git clone \(url) ~/.config/opencode/vorn"],
                          note: "Then add \"skills\": { \"paths\": [\"~/.config/opencode/vorn/skills\"] } to ~/.config/opencode/opencode.jsonc"),
            AgentMcpSetup(agent: .gemini, commands: ["gemini extensions install \(url)"],
                          note: "Gemini disables MCP servers in a folder it does not trust, user-level ones included."),
        ]
    }()
}
