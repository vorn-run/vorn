import { describe, expect, it } from 'vitest'
import type { WorkflowNode } from '../packages/shared/src/types'
import {
  checkNodeConfigs,
  nodeSchema,
  triggerConfigSchema,
  validateGateFeedback
} from '../packages/mcp/src/tools/workflows'
import { NODE_TYPES } from '../packages/mcp/src/tools/node-config-schemas'

describe('MCP workflow authoring schemas', () => {
  it('accepts a webhook trigger', () => {
    const parsed = triggerConfigSchema.parse({
      triggerType: 'webhook',
      method: 'POST',
      token: 'abc123'
    })
    expect(parsed).toMatchObject({ triggerType: 'webhook', method: 'POST' })
  })

  it('rejects a webhook trigger with an unsupported method', () => {
    expect(() =>
      triggerConfigSchema.parse({ triggerType: 'webhook', method: 'DELETE', token: 'abc123' })
    ).toThrow()
  })

  it('accepts a connector poll trigger', () => {
    const parsed = triggerConfigSchema.parse({
      triggerType: 'connectorPoll',
      connectionId: 'conn-1',
      event: 'issueCreated',
      cron: '*/5 * * * *'
    })
    expect(parsed).toMatchObject({ triggerType: 'connectorPoll', event: 'issueCreated' })
  })

  it('accepts an httpRequest node', () => {
    const parsed = nodeSchema.parse({
      id: 'n1',
      type: 'httpRequest',
      label: 'Call API',
      slug: 'call-api',
      config: {
        nodeType: 'httpRequest',
        method: 'GET',
        url: 'https://x.test',
        headers: {},
        body: ''
      },
      position: { x: 0, y: 0 }
    })
    expect(parsed.type).toBe('httpRequest')
  })
})

const node = (type: string, config: Record<string, unknown>) => ({
  id: 'n1',
  type,
  label: 'Step',
  config,
  position: { x: 0, y: 0 }
})

// One sound config per node type, and one known key in it set to the wrong
// kind of value: what an agent guessing at a field most often gets wrong.
const CASES: { type: string; valid: Record<string, unknown>; wrong: [string, unknown] }[] = [
  {
    type: 'trigger',
    valid: { triggerType: 'sessionRestored', restore: 'any', concurrency: 'perProject' },
    wrong: ['restore', 'sometimes']
  },
  {
    type: 'launchAgent',
    valid: {
      agentType: 'fromTask',
      projectName: 'vorn',
      projectPath: '/code/vorn',
      prompt: 'Fix {{task.title}}',
      headless: true,
      outputSchema: { type: 'object' },
      worktreeMode: 'fromStep',
      worktreeFromStepSlug: 'build',
      timeoutMs: 60_000
    },
    wrong: ['timeoutMs', 'soon']
  },
  {
    type: 'script',
    valid: { scriptType: 'bash', scriptContent: 'echo hi', secretsFrom: 'conn-1' },
    wrong: ['scriptType', 'ruby']
  },
  {
    type: 'condition',
    valid: { variable: '{{steps.review.approved}}', operator: 'equals', value: 'true' },
    wrong: ['operator', 'greaterThan']
  },
  {
    type: 'approval',
    valid: {
      message: 'Ship it?',
      edit: '{{steps.review.findings}}',
      feedback: { from: 'build', maxRounds: 3 }
    },
    wrong: ['timeoutMs', '1h']
  },
  {
    type: 'createTaskFromItem',
    valid: { nodeType: 'createTaskFromItem', project: 'fromConnection', initialStatus: 'todo' },
    wrong: ['initialStatus', 'open']
  },
  {
    type: 'callConnectorAction',
    valid: {
      nodeType: 'callConnectorAction',
      connectionId: 'conn-1',
      action: 'commentOnPullRequest',
      args: { body: '{{loop.item.body}}' }
    },
    wrong: ['args', { line: 12 }]
  },
  {
    type: 'httpRequest',
    valid: { nodeType: 'httpRequest', method: 'POST', url: 'https://x.test', body: '{}' },
    wrong: ['method', 'FETCH']
  },
  {
    type: 'loop',
    valid: { nodeType: 'loop', mode: 'repeat', bodyNodeIds: ['a'], maxIterations: 3 },
    wrong: ['bodyNodeIds', 'a']
  }
]

