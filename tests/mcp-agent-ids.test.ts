import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { getRecentSessionActivityLabel } from '../packages/shared/src/types'

/** What vornd answers, and the methods asked; a plain function, so a refusal is just a rejection. */
let answer: (method: string) => Promise<unknown> = async () => null
const asked: string[] = []
vi.mock('../packages/mcp/src/rpc-client', () => ({
  rpcCall: (method: string) => {
    asked.push(method)
    return answer(method)
  }
}))

const { agentsOff } = await import('../packages/mcp/src/tools/agents')
const { registerSessionTools } = await import('../packages/mcp/src/tools/sessions')

type Handler = (args: Record<string, unknown>) => Promise<{
  content: Array<{ type: string; text: string }>
  isError?: boolean
}>

function sessionTools(): Map<string, Handler> {
  const tools = new Map<string, Handler>()
  const server = {
    tool: (name: string, _desc: string, schemaOrHandler: unknown, maybeHandler?: unknown) => {
      tools.set(name, (maybeHandler ?? schemaOrHandler) as Handler)
    }
  } as unknown as McpServer
  registerSessionTools(server)
  return tools
}

/** `agent:list` answers these as on; any other call answers null. */
function listing(on: unknown): void {
  answer = async (method) => (method === 'agent:list' ? on : null)
}

describe('agent ids in the MCP tools', () => {
  beforeEach(() => {
    asked.length = 0
  })

  it('lets an agent that is on through, and asks nothing for none', async () => {
    listing([{ id: 'acme' }, { id: 'beta' }])
    expect(await agentsOff(['beta'])).toBeNull()
    asked.length = 0
    expect(await agentsOff([])).toBeNull()
    expect(asked).toEqual([])
  })

  it('names the agents that are on when one is not', async () => {
    listing([{ id: 'acme' }, null, { id: 7 }, { id: 'beta' }])
    expect((await agentsOff(['acme', 'gamma']))?.content[0].text).toBe(
      'Error: agent "gamma" is not on in Vorn. Agents that are on: acme, beta.'
    )
  })

  it('says where to switch one on when none is', async () => {
    listing([])
    expect((await agentsOff(['acme']))?.content[0].text).toBe(
      'Error: agent "acme" is not on in Vorn. No agent is on; switch one on in Settings › Extensions.'
    )
  })

  it('leaves the decision to vornd when it cannot list its agents', async () => {
    answer = async () => {
      throw new Error('older server')
    }
    expect(await agentsOff(['acme'])).toBeNull()
  })

  it('launch_session refuses an agent that is off before creating anything', async () => {
    listing([{ id: 'acme' }])
    const launch = sessionTools().get('launch_session')!
    const res = await launch({ agent_type: 'beta', project_name: 'p', project_path: '/p' })
    expect(res.isError).toBe(true)
    expect(res.content[0].text).toContain('agent "beta" is not on')
    expect(asked).toEqual(['agent:list'])
  })

  it('labels an unknown agent’s recent activity as a session', () => {
    expect(getRecentSessionActivityLabel('acme')).toBe('session')
  })
})
