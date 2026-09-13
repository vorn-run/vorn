import { describe, it, expect, vi } from 'vitest'
import type { SdkAction, SourceConnection } from '../packages/shared/src/types'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

const { mcpConnectionActions, mcpToolToConnectorAction } =
  await import('../packages/server/src/connectors/mcp')
const { sdkActionDef } = await import('../packages/server/src/connectors/sdk')

function connection(filters: Record<string, unknown>): SourceConnection {
  return {
    id: 'conn-1',
    connectorId: 'mcp',
    name: 'Filesystem',
    filters,
    syncIntervalMinutes: 5,
    statusMapping: {},
    createdAt: '2026-09-01T00:00:00Z'
  } as SourceConnection
}

describe('actions offered for an MCP connection', () => {
  it('keeps every tool the server offers', () => {
    const actions = mcpConnectionActions(
      connection({ discoveredTools: [{ name: 'read_file' }, { name: 'write_file' }] })
    )
    expect(actions.map((a) => a.type)).toEqual(['read_file', 'write_file'])
  })

  it('falls back to the tool name when a server offers no title', () => {
    expect(mcpToolToConnectorAction({ name: 'run_query' }).label).toBe('run_query')
    expect(mcpToolToConnectorAction({ name: 'run_query', title: '  ' }).label).toBe('run_query')
  })

  it('is empty until discovery has run', () => {
    expect(mcpConnectionActions(connection({}))).toEqual([])
  })
})

describe('actions offered for an SDK connection', () => {
  const action: SdkAction = {
    type: 'imagine',
    label: 'Imagine',
    description: 'Submit a prompt',
    inputs: [
      {
        key: 'prompt',
        label: 'Prompt',
        type: 'string',
        required: true,
        description: 'What to draw'
      },
      { key: 'count', label: 'Count', type: 'number', required: false },
      {
        key: 'aspect',
        label: 'Aspect',
        type: 'select',
        required: false,
        options: [{ value: '16:9', label: 'Wide' }, { value: '1:1' }]
      },
      { key: 'style', label: 'Style', type: 'json', required: false }
    ],
    outputs: [
      { key: 'jobId', type: 'string', description: 'The job to wait on' },
      { key: 'images', type: 'array' }
    ]
  }

  it('draws each argument as the field its type needs', () => {
    const def = sdkActionDef(action)
    expect(def).toMatchObject({ type: 'imagine', label: 'Imagine', description: 'Submit a prompt' })
    expect(def.configFields).toEqual([
      {
        key: 'prompt',
        label: 'Prompt',
        required: true,
        supportsTemplates: true,
        description: 'What to draw',
        type: 'text'
      },
      { key: 'count', label: 'Count', required: false, supportsTemplates: true, type: 'text' },
      {
        key: 'aspect',
        label: 'Aspect',
        required: false,
        supportsTemplates: true,
        type: 'select',
        options: [
          { value: '16:9', label: 'Wide' },
          { value: '1:1', label: '1:1' }
        ]
      },
      {
        key: 'style',
        label: 'Style',
        required: false,
        supportsTemplates: true,
        type: 'textarea',
        placeholder: '{} or []'
      }
    ])
  })

  it('names its outputs, so a later step can refer to them', () => {
    expect(sdkActionDef(action).outputSchema).toEqual({
      type: 'object',
      properties: {
        jobId: { type: 'string', description: 'The job to wait on' },
        images: { type: 'array' }
      }
    })
  })

  it('says nothing about outputs an action never declared', () => {
    expect(sdkActionDef({ type: 'ping', label: 'Ping' })).toEqual({
      type: 'ping',
      label: 'Ping',
      configFields: []
    })
  })
})
