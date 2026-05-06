// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

// Mock dependencies before imports
const mockCreateTerminal = vi.fn()
const mockCreateShellTerminal = vi.fn()
const mockListBranches = vi.fn()
const mockListWorktrees = vi.fn()

Object.defineProperty(window, 'api', {
  value: {
    createTerminal: (...args: unknown[]) => mockCreateTerminal(...args),
    createShellTerminal: (...args: unknown[]) => mockCreateShellTerminal(...args),
    listBranches: (...args: unknown[]) => mockListBranches(...args),
    listWorktrees: (...args: unknown[]) => mockListWorktrees(...args),
    detectInstalledAgents: vi.fn().mockResolvedValue({
      claude: true,
      copilot: true,
      codex: false,
      opencode: false,
      gemini: false
    }),
    killTerminal: vi.fn(),
    saveConfig: vi.fn(),
    notifyWidgetStatus: vi.fn(),
    isWorktreeDirty: vi.fn().mockResolvedValue(false),
    getGitDiffStat: vi.fn().mockResolvedValue(null)
  },
  writable: true
})

vi.mock('../src/renderer/components/Toast', () => ({
  toast: { success: vi.fn(), error: vi.fn() }
}))

vi.mock('../src/renderer/lib/terminal-close', () => ({
  closeTerminalSession: vi.fn()
}))

vi.mock('../src/renderer/hooks/useIsMobile', () => ({
  useIsMobile: () => false
}))

const mockExecuteWorkflow = vi.fn()
vi.mock('../src/renderer/lib/workflow-execution', () => ({
  executeWorkflow: (...args: unknown[]) => mockExecuteWorkflow(...args)
}))

import { useAppStore } from '../src/renderer/stores'
import { CardContextMenu } from '../src/renderer/components/CardContextMenu'

const mockTerminal = {
  id: 'term-1',
  session: {
    id: 'term-1',
    agentType: 'claude' as const,
    projectName: 'Vorn',
    projectPath: '/tmp/vorn',
    isWorktree: false,
    branch: 'main'
  },
  status: 'idle' as const,
  lastOutputTimestamp: Date.now()
}

const mockWorktreeTerminal = {
  id: 'term-2',
  session: {
    id: 'term-2',
    agentType: 'claude' as const,
    projectName: 'Vorn',
    projectPath: '/tmp/vorn',
    isWorktree: true,
    branch: 'feature-auth',
    worktreePath: '/tmp/.vorn-worktrees/vorn/feature-auth'
  },
  status: 'running' as const,
  lastOutputTimestamp: Date.now()
}

const mockConfig = {
  projects: [
    {
      name: 'Vorn',
      path: '/tmp/vorn',
      icon: 'Rocket',
      iconColor: '#ff0000',
      preferredAgents: ['claude' as const]
    }
  ],
  workflows: [],
  defaults: { defaultAgent: 'claude' as const, rowHeight: 208 },
  remoteHosts: [],
  workspaces: []
}

beforeEach(() => {
  vi.clearAllMocks()
  mockListWorktrees.mockResolvedValue([])
  mockListBranches.mockResolvedValue({ current: 'main', branches: [] })
  const terminals = new Map()
  terminals.set('term-1', mockTerminal)
  terminals.set('term-2', mockWorktreeTerminal)

  useAppStore.setState({
    terminals,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    config: mockConfig as any,
    focusedTerminalId: null,
    worktreeCache: new Map(),
    activeProject: 'Vorn',
    activeWorktreePath: null
  })
})

