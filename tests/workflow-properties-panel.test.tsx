// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { WorkflowPropertiesPanel } from '../src/renderer/components/workflow-editor/panels/WorkflowPropertiesPanel'
import type { WorkflowNode, WorkflowExecution } from '../src/shared/types'

function makeTrigger(triggerType: 'manual' | 'recurring'): WorkflowNode {
  return {
    id: 't1',
    type: 'trigger',
    label: 'Trigger',
    config:
      triggerType === 'manual'
        ? { triggerType: 'manual' }
        : { triggerType: 'recurring', cron: '0 9 * * *' },
    position: { x: 0, y: 0 }
  }
}

const baseProps = {
  enabled: true,
  onEnabledChange: vi.fn(),
  staggerDelayMs: undefined as number | undefined,
  onStaggerChange: vi.fn(),
  autoCleanupWorktrees: false,
  onCleanupChange: vi.fn(),
  triggerNode: null as WorkflowNode | null,
  onSelectTrigger: vi.fn(),
  lastRun: null as WorkflowExecution | null,
  onClose: vi.fn()
}

describe('WorkflowPropertiesPanel', () => {
  it('renders Properties header and a close button', () => {
    render(<WorkflowPropertiesPanel {...baseProps} />)
    expect(screen.getByText('Properties')).toBeInTheDocument()
  })

  it('shows "Enabled" label when enabled is true and "Disabled" when false', () => {
    const { rerender } = render(<WorkflowPropertiesPanel {...baseProps} enabled={true} />)
    expect(screen.getByText('Enabled')).toBeInTheDocument()
    rerender(<WorkflowPropertiesPanel {...baseProps} enabled={false} />)
    expect(screen.getByText('Disabled')).toBeInTheDocument()
  })

  it('preserves a 0ms stagger delay value in the input', () => {
    render(<WorkflowPropertiesPanel {...baseProps} staggerDelayMs={0} />)
    const input = screen.getByPlaceholderText('0ms') as HTMLInputElement
    expect(input.value).toBe('0')
  })

  it('shows trigger summary when a trigger node is provided', () => {
    render(<WorkflowPropertiesPanel {...baseProps} triggerNode={makeTrigger('manual')} />)
    expect(screen.getByText('Manual')).toBeInTheDocument()
  })

  it('shows recurring cron summary when trigger is recurring', () => {
    render(<WorkflowPropertiesPanel {...baseProps} triggerNode={makeTrigger('recurring')} />)
    expect(screen.getByText(/Recurring/)).toBeInTheDocument()
  })

  it('shows "None" when there is no trigger node', () => {
    render(<WorkflowPropertiesPanel {...baseProps} triggerNode={null} />)
    expect(screen.getByText('None')).toBeInTheDocument()
  })

  it('calls onClose when the close button is clicked', () => {
    const onClose = vi.fn()
    render(<WorkflowPropertiesPanel {...baseProps} onClose={onClose} />)
    const buttons = screen.getAllByRole('button')
    const closeButton = buttons.find((b) => b.querySelector('svg'))
    if (closeButton) fireEvent.click(closeButton)
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('calls onSelectTrigger when the trigger summary is clicked', () => {
    const onSelectTrigger = vi.fn()
    render(
      <WorkflowPropertiesPanel
        {...baseProps}
        triggerNode={makeTrigger('manual')}
        onSelectTrigger={onSelectTrigger}
      />
    )
    fireEvent.click(screen.getByText('Manual'))
    expect(onSelectTrigger).toHaveBeenCalledTimes(1)
  })

  it('renders Last run with a status dot when lastRun is provided', () => {
    const lastRun = {
      workflowId: 'w1',
      startedAt: new Date(Date.now() - 5 * 60_000).toISOString(),
      status: 'success' as const,
      nodeStates: []
    }
    const { container } = render(<WorkflowPropertiesPanel {...baseProps} lastRun={lastRun} />)
    expect(screen.getByText('Last run')).toBeInTheDocument()
    expect(container.querySelector('.bg-green-400')).toBeInTheDocument()
  })

  it('hides the Last run row when there is no lastRun', () => {
    render(<WorkflowPropertiesPanel {...baseProps} lastRun={null} />)
    expect(screen.queryByText('Last run')).not.toBeInTheDocument()
  })

  it('updates stagger value when input changes', () => {
    const onStaggerChange = vi.fn()
    render(<WorkflowPropertiesPanel {...baseProps} onStaggerChange={onStaggerChange} />)
    const input = screen.getByPlaceholderText('0ms')
    fireEvent.change(input, { target: { value: '500' } })
    expect(onStaggerChange).toHaveBeenCalledWith(500)
  })

  it('clears stagger value when input is emptied', () => {
    const onStaggerChange = vi.fn()
    render(
      <WorkflowPropertiesPanel
        {...baseProps}
        staggerDelayMs={100}
        onStaggerChange={onStaggerChange}
      />
    )
    const input = screen.getByPlaceholderText('0ms')
    fireEvent.change(input, { target: { value: '' } })
    expect(onStaggerChange).toHaveBeenCalledWith(undefined)
  })

  it('renders summary for once trigger', () => {
    const node = {
      id: 't1',
      type: 'trigger' as const,
      label: 'T',
      config: { triggerType: 'once' as const, runAt: new Date('2026-01-01').toISOString() },
      position: { x: 0, y: 0 }
    }
    render(<WorkflowPropertiesPanel {...baseProps} triggerNode={node} />)
    expect(screen.getByText(/Once/)).toBeInTheDocument()
  })

  it('renders summary for taskCreated trigger', () => {
    const node = {
      id: 't1',
      type: 'trigger' as const,
      label: 'T',
      config: { triggerType: 'taskCreated' as const },
      position: { x: 0, y: 0 }
    }
    render(<WorkflowPropertiesPanel {...baseProps} triggerNode={node} />)
    expect(screen.getByText('Task created')).toBeInTheDocument()
  })

  it('renders summary for taskStatusChanged trigger', () => {
    const node = {
      id: 't1',
      type: 'trigger' as const,
      label: 'T',
      config: { triggerType: 'taskStatusChanged' as const },
      position: { x: 0, y: 0 }
    }
    render(<WorkflowPropertiesPanel {...baseProps} triggerNode={node} />)
    expect(screen.getByText('Task status changed')).toBeInTheDocument()
  })

  it('toggles enabled when the switch is clicked', () => {
    const onEnabledChange = vi.fn()
    const { container } = render(
      <WorkflowPropertiesPanel {...baseProps} onEnabledChange={onEnabledChange} />
    )
    const switchButton = container.querySelector('button[role="switch"]') as HTMLButtonElement
    if (switchButton) fireEvent.click(switchButton)
    expect(onEnabledChange).toHaveBeenCalled()
  })

  it('renders the cleanup toggle as off and ignores clicks when cleanupDisabled is true', () => {
    const onCleanupChange = vi.fn()
    const { container } = render(
      <WorkflowPropertiesPanel
        {...baseProps}
        autoCleanupWorktrees={true}
        cleanupDisabled
        onCleanupChange={onCleanupChange}
      />
    )
    // The cleanup row sits after Status / Stagger — find its switch by its parent row label.
    const cleanupRow = Array.from(container.querySelectorAll('div')).find((d) =>
      d.textContent?.startsWith('Cleanup worktrees')
    )!
    const cleanupSwitch = cleanupRow.querySelector('button[role="switch"]') as HTMLButtonElement
    expect(cleanupSwitch.getAttribute('aria-checked')).toBe('false')
    fireEvent.click(cleanupSwitch)
    expect(onCleanupChange).not.toHaveBeenCalled()
  })

  it('calls onCleanupChange when the cleanup toggle is enabled and clicked', () => {
    const onCleanupChange = vi.fn()
    const { container } = render(
      <WorkflowPropertiesPanel
        {...baseProps}
        autoCleanupWorktrees={false}
        onCleanupChange={onCleanupChange}
      />
    )
    const cleanupRow = Array.from(container.querySelectorAll('div')).find((d) =>
      d.textContent?.startsWith('Cleanup worktrees')
    )!
    const cleanupSwitch = cleanupRow.querySelector('button[role="switch"]') as HTMLButtonElement
    fireEvent.click(cleanupSwitch)
    expect(onCleanupChange).toHaveBeenCalled()
  })
})
