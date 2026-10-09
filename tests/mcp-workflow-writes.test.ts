import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type { AppConfig, WorkflowDefinition } from '../packages/shared/src/types'

/**
 * The MCP workflow tools write through workflow:create, workflow:update and
 * workflow:delete, so the server stores the change and arms its schedule, and
 * never rewrite the whole configuration through config:save.
 */

const calls: Array<{ method: string; params?: unknown }> = []
let workflows: WorkflowDefinition[]

vi.mock('../packages/server/src/rpc-client', () => ({
  rpcCall: async (method: string, params?: unknown) => {
    calls.push({ method, params })
    const id = (params as { id?: string } | undefined)?.id
    switch (method) {
      case 'config:load':
        return { version: 1, revision: 3, workflows } as unknown as AppConfig
      case 'workflow:create':
        return (params as { workflow: WorkflowDefinition }).workflow
      case 'workflow:update':
      case 'workflow:delete':
        return { ok: workflows.some((w) => w.id === id) }
      default:
        throw new Error(`unexpected ${method}`)
    }
  }
}))

const { registerWorkflowTools } = await import('../packages/mcp/src/tools/workflows')
const { dbUpdateWorkflow, dbDeleteWorkflow } = await import('../packages/mcp/src/data-access')

type Handler = (args: Record<string, unknown>) => Promise<{
  content: Array<{ type: string; text: string }>
  isError?: boolean
}>

function tools(): Map<string, Handler> {
  const out = new Map<string, Handler>()
  const server = {
    tool: (name: string, _desc: string, schemaOrHandler: unknown, maybeHandler?: unknown) => {
      out.set(name, (maybeHandler ?? schemaOrHandler) as Handler)
    }
  } as unknown as McpServer
  registerWorkflowTools(server)
  return out
}

const stored: WorkflowDefinition = {
  id: 'wf-1',
  name: 'Nightly',
  icon: 'Zap',
  iconColor: '#6366f1',
  nodes: [],
  edges: [],
  enabled: true
}

const writes = () => calls.filter((c) => c.method !== 'config:load')

beforeEach(() => {
  calls.length = 0
  workflows = [stored]
})

describe('workflow tools', () => {
  it('create_workflow stores the new workflow through workflow:create', async () => {
    const result = await tools().get('create_workflow')!({ name: 'Morning' })

    expect(result.isError).toBeUndefined()
    expect(writes()).toHaveLength(1)
    expect(writes()[0].method).toBe('workflow:create')
    const sent = (writes()[0].params as { workflow: WorkflowDefinition }).workflow
    expect(sent.name).toBe('Morning')
    expect(JSON.parse(result.content[0].text).id).toBe(sent.id)
  })

  it('update_workflow sends only the changed fields through workflow:update', async () => {
    const result = await tools().get('update_workflow')!({
      workflow_id: 'wf-1',
      name: 'Nightly 2',
      enabled: false
    })

    expect(result.isError).toBeUndefined()
    expect(writes()).toEqual([
      {
        method: 'workflow:update',
        params: { id: 'wf-1', updates: { name: 'Nightly 2', enabled: false } }
      }
    ])
  })

  it('delete_workflow removes it through workflow:delete', async () => {
    const result = await tools().get('delete_workflow')!({ workflow_id: 'wf-1' })

    expect(result.content[0].text).toBe('Deleted workflow: Nightly')
    expect(writes()).toEqual([{ method: 'workflow:delete', params: { id: 'wf-1' } }])
  })

  it('never rewrites the whole configuration', async () => {
    const all = tools()
    await all.get('create_workflow')!({ name: 'Morning' })
    await all.get('update_workflow')!({ workflow_id: 'wf-1', name: 'Renamed' })
    await all.get('delete_workflow')!({ workflow_id: 'wf-1' })

    expect(calls.some((c) => c.method === 'config:save')).toBe(false)
  })
})

describe('a workflow gone by the time it is written', () => {
  it('fails the update instead of reporting it saved', async () => {
    workflows = []
    await expect(dbUpdateWorkflow('wf-1', { name: 'x' })).rejects.toThrow(
      'workflow "wf-1" not found'
    )
  })

  it('fails the delete the same way', async () => {
    workflows = []
    await expect(dbDeleteWorkflow('wf-1')).rejects.toThrow('workflow "wf-1" not found')
  })
})
