// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, fireEvent, cleanup, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { WorkflowExecution } from '../src/shared/types'

const stopWorkflowRun = vi.hoisted(() => vi.fn())
vi.mock('../src/renderer/lib/workflow-execution', () => ({
  stopWorkflowRun,
  isRunStoppable: (e: WorkflowExecution) => e.status === 'running'
}))

const toastError = vi.hoisted(() => vi.fn())
vi.mock('../src/renderer/components/Toast', () => ({ toast: { error: toastError } }))

vi.mock('../src/renderer/components/Tooltip', () => ({
  Tooltip: ({ children }: React.PropsWithChildren) => <>{children}</>
}))

import { StopRunButton } from '../src/renderer/components/workflow-runs/StopRunButton'

/**
 * Ending a run that is still going. A run with no way to stop it is half of
 * what turned one wedged step into a permanently blocked workflow.
 */

function run(status: WorkflowExecution['status']): WorkflowExecution {
  return {
    runId: 'run-1',
    workflowId: 'wf-1',
    startedAt: new Date().toISOString(),
    status,
    nodeStates: []
  } as WorkflowExecution
}

beforeEach(() => {
  stopWorkflowRun.mockReset()
  stopWorkflowRun.mockResolvedValue(undefined)
  toastError.mockReset()
})
afterEach(() => cleanup())

describe('StopRunButton', () => {
  it('stops the run it was given', async () => {
    render(<StopRunButton execution={run('running')} />)
    fireEvent.click(screen.getByLabelText('Stop run'))
    await waitFor(() => expect(stopWorkflowRun).toHaveBeenCalledWith('run-1'))
  })

  it.each([['success'], ['error'], ['cancelled']] as const)(
    'renders nothing for a %s run',
    (status) => {
      // Nothing left to stop, so the control should not be offered.
      const { container } = render(<StopRunButton execution={run(status)} />)
      expect(container).toBeEmptyDOMElement()
    }
  )

  it('ignores a second click while the first is still stopping', async () => {
    let release: () => void = () => {}
    stopWorkflowRun.mockImplementation(() => new Promise<void>((r) => (release = r)))
    render(<StopRunButton execution={run('running')} />)
    const button = screen.getByLabelText('Stop run')
    fireEvent.click(button)
    fireEvent.click(button)
    expect(stopWorkflowRun).toHaveBeenCalledTimes(1)
    release()
  })

  it('keeps the click off the row it sits in', () => {
    // The button lives inside a clickable run row, which would otherwise open
    // the run at the same time as stopping it.
    const onRowClick = vi.fn()
    render(
      <div onClick={onRowClick}>
        <StopRunButton execution={run('running')} />
      </div>
    )
    fireEvent.click(screen.getByLabelText('Stop run'))
    expect(onRowClick).not.toHaveBeenCalled()
  })

  it('says so when stopping fails', async () => {
    // An async click handler that throws is an unhandled rejection, and the run
    // just appears not to stop.
    stopWorkflowRun.mockRejectedValue(new Error('core unreachable'))
    render(<StopRunButton execution={run('running')} />)
    fireEvent.click(screen.getByLabelText('Stop run'))
    await waitFor(() => expect(toastError).toHaveBeenCalled())
    expect(screen.getByLabelText('Stop run')).not.toBeDisabled()
  })
})
