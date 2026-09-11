// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
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
  listConnectorPacks.mockReset().mockResolvedValue([])
  ;(window as unknown as { api: unknown }).api = {
    listConnections,
    listConnectorPacks,
    onConfigChanged: () => () => {}
  }
})

const { RunsList } = await import('../src/renderer/components/workflow-runs/RunsList')

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
