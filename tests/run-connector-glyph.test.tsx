// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { WorkflowExecution, WorkflowNode } from '../src/shared/types'
import type { RunListEntry } from '../src/renderer/hooks/useAllWorkflowRuns'
import { __resetConnectionsCacheForTests } from '../src/renderer/lib/use-connections'

const mockState = {
  setMainViewMode: vi.fn(),
  setEditingWorkflowId: vi.fn(),
  config: { workflows: [] }
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) =>
    selector ? selector(mockState) : mockState
}))

/** The mark the pack ships, which no built-in lookup can produce. */
const PACK_PATH = 'M2 2h9v9z'

const listConnections = vi.fn()
const listConnectorPacks = vi.fn()

beforeEach(() => {
  __resetConnectionsCacheForTests()
  listConnections.mockReset().mockResolvedValue([
    {
      id: 'conn-1',
      connectorId: 'mcp',
      name: 'Pack Demo',
      filters: { sdkConnectorId: 'packdemo' }
    }
  ])
  listConnectorPacks.mockReset().mockResolvedValue([
    {
      id: 'packdemo',
      name: 'Pack Demo',
      version: '1.0.0',
      icon: { viewBox: '0 0 24 24', paths: [PACK_PATH] }
    }
  ])
  ;(window as unknown as { api: unknown }).api = {
    listConnections,
    listConnectorPacks,
    onConfigChanged: () => () => {}
  }
})

const { RunsList } = await import('../src/renderer/components/workflow-runs/RunsList')
const { RunStepsList } = await import('../src/renderer/components/workflow-editor/RunEntry')

function packagedRun(overrides: Partial<RunListEntry> = {}): RunListEntry {
  return {
    runId: 'run-1',
    workflowId: 'wf-1',
    startedAt: '2026-09-02T11:59:00Z',
    completedAt: '2026-09-02T12:00:00Z',
    status: 'success',
    nodeStates: [{ nodeId: 'n1', status: 'success' }],
    connectorItem: {
      connectionId: 'conn-1',
      // A packaged connector only ever knows itself as `mcp` here.
      connectorId: 'mcp',
      externalId: '7',
      title: 'Tick 7',
      raw: {}
    },
    ...overrides
  } as RunListEntry
}

describe('All runs', () => {
  it('draws a packaged connector with the glyph its pack ships', async () => {
    const { container } = render(
      <RunsList
        runs={[packagedRun()]}
        workflowsById={new Map([['wf-1', { name: 'Ticks', nodes: [] }]])}
        filter="all"
        selectedId={null}
        onSelect={() => {}}
      />
    )

    await waitFor(() => {
      expect(container.querySelector(`path[d="${PACK_PATH}"]`)).not.toBeNull()
    })
  })

  it('names the connector it really is rather than the mcp it is stored as', async () => {
    const { getByText } = render(
      <RunsList
        runs={[packagedRun()]}
        workflowsById={new Map([['wf-1', { name: 'Ticks', nodes: [] }]])}
        filter="all"
        selectedId={null}
        onSelect={() => {}}
      />
    )

    await waitFor(() => expect(getByText('packdemo 7')).toBeInTheDocument())
  })
})

describe('a run trace', () => {
  const nodes: WorkflowNode[] = [
    {
      id: 'n1',
      type: 'callConnectorAction',
      label: 'Echo',
      config: { connectionId: 'conn-1', action: 'echo' },
      position: { x: 0, y: 0 }
    } as WorkflowNode
  ]

  it('draws each connector step with its own glyph', async () => {
    const execution = {
      runId: 'run-1',
      workflowId: 'wf-1',
      startedAt: '2026-09-02T11:59:00Z',
      status: 'success',
      nodeStates: [{ nodeId: 'n1', status: 'success' }]
    } as WorkflowExecution

    const { container } = render(<RunStepsList execution={execution} nodes={nodes} />)

    await waitFor(() => {
      expect(container.querySelector(`path[d="${PACK_PATH}"]`)).not.toBeNull()
    })
  })
})
