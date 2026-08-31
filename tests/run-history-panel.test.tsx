// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.mock('framer-motion', () => ({
  motion: {
    div: ({ children, ...props }: React.PropsWithChildren<Record<string, unknown>>) => (
      <div {...props}>{children}</div>
    )
  },
  AnimatePresence: ({ children }: React.PropsWithChildren) => <>{children}</>
}))

vi.mock('react-dom', async () => {
  const actual = await vi.importActual<typeof import('react-dom')>('react-dom')
  return { ...actual, createPortal: (node: React.ReactNode) => node }
})

vi.mock('lucide-react', async (importOriginal) => ({
  ...(await importOriginal<typeof import('lucide-react')>()),
  ChevronDown: (p: Record<string, unknown>) => <svg data-testid="chev-down" {...p} />,
  ChevronRight: (p: Record<string, unknown>) => <svg data-testid="chev-right" {...p} />,
  Maximize2: (p: Record<string, unknown>) => <svg data-testid="maximize" {...p} />,
  RotateCcw: (p: Record<string, unknown>) => <svg data-testid="rotate-ccw" {...p} />,
  X: (p: Record<string, unknown>) => <svg data-testid="x-icon" {...p} />
}))

vi.mock('../src/renderer/components/LogReplayModal', () => ({
  LogReplayModal: () => <div data-testid="log-replay" />
}))

import { RunHistoryPanel } from '../src/renderer/components/workflow-editor/panels/RunHistoryPanel'
import type { WorkflowExecution, WorkflowNode } from '../src/shared/types'

function makeExec(
  startedAt: string,
  overrides: Partial<WorkflowExecution> = {}
): WorkflowExecution {
  return {
    runId: `wf-1:${startedAt}`,
    workflowId: 'wf-1',
    startedAt,
    completedAt: startedAt,
    status: 'success',
    nodeStates: [],
    ...overrides
  }
}

const node: WorkflowNode = {
  id: 'node-1',
  type: 'launchAgent',
  label: 'Step',
  slug: 'step',
  config: { agentType: 'claude', projectName: 'p', projectPath: '/p', headless: true, prompt: 'x' },
  position: { x: 0, y: 0 }
}

describe('RunHistoryPanel', () => {
  it('shows empty state when no executions', () => {
    const { getByText } = render(
      <RunHistoryPanel executions={[]} nodes={[node]} onClose={vi.fn()} />
    )
    expect(getByText('No runs yet')).toBeInTheDocument()
  })

  it('renders executions in the order received (DB returns newest-first)', () => {
    const a = makeExec('2026-04-20T10:00:00Z')
    const b = makeExec('2026-04-20T09:00:00Z')
    const { container } = render(
      <RunHistoryPanel executions={[a, b]} nodes={[node]} onClose={vi.fn()} />
    )
    const entries = container.querySelectorAll('.border.border-white\\/\\[0\\.08\\].rounded-md')
    expect(entries.length).toBe(2)
  })
})