describe('node configs are checked against their type', () => {
  it.each(CASES)('accepts a valid $type config', ({ type, valid }) => {
    expect(nodeSchema.safeParse(node(type, valid)).success).toBe(true)
  })

  it.each(CASES)('rejects a wrongly typed key on $type, naming it', ({ type, valid, wrong }) => {
    const [key, value] = wrong
    const result = nodeSchema.safeParse(node(type, { ...valid, [key]: value }))
    expect(result.success).toBe(false)
    expect(result.error?.issues.some((i) => i.path[0] === 'config' && i.path[1] === key)).toBe(true)
  })

  it.each(CASES)('passes unknown keys on $type through untouched', ({ type, valid }) => {
    const result = nodeSchema.safeParse(node(type, { ...valid, fromANewerBuild: { x: 1 } }))
    expect(result.success).toBe(true)
    expect(result.data?.config.fromANewerBuild).toEqual({ x: 1 })
  })

  it('covers every node type', () => {
    expect(CASES.map((c) => c.type).sort()).toEqual([...NODE_TYPES].sort())
  })

  it('refuses outputSchema on an agent that is not headless', () => {
    const result = nodeSchema.safeParse(
      node('launchAgent', {
        agentType: 'claude',
        projectName: 'vorn',
        projectPath: '/code/vorn',
        outputSchema: { type: 'object' }
      })
    )
    expect(result.error?.issues[0].path).toEqual(['config', 'outputSchema'])
  })
})

describe('loop configs', () => {
  const loop = (config: Record<string, unknown>) =>
    nodeSchema.safeParse(node('loop', { nodeType: 'loop', bodyNodeIds: ['a'], ...config }))

  it('rejects a forEach loop without items', () => {
    const result = loop({ mode: 'forEach' })
    expect(result.error?.issues[0].path).toEqual(['config', 'items'])
  })

  it('accepts a forEach loop with items and no maxIterations', () => {
    expect(loop({ mode: 'forEach', items: '{{steps.gate.items}}' }).success).toBe(true)
  })

  it('rejects a repeat loop past the pass cap', () => {
    const result = loop({ mode: 'repeat', maxIterations: 11 })
    expect(result.error?.issues[0].path).toEqual(['config', 'maxIterations'])
  })

  it('reads a loop without a mode as repeat, which needs maxIterations', () => {
    expect(loop({}).success).toBe(false)
    expect(loop({ maxIterations: 10 }).success).toBe(true)
  })

  it('rejects an empty body', () => {
    const result = loop({ maxIterations: 2, bodyNodeIds: [] })
    expect(result.error?.issues[0].path).toEqual(['config', 'bodyNodeIds'])
  })
})

describe('update_workflow config checks', () => {
  const broken = {
    id: 'l1',
    type: 'loop' as const,
    label: 'Old loop',
    config: { nodeType: 'loop', bodyNodeIds: ['a'], maxIterations: 50 },
    position: { x: 0, y: 0 }
  }

  it('lets a config left exactly as stored through as a warning', () => {
    const stored = [structuredClone(broken)] as unknown as WorkflowNode[]
    const { errors, warnings } = checkNodeConfigs([broken], stored)
    expect(errors).toEqual([])
    expect(warnings[0]).toContain('config.maxIterations')
  })

  it('holds a changed config to the current rules', () => {
    const stored = [{ ...broken, config: { ...broken.config, maxIterations: 40 } }]
    const { errors, warnings } = checkNodeConfigs([broken], stored as unknown as WorkflowNode[])
    expect(warnings).toEqual([])
    expect(errors[0]).toContain('Old loop')
  })

  it('holds a new node to the current rules', () => {
    expect(checkNodeConfigs([broken], []).errors).toHaveLength(1)
  })
})

describe('gate feedback', () => {
  it('refuses a gate that redoes from a step inside a loop body', () => {
    const nodes = [
      { id: 'loop', type: 'loop', label: 'Loop', config: { bodyNodeIds: ['write'] } },
      { id: 'write', type: 'script', label: 'Write', config: {} },
      {
        id: 'gate',
        type: 'approval',
        label: 'Gate',
        config: { feedback: { from: 'write', maxRounds: 3 } }
      }
    ]
    const edges = [
      { source: 'loop', target: 'write' },
      { source: 'write', target: 'gate' }
    ]
    expect(validateGateFeedback(nodes, edges).join()).toContain('inside loop "Loop"')
  })
})
