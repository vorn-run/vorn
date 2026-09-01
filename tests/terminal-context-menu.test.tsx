// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const mockGetTerminalSelection = vi.fn()
const mockClearTerminalSelection = vi.fn()
const mockPasteToTerminal = vi.fn()
const mockFocusTerminal = vi.fn()

vi.mock('../src/renderer/lib/terminal-registry', () => ({
  getTerminalSelection: (...args: unknown[]) => mockGetTerminalSelection(...args),
  clearTerminalSelection: (...args: unknown[]) => mockClearTerminalSelection(...args),
  pasteToTerminal: (...args: unknown[]) => mockPasteToTerminal(...args),
  focusTerminal: (...args: unknown[]) => mockFocusTerminal(...args)
}))

const mockExecuteWorkflow = vi.fn()
vi.mock('../src/renderer/lib/workflow-execution', () => ({
  executeWorkflow: (...args: unknown[]) => mockExecuteWorkflow(...args)
}))

import { useAppStore } from '../src/renderer/stores'
import { TerminalContextMenu } from '../src/renderer/components/TerminalContextMenu'

const mockConfig = {
  projects: [],
  workflows: [],
  defaults: { defaultAgent: 'claude' as const, rowHeight: 208 },
  remoteHosts: [],
  workspaces: []
}

beforeEach(() => {
  vi.clearAllMocks()
  mockGetTerminalSelection.mockReturnValue('')

  Object.defineProperty(navigator, 'clipboard', {
    value: {
      writeText: vi.fn().mockResolvedValue(undefined),
      readText: vi.fn().mockResolvedValue('')
    },
    writable: true,
    configurable: true
  })

  useAppStore.setState({
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    config: mockConfig as any,
    activeWorkspace: 'personal'
  })
})

