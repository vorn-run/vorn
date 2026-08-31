// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.mock('../src/renderer/lib/use-connections', () => ({
  useConnections: () => [],
  useConnectorIdFor: () => null,
  useConnectionIconFor: () => undefined
}))
vi.mock('../src/renderer/lib/workflow-execution', () => ({
  isRunStoppable: () => false,
  stopWorkflowRun: vi.fn(),
  approveWorkflowGate: vi.fn(),
  rejectWorkflowGate: vi.fn()
}))
vi.mock('../src/renderer/components/Tooltip', () => ({
  Tooltip: ({ children }: React.PropsWithChildren) => <>{children}</>
}))

import { RunEntry } from '../src/renderer/components/workflow-editor/RunEntry'
import type { WorkflowExecution, WorkflowNode } from '../packages/shared/src/types'

const nodes = [
  { id: 't', type: 'trigger', label: 'Manual', position: { x: 0, y: 0 }, config: {} },
  {
    id: 'a',
    type: 'launchAgent',
    label: 'Do the work',
    slug: 'do_the_work',
    position: { x: 0, y: 0 },
    config: { agentType: 'claude', projectName: '', projectPath: '', headless: true }
  }
] as unknown as WorkflowNode[]

const running: WorkflowExecution = {
  runId: 'r1',
  workflowId: 'wf',
  startedAt: new Date().toISOString(),
  status: 'running',
  nodeStates: [
    { nodeId: 't', status: 'success' },
    { nodeId: 'a', status: 'running' }
  ]
} as WorkflowExecution

describe('a followed run', () => {
  it('starts expanded with its live step timeline open', () => {
    const { getByText } = render(<RunEntry execution={running} nodes={nodes} follow />)
    // Expanded without a click, down to the running step's own detail.
    expect(getByText('Do the work')).toBeInTheDocument()
    expect(getByText(/No output captured yet/)).toBeInTheDocument()
  })

  it('stays collapsed when not followed', () => {
    const { queryByText } = render(<RunEntry execution={running} nodes={nodes} />)
    expect(queryByText('Do the work')).toBeNull()
  })
})
