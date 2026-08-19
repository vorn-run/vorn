// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, cleanup, fireEvent, within } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

import { AGENT_MCP_SETUPS } from '../src/renderer/lib/mcp-data'

/**
 * The install instructions a person meets first.
 *
 * Onboarding is where someone connects their agent before they have any feel
 * for what is normal, so a wrong instruction here reads as "this app is broken"
 * rather than "that line was out of date". The case that matters most is the
 * heading: Claude's commands are typed in the agent, and under "Run in your
 * terminal" they produce "command not found" with nothing pointing at the real
 * cause.
 */

vi.hoisted(() => {
  Object.defineProperty(window, 'api', {
    value: {
      detectInstalledAgents: () => Promise.resolve(['claude', 'opencode']),
      listDir: () => Promise.resolve([])
    },
    writable: true,
    // Configurable so a later test file can redefine it — many re-stub `window.api`,
    // and a non-configurable stub here would make them throw.
    configurable: true
  })
  Object.defineProperty(window, 'matchMedia', {
    value: () => ({ matches: false, addEventListener: () => {}, removeEventListener: () => {} }),
    writable: true,
    configurable: true
  })
})

// Detection reaches out to the host, so pin it: these cases are about what the
// step renders for a given set of agents, not about finding them.
vi.mock('../src/renderer/hooks/useAgentInstallStatus', () => ({
  useAgentInstallStatus: () => ({
    status: { claude: true, opencode: true, copilot: false, codex: false, gemini: false },
    loading: false,
    refresh: () => Promise.resolve()
  })
}))

import { OnboardingModal } from '../src/renderer/components/OnboardingModal'
import { useAppStore } from '../src/renderer/stores'

const claude = AGENT_MCP_SETUPS.find((s) => s.agentType === 'claude')!
const opencode = AGENT_MCP_SETUPS.find((s) => s.agentType === 'opencode')!

/** The commands sit behind a disclosure that starts closed. */
function openMcpSection(): void {
  render(<OnboardingModal />)
  fireEvent.click(screen.getByText(/Install skills & MCP server/i))
}

afterEach(() => vi.unstubAllGlobals())

beforeEach(() => {
  cleanup()
  useAppStore.setState({ isOnboardingOpen: true })
})

describe('onboarding — connecting an agent', () => {
  it('shows every step of a multi-step install', () => {
    // Claude needs the marketplace registered before the install will resolve.
    // Showing only the first line leaves someone with a step that succeeds and
    // an agent that never gains the tools.
    openMcpSection()
    for (const command of claude.commands) {
      expect(screen.getByText(command)).toBeInTheDocument()
    }
  })

  it('says to type Claude’s commands in Claude, not a terminal', () => {
    openMcpSection()
    expect(screen.getByText(/Run in Claude Code/i)).toBeInTheDocument()
  })

  it('still says terminal for an agent whose commands are shell commands', () => {
    openMcpSection()
    expect(screen.getByText(/Run in your terminal/i)).toBeInTheDocument()
  })

  it('shows the note for a setup a command line cannot finish', () => {
    // opencode's remaining step is JSON config; without it the two commands
    // read as the whole job.
    openMcpSection()
    expect(screen.getByText(opencode.note!)).toBeInTheDocument()
  })

  it('gives each command its own copy button', () => {
    const writeText = vi.fn()
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    openMcpSection()

    const second = claude.commands[1]
    const row = screen.getByText(second).closest('div')!
    fireEvent.click(within(row).getByRole('button'))

    expect(writeText).toHaveBeenCalledWith(second)
  })

  it('offers only the agents that are actually installed', () => {
    // Copilot is not installed here, so its commands would be noise.
    openMcpSection()
    const copilot = AGENT_MCP_SETUPS.find((s) => s.agentType === 'copilot')!
    expect(screen.queryByText(copilot.commands[0])).not.toBeInTheDocument()
  })
})