describe('CardContextMenu', () => {
  it('renders "New session" quick-launch', () => {
    render(<CardContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={vi.fn()} />)
    expect(screen.getByText('New session')).toBeInTheDocument()
  })

  it('quick-launch creates terminal in same project', async () => {
    mockCreateTerminal.mockResolvedValue({
      id: 'new-term',
      session: {
        id: 'new-term',
        agentType: 'claude',
        projectName: 'Vorn',
        projectPath: '/tmp/vorn'
      },
      status: 'idle',
      lastOutputTimestamp: Date.now()
    })

    const onClose = vi.fn()
    render(<CardContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={onClose} />)

    fireEvent.click(screen.getByText('New session'))

    expect(onClose).toHaveBeenCalled()
    expect(mockCreateTerminal).toHaveBeenCalledWith(
      expect.objectContaining({
        agentType: 'claude',
        projectName: 'Vorn',
        projectPath: '/tmp/vorn'
      })
    )
  })

  it('quick-launch for worktree terminal creates in same worktree', async () => {
    mockCreateTerminal.mockResolvedValue({
      id: 'new-term',
      session: {
        id: 'new-term',
        agentType: 'claude',
        projectName: 'Vorn',
        projectPath: '/tmp/vorn'
      },
      status: 'idle',
      lastOutputTimestamp: Date.now()
    })

    const onClose = vi.fn()
    render(<CardContextMenu terminalId="term-2" position={{ x: 100, y: 100 }} onClose={onClose} />)

    fireEvent.click(screen.getByText('New session'))

    expect(mockCreateTerminal).toHaveBeenCalledWith(
      expect.objectContaining({
        agentType: 'claude',
        projectName: 'Vorn',
        projectPath: '/tmp/vorn',
        branch: 'feature-auth',
        existingWorktreePath: '/tmp/.vorn-worktrees/vorn/feature-auth'
      })
    )
  })

  it('renders New session and New terminal entries', () => {
    // Expand / Rename / Close used to live here but are first-class header
    // buttons now (see #264) so the ⋯ menu only carries the unique actions.
    render(<CardContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={vi.fn()} />)
    expect(screen.getByText('New session')).toBeInTheDocument()
    expect(screen.getByText('New terminal')).toBeInTheDocument()
  })

  it('calls onClose on click outside', () => {
    const onClose = vi.fn()
    render(
      <div>
        <div data-testid="outside">outside</div>
        <CardContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={onClose} />
      </div>
    )
    fireEvent.pointerDown(screen.getByTestId('outside'))
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('returns null when terminal not found', () => {
    const { container } = render(
      <CardContextMenu terminalId="nonexistent" position={{ x: 100, y: 100 }} onClose={vi.fn()} />
    )
    expect(container.innerHTML).toBe('')
  })

  it('New terminal creates shell in project context', () => {
    mockCreateShellTerminal.mockResolvedValue({
      id: 'sh-1',
      agentType: 'shell',
      projectName: 'Vorn',
      projectPath: '/tmp/vorn',
      status: 'running'
    })
    const onClose = vi.fn()
    render(<CardContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={onClose} />)
    fireEvent.click(screen.getByText('New terminal'))
    expect(onClose).toHaveBeenCalled()
    expect(mockCreateShellTerminal).toHaveBeenCalledWith('/tmp/vorn')
  })

  it('shows "New session with…" submenu with installed agents', () => {
    render(<CardContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={vi.fn()} />)
    const trigger = screen.getByText('New session with…')
    expect(trigger).toBeInTheDocument()
    fireEvent.mouseEnter(trigger.closest('button')!)
    expect(screen.getByText('Claude Code')).toBeInTheDocument()
    expect(screen.getByText('GitHub Copilot')).toBeInTheDocument()
  })

  it('"New session with…" submenu creates session with selected agent', () => {
    mockCreateTerminal.mockResolvedValue({
      id: 'new-term',
      session: {
        id: 'new-term',
        agentType: 'copilot',
        projectName: 'Vorn',
        projectPath: '/tmp/vorn'
      },
      status: 'idle',
      lastOutputTimestamp: Date.now()
    })
    const onClose = vi.fn()
    render(<CardContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={onClose} />)
    fireEvent.mouseEnter(screen.getByText('New session with…').closest('button')!)
    fireEvent.click(screen.getByText('GitHub Copilot'))
    expect(onClose).toHaveBeenCalled()
    expect(mockCreateTerminal).toHaveBeenCalledWith(
      expect.objectContaining({ agentType: 'copilot' })
    )
  })

  it('shows "Run workflow" submenu when workspace has workflows', () => {
    const configWithWorkflows = {
      ...mockConfig,
      workflows: [
        {
          id: 'wf-1',
          name: 'Deploy Staging',
          icon: 'Rocket',
          iconColor: '#ff6600',
          nodes: [
            {
              id: 'trigger-1',
              type: 'trigger',
              config: { triggerType: 'manual', contextual: true },
              position: { x: 0, y: 0 },
              label: 'Manual'
            }
          ],
          edges: [],
          enabled: true,
          workspaceId: 'personal'
        },
        {
          id: 'wf-2',
          name: 'Run Tests',
          icon: 'Play',
          iconColor: '#00ff00',
          nodes: [
            {
              id: 'trigger-2',
              type: 'trigger',
              config: { triggerType: 'manual', contextual: true },
              position: { x: 0, y: 0 },
              label: 'Manual'
            }
          ],
          edges: [],
          enabled: true,
          workspaceId: 'personal'
        }
      ]
    }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    useAppStore.setState({ config: configWithWorkflows as any })

    render(<CardContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={vi.fn()} />)
    expect(screen.getByText('Run workflow')).toBeInTheDocument()
  })

  it('does not show "Run workflow" when no workflows exist', () => {
    render(<CardContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={vi.fn()} />)
    expect(screen.queryByText('Run workflow')).not.toBeInTheDocument()
  })

  it('shows workflow names in submenu on hover and executes on click', () => {
    const configWithWorkflows = {
      ...mockConfig,
      workflows: [
        {
          id: 'wf-1',
          name: 'Deploy Staging',
          icon: 'Rocket',
          iconColor: '#ff6600',
          nodes: [
            {
              id: 'trigger-1',
              type: 'trigger',
              config: { triggerType: 'manual', contextual: true },
              position: { x: 0, y: 0 },
              label: 'Manual'
            }
          ],
          edges: [],
          enabled: true,
          workspaceId: 'personal'
        }
      ]
    }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    useAppStore.setState({ config: configWithWorkflows as any })

    const onClose = vi.fn()
    render(<CardContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={onClose} />)

    const trigger = screen.getByText('Run workflow')
    fireEvent.mouseEnter(trigger.closest('button')!)

    expect(screen.getByText('Deploy Staging')).toBeInTheDocument()

    fireEvent.click(screen.getByText('Deploy Staging'))
    expect(onClose).toHaveBeenCalled()
    expect(mockExecuteWorkflow).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'wf-1', name: 'Deploy Staging' }),
      expect.objectContaining({
        source: expect.objectContaining({ projectName: expect.any(String) })
      }),
      { source: 'manual' }
    )
  })

  it('only shows workflows from the active workspace', () => {
    const contextualTrigger = [
      {
        id: 'trigger-x',
        type: 'trigger',
        config: { triggerType: 'manual', contextual: true },
        position: { x: 0, y: 0 },
        label: 'Manual'
      }
    ]
    const configWithWorkflows = {
      ...mockConfig,
      workflows: [
        {
          id: 'wf-1',
          name: 'Personal WF',
          icon: 'Zap',
          iconColor: '#fff',
          nodes: contextualTrigger,
          edges: [],
          enabled: true,
          workspaceId: 'personal'
        },
        {
          id: 'wf-2',
          name: 'Work WF',
          icon: 'Zap',
          iconColor: '#fff',
          nodes: contextualTrigger,
          edges: [],
          enabled: true,
          workspaceId: 'work'
        }
      ]
    }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    useAppStore.setState({ config: configWithWorkflows as any, activeWorkspace: 'personal' })

    render(<CardContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={vi.fn()} />)

    const trigger = screen.getByText('Run workflow')
    fireEvent.mouseEnter(trigger.closest('button')!)

    expect(screen.getByText('Personal WF')).toBeInTheDocument()
    expect(screen.queryByText('Work WF')).not.toBeInTheDocument()
  })

  it('excludes scheduled workflows from "Run workflow" submenu', () => {
    const configWithWorkflows = {
      ...mockConfig,
      workflows: [
        {
          id: 'wf-manual',
          name: 'Manual Deploy',
          icon: 'Zap',
          iconColor: '#fff',
          nodes: [
            {
              id: 'trigger-manual',
              type: 'trigger',
              config: { triggerType: 'manual', contextual: true },
              position: { x: 0, y: 0 },
              label: 'Manual'
            }
          ],
          edges: [],
          enabled: true,
          workspaceId: 'personal'
        },
        {
          id: 'wf-scheduled',
          name: 'Nightly Build',
          icon: 'Zap',
          iconColor: '#fff',
          nodes: [
            {
              id: 'trigger-1',
              type: 'trigger',
              config: { triggerType: 'recurring', cron: '0 0 * * *' },
              position: { x: 0, y: 0 },
              label: 'Schedule'
            }
          ],
          edges: [],
          enabled: true,
          workspaceId: 'personal'
        }
      ]
    }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    useAppStore.setState({ config: configWithWorkflows as any, activeWorkspace: 'personal' })

    render(<CardContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={vi.fn()} />)

    const trigger = screen.getByText('Run workflow')
    fireEvent.mouseEnter(trigger.closest('button')!)

    expect(screen.getByText('Manual Deploy')).toBeInTheDocument()
    expect(screen.queryByText('Nightly Build')).not.toBeInTheDocument()
  })
})