describe('TerminalContextMenu', () => {
  it('renders Copy and Paste buttons', () => {
    render(
      <TerminalContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={vi.fn()} />
    )
    expect(screen.getByText('Copy')).toBeInTheDocument()
    expect(screen.getByText('Paste')).toBeInTheDocument()
  })

  it('Copy is disabled when no selection', () => {
    mockGetTerminalSelection.mockReturnValue('')
    render(
      <TerminalContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={vi.fn()} />
    )
    expect(screen.getByText('Copy').closest('button')).toBeDisabled()
  })

  it('Copy is enabled when there is a selection', () => {
    mockGetTerminalSelection.mockReturnValue('some text')
    render(
      <TerminalContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={vi.fn()} />
    )
    expect(screen.getByText('Copy').closest('button')).not.toBeDisabled()
  })

  it('calls onClose and focusTerminal on Copy click', () => {
    mockGetTerminalSelection.mockReturnValue('some text')
    const onClose = vi.fn()
    render(
      <TerminalContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={onClose} />
    )
    fireEvent.click(screen.getByText('Copy'))
    expect(onClose).toHaveBeenCalled()
    expect(mockFocusTerminal).toHaveBeenCalledWith('term-1')
  })

  it('calls onClose and focusTerminal on Paste click', () => {
    const onClose = vi.fn()
    render(
      <TerminalContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={onClose} />
    )
    fireEvent.click(screen.getByText('Paste'))
    expect(onClose).toHaveBeenCalled()
    expect(mockFocusTerminal).toHaveBeenCalledWith('term-1')
  })

  it('does not show "Run workflow" when no workflows exist', () => {
    render(
      <TerminalContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={vi.fn()} />
    )
    expect(screen.queryByText('Run workflow')).not.toBeInTheDocument()
  })

  it('shows "Run workflow" when workspace has workflows', () => {
    const configWithWorkflows = {
      ...mockConfig,
      workflows: [
        {
          id: 'wf-1',
          name: 'Deploy',
          icon: 'Rocket',
          iconColor: '#ff6600',
          nodes: [
            {
              id: 'trigger',
              type: 'trigger',
              label: 'Manual',
              config: { triggerType: 'manual' },
              position: { x: 0, y: 0 }
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

    render(
      <TerminalContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={vi.fn()} />
    )
    expect(screen.getByText('Run workflow')).toBeInTheDocument()
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
              id: 'trigger',
              type: 'trigger',
              label: 'Manual',
              config: { triggerType: 'manual' },
              position: { x: 0, y: 0 }
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
    render(
      <TerminalContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={onClose} />
    )

    const trigger = screen.getByText('Run workflow')
    fireEvent.mouseEnter(trigger.closest('button')!)

    expect(screen.getByText('Deploy Staging')).toBeInTheDocument()

    fireEvent.click(screen.getByText('Deploy Staging'))
    expect(onClose).toHaveBeenCalled()
    expect(mockFocusTerminal).toHaveBeenCalledWith('term-1')
    expect(mockExecuteWorkflow).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'wf-1', name: 'Deploy Staging' }),
      undefined,
      { source: 'manual' }
    )
  })

  it('toggles workflow submenu on click', () => {
    const configWithWorkflows = {
      ...mockConfig,
      workflows: [
        {
          id: 'wf-1',
          name: 'Deploy',
          icon: 'Rocket',
          iconColor: '#ff6600',
          nodes: [
            {
              id: 'trigger',
              type: 'trigger',
              label: 'Manual',
              config: { triggerType: 'manual' },
              position: { x: 0, y: 0 }
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

    render(
      <TerminalContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={vi.fn()} />
    )

    const trigger = screen.getByText('Run workflow').closest('button')!
    // Click to open
    fireEvent.click(trigger)
    expect(screen.getByText('Deploy')).toBeInTheDocument()
    // Click again to close
    fireEvent.click(trigger)
    expect(screen.queryByText('Deploy')).not.toBeInTheDocument()
  })

  it('hides workflow submenu on mouse leave after delay', async () => {
    vi.useFakeTimers()
    const configWithWorkflows = {
      ...mockConfig,
      workflows: [
        {
          id: 'wf-1',
          name: 'Deploy',
          icon: 'Rocket',
          iconColor: '#ff6600',
          nodes: [
            {
              id: 'trigger',
              type: 'trigger',
              label: 'Manual',
              config: { triggerType: 'manual' },
              position: { x: 0, y: 0 }
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

    const { unmount } = render(
      <TerminalContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={vi.fn()} />
    )

    const trigger = screen.getByText('Run workflow').closest('button')!
    fireEvent.mouseEnter(trigger)
    expect(screen.getByText('Deploy')).toBeInTheDocument()

    fireEvent.mouseLeave(trigger)
    await act(() => {
      vi.advanceTimersByTime(200)
    })
    expect(screen.queryByText('Deploy')).not.toBeInTheDocument()
    unmount()
    vi.useRealTimers()
  })

  it('only shows workflows from the active workspace', () => {
    const configWithWorkflows = {
      ...mockConfig,
      workflows: [
        {
          id: 'wf-1',
          name: 'Personal WF',
          icon: 'Zap',
          iconColor: '#fff',
          nodes: [
            {
              id: 'trigger',
              type: 'trigger',
              label: 'Manual',
              config: { triggerType: 'manual' },
              position: { x: 0, y: 0 }
            }
          ],
          edges: [],
          enabled: true,
          workspaceId: 'personal'
        },
        {
          id: 'wf-2',
          name: 'Work WF',
          icon: 'Zap',
          iconColor: '#fff',
          nodes: [
            {
              id: 'trigger',
              type: 'trigger',
              label: 'Manual',
              config: { triggerType: 'manual' },
              position: { x: 0, y: 0 }
            }
          ],
          edges: [],
          enabled: true,
          workspaceId: 'work'
        }
      ]
    }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    useAppStore.setState({ config: configWithWorkflows as any, activeWorkspace: 'personal' })

    render(
      <TerminalContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={vi.fn()} />
    )

    const trigger = screen.getByText('Run workflow')
    fireEvent.mouseEnter(trigger.closest('button')!)

    expect(screen.getByText('Personal WF')).toBeInTheDocument()
    expect(screen.queryByText('Work WF')).not.toBeInTheDocument()
  })

  it('closes menu on Escape key', () => {
    const onClose = vi.fn()
    render(
      <TerminalContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={onClose} />
    )
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('closes menu on click outside', () => {
    const onClose = vi.fn()
    render(
      <div>
        <div data-testid="outside">outside</div>
        <TerminalContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={onClose} />
      </div>
    )
    fireEvent.pointerDown(screen.getByTestId('outside'))
    expect(onClose).toHaveBeenCalledTimes(1)
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
              id: 'trigger',
              type: 'trigger',
              label: 'Manual',
              config: { triggerType: 'manual' },
              position: { x: 0, y: 0 }
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

    render(
      <TerminalContextMenu terminalId="term-1" position={{ x: 100, y: 100 }} onClose={vi.fn()} />
    )

    const trigger = screen.getByText('Run workflow')
    fireEvent.mouseEnter(trigger.closest('button')!)

    expect(screen.getByText('Manual Deploy')).toBeInTheDocument()
    expect(screen.queryByText('Nightly Build')).not.toBeInTheDocument()
  })
})
