import { describe, it, expect } from 'vitest'
import { AGENT_MCP_SETUPS, VORN_PLUGIN_REPO } from '../src/renderer/lib/mcp-data'
import { AGENT_LIST } from '../src/renderer/lib/agent-definitions'

/**
 * What a person is told to type.
 *
 * These strings are copied straight into a terminal, so a wrong one fails in
 * front of someone on their first run. Three of the five were wrong when first
 * written — each looked plausible and none had been executed. The assertions
 * here are the shape checks that can be made without a CLI present; they cannot
 * prove a command works, only that it has not silently reverted to a form we
 * already know to be wrong.
 */

describe('AGENT_MCP_SETUPS', () => {
  it('has one setup per agent', () => {
    expect(AGENT_MCP_SETUPS).toHaveLength(AGENT_LIST.length)
  })

  it('covers all known agent types', () => {
    const setupTypes = AGENT_MCP_SETUPS.map((s) => s.agentType)
    expect(setupTypes.sort()).toEqual(AGENT_LIST.map((a) => a.type).sort())
  })

  it('gives every agent at least one command to run', () => {
    for (const setup of AGENT_MCP_SETUPS) {
      expect({ [setup.agentType]: setup.commands.length > 0 }).toEqual({
        [setup.agentType]: true
      })
      for (const c of setup.commands) expect(c.trim()).toBeTruthy()
    }
  })

  it('points every agent at the plugin, not just the bare server', () => {
    // The reason this change exists: the raw `mcp add` gives an agent the tools
    // with none of the skills that say what they are for.
    for (const setup of AGENT_MCP_SETUPS) {
      const all = setup.commands.join(' ')
      expect({ [setup.agentType]: all.includes(VORN_PLUGIN_REPO) }).toEqual({
        [setup.agentType]: true
      })
    }
  })

  describe('the forms verified against each CLI', () => {
    const setupFor = (t: string) => AGENT_MCP_SETUPS.find((s) => s.agentType === t)!

    it('types Claude’s commands in the agent, not a shell', () => {
      // `/plugin` is a slash command. Told to run it in a terminal, a person
      // gets "command not found" and no reason to think the docs are wrong.
      const claude = setupFor('claude')
      expect(claude.inAgent).toBe(true)
      for (const c of claude.commands) expect(c.startsWith('/plugin')).toBe(true)
    })

    it('registers a marketplace before installing from it', () => {
      // Copilot and Codex both refuse to install a plugin from a marketplace
      // they have not been given.
      for (const t of ['copilot', 'codex']) {
        const { commands } = setupFor(t)
        expect({ [t]: commands[0].includes('marketplace add') }).toEqual({ [t]: true })
        expect({ [t]: commands.length > 1 }).toEqual({ [t]: true })
      }
    })

    it('uses Codex’s `plugin add`, and qualifies the plugin with its marketplace', () => {
      // Codex has no `plugin install`, and rejects a bare `vorn`.
      const { commands } = setupFor('codex')
      const install = commands[commands.length - 1]
      expect(install).toContain('plugin add')
      expect(install).not.toContain('plugin install')
      expect(install).toContain('vorn@vorn')
    })

    it('avoids Copilot’s deprecated install-straight-from-a-repo form', () => {
      // `copilot plugin install <owner>/<repo>` still works but warns it is
      // going away; the marketplace form is the one that will survive.
      const { commands } = setupFor('copilot')
      const install = commands[commands.length - 1]
      expect(install).toContain('vorn@vorn')
      expect(install).not.toContain(VORN_PLUGIN_REPO)
    })

    it('installs Gemini as an extension, which carries the server and the context', () => {
      // Gemini has no skills system and no plugin system — extensions are the
      // only path, and one manifest holds both halves.
      const { commands } = setupFor('gemini')
      expect(commands).toHaveLength(1)
      expect(commands[0]).toContain('extensions install')
    })

    it('gives opencode the server and the skills separately', () => {
      // opencode reads SKILL.md natively, so there is no plugin to install.
      const { commands, note } = setupFor('opencode')
      expect(commands.join(' ')).toContain('mcp add vorn')
      // The remaining step is JSON config, which no command line expresses.
      expect(note).toContain('skills')
    })

    it('never claims opencode or Gemini install a plugin', () => {
      // Both had a documented `plugin install` that could not work: opencode's
      // pointed at an unpublished package, Gemini's at a system it does not have.
      for (const t of ['opencode', 'gemini']) {
        expect({ [t]: setupFor(t).commands.join(' ').includes('plugin install') }).toEqual({
          [t]: false
        })
      }
    })
  })
})
