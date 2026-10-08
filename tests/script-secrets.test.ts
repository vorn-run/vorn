import { describe, it, expect } from 'vitest'
import {
  fromPortable,
  toPortable,
  unresolvedRequirements,
  type PortableWorkflow
} from '../packages/shared/src/workflow-portability'
import type { WorkflowDefinition } from '../packages/shared/src/types'

describe('what a workflow file says about the keys it needs', () => {
  const withSecrets: WorkflowDefinition = {
    id: 'wf-1',
    name: 'Live smoke',
    icon: 'Zap',
    iconColor: '#6366f1',
    enabled: true,
    nodes: [
      {
        id: 'run',
        type: 'script',
        label: 'Smoke',
        config: {
          scriptType: 'bash',
          scriptContent: 'echo "$SLACK_BOT_TOKEN"',
          secretsFrom: 'conn-slack'
        },
        position: { x: 0, y: 0 }
      }
    ],
    edges: []
  } as unknown as WorkflowDefinition

  const slack = {
    id: 'conn-slack',
    name: 'Sandbox Slack',
    connectorId: 'mcp',
    filters: { sdkConnectorId: 'slack' }
  }

  it('asks for the key by name rather than carrying an id of this machine', () => {
    const portable = toPortable(withSecrets, '/Users/someone/dev/novum', [slack])
    const config = portable.nodes[0].config as Record<string, unknown>
    expect(config.secretsFrom).toBeUndefined()
    expect(portable.requires).toEqual([
      {
        kind: 'connection',
        nodeId: 'run',
        connectorId: 'slack',
        name: 'Sandbox Slack',
        key: 'secretsFrom'
      }
    ])
    // The script itself still travels; only the binding is local.
    expect(config.scriptContent).toBe('echo "$SLACK_BOT_TOKEN"')
  })

  it('never binds a key on import, even where the same connection is held', () => {
    // A file names the key it wants; handing one over is a person's choice, made in the step.
    const portable = toPortable(withSecrets, '/Users/someone/dev/novum', [slack])
    const here = { ...slack, id: 'conn-local' }
    const definition = fromPortable(portable, 'bundle', { name: 'Novum', path: '/x' }, [here])
    expect((definition.nodes[0].config as Record<string, unknown>).secretsFrom).toBeUndefined()
    expect(unresolvedRequirements(portable, [here])).toEqual(portable.requires)
  })

  it('leaves the step asking where this machine holds no such key', () => {
    const portable = toPortable(withSecrets, '/Users/someone/dev/novum', [slack])
    const definition = fromPortable(portable, 'bundle', { name: 'Novum', path: '/x' }, [])
    expect((definition.nodes[0].config as Record<string, unknown>).secretsFrom).toBeUndefined()
  })

  it('refuses one a file carries anyway, rather than binding to a stranger', () => {
    const carried = {
      version: 1,
      name: 'Live smoke',
      slug: 'live-smoke',
      nodes: [
        {
          id: 'run',
          type: 'script',
          label: 'Smoke',
          config: { scriptType: 'bash', scriptContent: 'x', secretsFrom: 'conn-elsewhere' },
          position: { x: 0, y: 0 }
        }
      ],
      edges: []
    }
    const definition = fromPortable(carried as unknown as PortableWorkflow, 'bundle', {
      name: 'Novum',
      path: '/Users/someone/dev/novum'
    })
    expect((definition.nodes[0].config as Record<string, unknown>).secretsFrom).toBeUndefined()
  })

  it('says nothing about a script that borrows no key at all', () => {
    const plain = {
      ...withSecrets,
      nodes: [
        {
          ...withSecrets.nodes[0],
          config: { scriptType: 'bash', scriptContent: 'echo hi' }
        }
      ]
    } as unknown as WorkflowDefinition
    expect(toPortable(plain, '/Users/someone/dev/novum').requires).toBeUndefined()
  })
})
