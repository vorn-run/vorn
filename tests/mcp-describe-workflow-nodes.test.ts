import { describe, expect, it } from 'vitest'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { loopStructureError } from '@vornrun/shared/workflow-graph'
import type { WorkflowEdge, WorkflowNode } from '../packages/shared/src/types'
import { registerDescribeNodesTool } from '../packages/mcp/src/tools/describe-nodes'
import {
  edgeSchema,
  nodeSchema,
  validateGraph,
  validateLoopBodies
} from '../packages/mcp/src/tools/workflows'

type Handler = (args: Record<string, unknown>) => Promise<{
  content: Array<{ type: string; text: string }>
  isError?: boolean
}>

/** Collects the tools the module registers, so a handler can be called without a transport. */
function collect(): Map<string, Handler> {
  const tools = new Map<string, Handler>()
  const server = {
    tool: (name: string, _desc: string, schemaOrHandler: unknown, maybeHandler?: unknown) => {
      tools.set(name, (maybeHandler ?? schemaOrHandler) as Handler)
    }
  } as unknown as McpServer
  registerDescribeNodesTool(server)
  return tools
}

async function describeNodes(args: Record<string, unknown> = {}) {
  const handler = collect().get('describe_workflow_nodes')!
  const result = await handler(args)
  return JSON.parse(result.content[0].text) as {
    nodeTypes: Record<
      string,
      { summary: string; configSchema: { type?: string }; outputs: unknown[]; rules: string[] }
    >
    edgeSchema: unknown
    limits: Record<string, unknown>
    templates: { namespaces: Record<string, string> }
    example: { nodes: unknown[]; edges: unknown[] }
  }
}

const ALL = [
  'trigger',
  'launchAgent',
  'script',
  'condition',
  'approval',
  'createTaskFromItem',
  'callConnectorAction',
  'httpRequest',
  'loop'
]

describe('describe_workflow_nodes', () => {
  it('describes all nine node types', async () => {
    const reference = await describeNodes()
    expect(Object.keys(reference.nodeTypes).sort()).toEqual([...ALL].sort())
    for (const type of ALL) {
      expect(reference.nodeTypes[type].summary).toBeTruthy()
      expect(reference.nodeTypes[type].rules.length).toBeGreaterThan(0)
    }
  })

  it('gives each type a config schema that is an object', async () => {
    const reference = await describeNodes()
    for (const type of ALL) {
      const schema = reference.nodeTypes[type].configSchema as {
        type?: string
        anyOf?: unknown[]
        oneOf?: unknown[]
      }
      // The trigger is a union of objects, one per triggerType.
      if (type === 'trigger') expect(schema.oneOf ?? schema.anyOf).toBeTruthy()
      else expect(schema.type).toBe('object')
    }
  })

  it('names the loop and gate outputs a later step reads', async () => {
    const { nodeTypes } = await describeNodes()
    const fields = (type: string) =>
      (nodeTypes[type].outputs as { field: string }[]).map((o) => o.field)
    expect(fields('loop')).toEqual(
      expect.arrayContaining(['output', 'passes', 'count', 'results', 'outputs'])
    )
    expect(fields('approval')).toEqual(
      expect.arrayContaining(['text', 'items', 'comments', 'feedback', 'feedbackAll', 'round'])
    )
  })

  it('filters to the types asked for', async () => {
    const reference = await describeNodes({ types: ['loop', 'approval'] })
    expect(Object.keys(reference.nodeTypes).sort()).toEqual(['approval', 'loop'])
    expect(reference.templates.namespaces.loop).toBeTruthy()
  })

  it('states the limits the engine enforces', async () => {
    const { limits } = await describeNodes()
    expect(limits).toMatchObject({
      repeatMaxIterations: 10,
      forEachMaxItems: 'uncapped',
      maxGateRounds: 10,
      templateTextChars: 50_000,
      loopResultOutputChars: 8000
    })
  })

  it('embeds an example that create_workflow would accept', async () => {
    const { example } = await describeNodes()
    const parsed = example.nodes.map((n) => nodeSchema.parse(n))
    const edges = example.edges.map((e) => edgeSchema.parse(e)) as WorkflowEdge[]
    // The MCP checks take nodes as the tool parsed them; the shared one takes workflow nodes.
    const nodes = parsed as unknown as WorkflowNode[]

    expect(validateLoopBodies(parsed)).toEqual([])
    const loops = nodes.filter((n) => n.type === 'loop')
    expect(loops).toHaveLength(1)
    expect(loopStructureError(nodes, edges, loops[0])).toBeUndefined()
    expect(validateGraph(parsed, edges)).toEqual([])
  })
})
