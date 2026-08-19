// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, screen, within, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

import { McpSettings } from '../src/renderer/components/settings/McpSettings'
import { AGENT_MCP_SETUPS } from '../src/renderer/lib/mcp-data'

/**
 * That the panel shows a whole install, not the first line of one.
 *
 * Most agents now need a marketplace registered before they will install from
 * it, so a panel that renders `commands[0]` looks entirely correct and leaves
 * the person one step short — with no error, because the step they were given
 * succeeds on its own. The data tests cannot see this; only rendering can.
 */

// Stubbing the global rather than mutating `navigator`: the real object can be
// non-configurable in jsdom, and a mutation would leak into later cases.
afterEach(() => vi.unstubAllGlobals())

describe('McpSettings', () => {
  it('shows every command, not just the first', () => {
    render(<McpSettings />)
    for (const setup of AGENT_MCP_SETUPS) {
      for (const command of setup.commands) {
        expect(screen.getByText(command), `${setup.agentType}: ${command}`).toBeInTheDocument()
      }
    }
  })

  it('gives each command its own copy button', () => {
    // One button per command. A single button for a two-step install would copy
    // one of them and silently drop the other.
    render(<McpSettings />)
    const total = AGENT_MCP_SETUPS.reduce((n, s) => n + s.commands.length, 0)
    expect(screen.getAllByRole('button', { name: 'Copy' })).toHaveLength(total)
  })

  it('says where to run the commands, per agent', () => {
    // The same trap onboarding has: Claude's `/plugin` is a slash command, and
    // a panel that only names the agent leaves someone to assume a shell.
    render(<McpSettings />)
    expect(screen.getByText('Run in Claude Code')).toBeInTheDocument()
    expect(screen.getAllByText('Run in your terminal').length).toBeGreaterThan(0)
  })

  it('shows the note for an agent whose setup a command line cannot express', () => {
    // opencode's second half is JSON config; without the note the panel would
    // imply the two commands finish the job.
    render(<McpSettings />)
    const opencode = AGENT_MCP_SETUPS.find((s) => s.agentType === 'opencode')!
    expect(screen.getByText(opencode.note!)).toBeInTheDocument()
  })

  it('copies the command next to the button that was pressed', async () => {
    const writeText = vi.fn()
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    render(<McpSettings />)

    const claude = AGENT_MCP_SETUPS.find((s) => s.agentType === 'claude')!
    const second = claude.commands[1]
    const row = screen.getByText(second).closest('div')!
    fireEvent.click(within(row).getByRole('button', { name: 'Copy' }))

    expect(writeText).toHaveBeenCalledWith(second)
  })
})
