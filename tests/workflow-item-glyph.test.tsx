// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { WorkflowDefinition } from '../src/shared/types'

const mockStore = {
  setEditingWorkflowId: vi.fn(),
  setWorkflowEditorOpen: vi.fn(),
  workflowExecutions: new Map<string, unknown>(),
  editingWorkflowId: null as string | null
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) =>
    selector ? selector(mockStore) : mockStore
}))
vi.mock('../src/renderer/lib/workflow-execution', () => ({ executeWorkflow: vi.fn() }))

const { WorkflowItem } = await import('../src/renderer/components/project-sidebar/WorkflowItem')
const { __resetConnectionsCacheForTests } = await import('../src/renderer/lib/use-connections')

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

/** A workflow the connector itself seeded, whose id names the connection. */
function seeded(): WorkflowDefinition {
  return {
    id: 'connector:conn-1:tick',
    name: 'Ticks to tasks',
    icon: 'Zap',
    iconColor: '#ffffff',
    enabled: true,
    nodes: [
      {
        id: 't',
        type: 'trigger',
        label: 'T',
        config: { triggerType: 'connectorPoll', connectionId: 'conn-1', event: 'tick' },
        position: { x: 0, y: 0 }
      }
    ],
    edges: []
  } as unknown as WorkflowDefinition
}

describe('a connector-seeded row in the sidebar', () => {
  it('draws the mark the connector ships rather than a generic plug', async () => {
    const { container } = render(
      <WorkflowItem workflow={seeded()} isCollapsed={false} iconSize={14} onContextMenu={vi.fn()} />
    )

    await waitFor(() => {
      expect(container.querySelector(`path[d="${PACK_PATH}"]`)).not.toBeNull()
    })
  })
})
