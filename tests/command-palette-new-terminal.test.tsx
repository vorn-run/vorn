// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, fireEvent, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

// jsdom doesn't implement scrollIntoView; CommandPalette scrolls the active
// command into view on mount.
if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = vi.fn()
}

// Stub animations so framer-motion doesn't defer render
vi.mock('framer-motion', () => ({
  motion: {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    div: ({ children, ...props }: any) => <div {...props}>{children}</div>
  },
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>
}))

vi.mock('../src/renderer/lib/session-utils', async (orig) => {
  const actual = (await orig()) as Record<string, unknown>
  return {
    ...actual,
    resolveActiveProject: () => ({
      name: 'vorn',
      path: '/tmp/vorn',
      preferredAgents: ['claude']
    })
  }
})

const mockCreateShellTerminal = vi.fn()
const mockIsGitRepo = vi.fn().mockResolvedValue(false)
const mockGetRecentSessions = vi.fn().mockResolvedValue([])
const mockDetectInstalledAgents = vi.fn().mockResolvedValue({
  claude: true,
  copilot: false,
  codex: false,
  opencode: false,
  gemini: false
})

Object.defineProperty(window, 'api', {
  value: {
    createShellTerminal: (...args: unknown[]) => mockCreateShellTerminal(...args),
    isGitRepo: (...args: unknown[]) => mockIsGitRepo(...args),
    getRecentSessions: (...args: unknown[]) => mockGetRecentSessions(...args),
    detectInstalledAgents: (...args: unknown[]) => mockDetectInstalledAgents(...args),
    saveConfig: vi.fn(),
    notifyWidgetStatus: vi.fn()
  },
  writable: true
})

import { useAppStore } from '../src/renderer/stores'
import { CommandPalette } from '../src/renderer/components/CommandPalette'

const initialState = useAppStore.getState()

beforeEach(() => {
  mockCreateShellTerminal.mockReset()
  act(() => {
    useAppStore.setState({
      isCommandPaletteOpen: true,
      terminals: new Map(),
      config: {
        version: 1,
        projects: [{ name: 'vorn', path: '/tmp/vorn', preferredAgents: ['claude'] }],
        workflows: [],
        defaults: { shell: '/bin/zsh', fontSize: 13, theme: 'dark' },
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

describe('CommandPalette — New Terminal Session action', () => {
  it('surfaces a "New Terminal Session" entry in the palette', () => {
    render(<CommandPalette />)
    expect(screen.getByText('New Terminal Session')).toBeInTheDocument()
  })

  it('clicking the entry creates a shell in the active project cwd and activates it', async () => {
    const shellSession = {
      id: 'sh-42',
      agentType: 'shell' as const,
      projectName: 'vorn',
      projectPath: '/tmp/vorn',
      status: 'running' as const,
      createdAt: Date.now(),
      pid: 9999,
      displayName: 'Shell 1'
    }
    mockCreateShellTerminal.mockResolvedValue(shellSession)

    const addTerminal = vi.fn()
    const setActiveTabId = vi.fn()
    act(() => {
      useAppStore.setState({ addTerminal, setActiveTabId })
    })

    render(<CommandPalette />)
    fireEvent.click(screen.getByText('New Terminal Session'))
    // onClick is async
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0))
    })

    expect(mockCreateShellTerminal).toHaveBeenCalledWith('/tmp/vorn')
    expect(addTerminal).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'sh-42', projectName: 'vorn' })
    )
    expect(setActiveTabId).toHaveBeenCalledWith('sh-42')
  })

  it('does not expose "Toggle Terminal Panel" anymore', () => {
    render(<CommandPalette />)
    expect(screen.queryByText(/Toggle Terminal Panel/i)).not.toBeInTheDocument()
  })
})

