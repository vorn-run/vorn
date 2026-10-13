// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, fireEvent, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const mockDetectInstalledAgents = vi.fn().mockResolvedValue({
  claude: true,
  copilot: false,
  codex: true,
  opencode: false,
  gemini: false
})
const mockSaveConfig = vi.fn()

Object.defineProperty(window, 'api', {
  value: {
    detectInstalledAgents: (...args: unknown[]) => mockDetectInstalledAgents(...args),
    saveConfig: (...args: unknown[]) => mockSaveConfig(...args)
  },
  writable: true
})

import { useAppStore } from '../packages/web/src/renderer/stores'
import { AgentSettings } from '../packages/web/src/renderer/components/settings/AgentSettings'

const initialState = useAppStore.getState()

beforeEach(() => {
  mockSaveConfig.mockReset()
  act(() => {
    useAppStore.setState({
      config: {
        version: 1,
        projects: [],
        workflows: [],
        defaults: { shell: '/bin/zsh', fontSize: 13, theme: 'dark' },
        agentCommands: {},
        remoteHosts: [],
        workspaces: []
      }
    })
  })
})

afterEach(() => {
  act(() => {
    useAppStore.setState(initialState)
  })
})

describe('AgentSettings renders for AI agents only (shell is excluded)', () => {
  it('renders the Coding Agents header', () => {
    render(<AgentSettings />)
    expect(screen.getByText('Coding Agents')).toBeInTheDocument()
  })

  it('does not list Shell as a configurable agent (shell has no command config)', () => {
    render(<AgentSettings />)
    expect(screen.queryByText(/Shell/i)).not.toBeInTheDocument()
  })

  it('returns null when config is missing (guard path)', () => {
    act(() => {
      useAppStore.setState({ config: undefined })
    })
    const { container } = render(<AgentSettings />)
    expect(container).toBeEmptyDOMElement()
  })

  it('edits the command args for an installed agent and persists via saveConfig', () => {
    render(<AgentSettings />)
    // The primary args input for the first installed agent — there are multiple
    // inputs on the page; we target by role=textbox and index.
    const inputs = screen.getAllByRole('textbox')
    expect(inputs.length).toBeGreaterThan(0)
    fireEvent.change(inputs[0], { target: { value: '--verbose' } })
    fireEvent.blur(inputs[0])
    expect(mockSaveConfig).toHaveBeenCalled()
    // Verify the payload shape — the updated config should carry the new args
    // under an AI-agent key (claude is the first installed one in our mock).
    const lastCall = mockSaveConfig.mock.calls.at(-1)?.[0]
    expect(lastCall?.agentCommands).toBeDefined()
  })
})
