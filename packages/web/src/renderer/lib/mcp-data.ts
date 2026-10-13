import { AiAgentType } from '../../shared/types'

/**
 * How a person connects one agent to Vorn.
 *
 * The plugin rather than a bare `mcp add`, wherever the agent has a plugin
 * system. Both install the same MCP server; the plugin also carries the skills
 * that say what the tools are *for*. Agents defer tool descriptions, so a name
 * like `browser_interact` otherwise arrives with no documentation and goes
 * unused — the tools are present and invisible.
 *
 * Every command here was run against the real CLI. That is not ceremony: three
 * of the five were wrong the first time they were written, in ways no amount of
 * reading the docs would have caught.
 */

export interface AgentMcpSetup {
  agentType: AiAgentType
  /**
   * Ordered steps. More than one where the harness wants a marketplace
   * registered before it will install from it.
   */
  commands: string[]
  /**
   * Typed inside the agent rather than in a shell. Claude's `/plugin` is a
   * slash command, so telling someone to run it in a terminal sends them
   * somewhere it does nothing.
   */
  inAgent?: boolean
  /** For what a command line cannot express — see opencode. */
  note?: string
}

/** The plugin's repo. Also the marketplace source, which is the same thing here. */
export const VORN_PLUGIN_REPO = 'vorn-run/plugin'

const PLUGIN_URL = `https://github.com/${VORN_PLUGIN_REPO}`

/** The raw server, for agents with no plugin path and as the opencode first step. */
const MCP_ONLY = `mcp add vorn -- npx -y @vornrun/mcp@latest`

export const AGENT_MCP_SETUPS: AgentMcpSetup[] = [
  {
    agentType: 'claude',
    commands: [`/plugin marketplace add ${VORN_PLUGIN_REPO}`, '/plugin install vorn'],
    inAgent: true
  },
  {
    agentType: 'copilot',
    // `copilot plugin install vorn-run/plugin` also works, but installing
    // straight from a repo is deprecated and warns that it will be removed.
    commands: [
      `copilot plugin marketplace add ${VORN_PLUGIN_REPO}`,
      'copilot plugin install vorn@vorn'
    ]
  },
  {
    agentType: 'codex',
    // Codex has no `plugin install` — only `plugin add`, and only from a
    // registered marketplace. It rejects a bare name, wanting plugin@marketplace.
    commands: [`codex plugin marketplace add ${VORN_PLUGIN_REPO}`, 'codex plugin add vorn@vorn'],
    note: 'Restart, or start a fresh session — skills load at session start.'
  },
  {
    agentType: 'opencode',
    // opencode reads SKILL.md natively through its own `skill` tool, so there is
    // no plugin to install: the server and a path to the skills, separately. The
    // second step is JSON config rather than a command, which is why it is a note.
    commands: [`opencode ${MCP_ONLY}`, `git clone ${PLUGIN_URL} ~/.config/opencode/vorn`],
    note: 'Then add "skills": { "paths": ["~/.config/opencode/vorn/skills"] } to ~/.config/opencode/opencode.jsonc'
  },
  {
    agentType: 'gemini',
    // Gemini has no skills system, but its extensions carry both an MCP server
    // and a context file — so one install still delivers both halves.
    commands: [`gemini extensions install ${PLUGIN_URL}`],
    note: 'Gemini disables MCP servers in a folder it does not trust, user-level ones included.'
  }
]