describe('CommandPalette — per-project and per-worktree Terminal entries', () => {
  async function renderAndQuery(q: string) {
    const result = render(<CommandPalette />)
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0))
    })
    const input = result.container.querySelector('input[placeholder="Type a command..."]')!
    act(() => {
      fireEvent.change(input, { target: { value: q } })
    })
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0))
    })
    return result
  }

  it('surfaces "Terminal in <project>" when the user types a matching query', async () => {
    await renderAndQuery('Terminal in vorn')
    expect(screen.getByText((t) => t.includes('Terminal in vorn'))).toBeInTheDocument()
  })

  it('clicking "Terminal in <project>" creates a shell in that project path', async () => {
    const shellSession = {
      id: 'sh-99',
      agentType: 'shell' as const,
      projectName: 'vorn',
      projectPath: '/tmp/vorn',
      status: 'running' as const,
      createdAt: Date.now(),
      pid: 9999,
      displayName: 'Shell 1'
    }
    mockCreateShellTerminal.mockResolvedValue(shellSession)
    const addTerminal = vi.fn()
    const setActiveTabId = vi.fn()
    act(() => {
      useAppStore.setState({ addTerminal, setActiveTabId })
    })

    await renderAndQuery('Terminal in vorn')
    const entry = screen.getByText((t) => t.includes('Terminal in vorn'))
    fireEvent.click(entry)
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0))
    })

    expect(mockCreateShellTerminal).toHaveBeenCalledWith('/tmp/vorn')
    expect(addTerminal).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'sh-99', projectName: 'vorn' })
    )
  })

  it('clicking a worktree Terminal entry creates a shell in the worktree path', async () => {
    mockIsGitRepo.mockResolvedValue(true)
    const nonMainWt = {
      path: '/tmp/vorn/.vorn-worktrees/vorn/feature-a',
      branch: 'feature-a',
      name: 'feature-a',
      isMain: false,
      isDirty: false
    }
    const shellSession = {
      id: 'sh-100',
      agentType: 'shell' as const,
      projectName: 'feature-a',
      projectPath: nonMainWt.path,
      status: 'running' as const,
      createdAt: Date.now(),
      pid: 10000,
      displayName: 'Shell 1'
    }
    mockCreateShellTerminal.mockResolvedValue(shellSession)
    const addTerminal = vi.fn()
    const setActiveTabId = vi.fn()
    act(() => {
      useAppStore.setState({
        addTerminal,
        setActiveTabId,
        worktreeCache: new Map([['/tmp/vorn', [nonMainWt]]])
      })
    })

    const { container } = await renderAndQuery('feature-a')
    const entry = Array.from(container.querySelectorAll('[data-cmd-index]')).find(
      (el) => el.textContent?.includes('Terminal in vorn') && el.textContent?.includes('feature-a')
    )
    expect(entry).toBeDefined()
    fireEvent.click(entry!)
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0))
    })

    expect(mockCreateShellTerminal).toHaveBeenCalledWith(nonMainWt.path)
  })
})

describe('CommandPalette — shell terminal entry uses the Shell label', () => {
  it('labels shell sessions as Shell (no AGENT_DEFINITIONS lookup)', () => {
    act(() => {
      useAppStore.setState({
        terminals: new Map([
          [
            'sh-1',
            {
              id: 'sh-1',
              session: {
                id: 'sh-1',
                agentType: 'shell' as const,
                projectName: 'vorn',
                projectPath: '/tmp/vorn',
                status: 'running' as const,
                createdAt: Date.now(),
                pid: 1,
                displayName: 'Shell 1'
              },
              status: 'running' as const,
              lastOutputTimestamp: Date.now()
            }
          ]
        ])
      })
    })

    render(<CommandPalette />)
    // The command palette renders one entry per terminal; the shell one should be present
    // and not throw (which would happen if AGENT_DEFINITIONS['shell'] were accessed).
    expect(screen.getByText('Shell 1')).toBeInTheDocument()
  })
})

describe('CommandPalette — popped-out cards', () => {
  it('lists a card, so the palette can reach everything the grid can', () => {
    // A card has a cell, a tab, a sidebar row, a dock pill and a focus stage.
    // Leaving it out made the palette the one way of reaching a thing that
    // could not reach half of them.
    act(() => {
      useAppStore.setState({
        terminals: new Map([
          [
            't1',
            {
              id: 't1',
              session: {
                id: 't1',
                agentType: 'claude' as const,
                projectName: 'vorn',
                projectPath: '/repo',
                status: 'running' as const,
                createdAt: Date.now(),
                pid: 1,
                branch: 'main',
                displayName: 'Session One'
              },
              status: 'running' as const,
              lastOutputTimestamp: Date.now()
            }
          ]
        ])
      })
      useAppStore.getState().promoteFile('t1', '/repo/src/server.ts')
      useAppStore.getState().openBrowserPane('t1', 'vorn.dev')
      useAppStore.getState().promoteBrowserTab('t1', 0)
    })

    render(<CommandPalette />)

    expect(screen.getByText('server.ts')).toBeInTheDocument()
    expect(screen.getByText('vorn.dev')).toBeInTheDocument()
  })

  it('focuses the card when its entry is chosen', () => {
    let cardId = ''
    act(() => {
      useAppStore.setState({
        terminals: new Map([
          [
            't1',
            {
              id: 't1',
              session: {
                id: 't1',
                agentType: 'claude' as const,
                projectName: 'vorn',
                projectPath: '/repo',
                status: 'running' as const,
                createdAt: Date.now(),
                pid: 1,
                displayName: 'Session One'
              },
              status: 'running' as const,
              lastOutputTimestamp: Date.now()
            }
          ]
        ])
      })
      cardId = useAppStore.getState().promoteFile('t1', '/repo/notes.md')
    })

    render(<CommandPalette />)
    fireEvent.click(screen.getByText('notes.md'))

    expect(useAppStore.getState().focusedTerminalId).toBe(cardId)
  })
})
