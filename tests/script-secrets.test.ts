import { describe, it, expect } from 'vitest'
import { secretEnvFor } from '../packages/server/src/script-runner'
import { fromPortable, toPortable } from '../packages/shared/src/workflow-portability'
import type { WorkflowDefinition } from '../packages/shared/src/types'

describe('the environment a step borrows from a connection', () => {
  const decrypted: Record<string, Record<string, string>> = {
    'conn-http': { secret: 'sk_live_abc' },
    'conn-slack': { secretEnv: '{"SLACK_BOT_TOKEN":"xoxb-abc","PORT":"1"}' },
    'conn-mixed': { apiKey: 'k', secretEnv: '{"TOKEN":"t"}' }
  }
  const lookup = (id: string) => decrypted[id]

  it('names a single value after the field that holds it', () => {
    expect(secretEnvFor('conn-http', lookup)).toEqual({ SECRET: 'sk_live_abc' })
  })

  it('spreads a blob as the variables it already names', () => {
    expect(secretEnvFor('conn-slack', lookup)).toEqual({
      SLACK_BOT_TOKEN: 'xoxb-abc',
      PORT: '1'
    })
  })

  it('takes both kinds from one connection', () => {
    expect(secretEnvFor('conn-mixed', lookup)).toEqual({ API_KEY: 'k', TOKEN: 't' })
  })

  it('gives a step that asked for nothing nothing at all', () => {
    expect(secretEnvFor(undefined, lookup)).toEqual({})
  })

  it('gives nothing for a connection this machine cannot read', () => {
    expect(secretEnvFor('conn-missing', lookup)).toEqual({})
    expect(secretEnvFor('conn-bad', () => ({ secretEnv: 'not json' }))).toEqual({})
  })
})

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

  it('names no connection of this machine, so the file asks rather than inherits', () => {
    const portable = toPortable(withSecrets, '/Users/someone/dev/novum')
    const config = portable.nodes[0].config as Record<string, unknown>
    expect(config.secretsFrom).toBeUndefined()
    // The script itself still travels; only the binding is local.
    expect(config.scriptContent).toBe('echo "$SLACK_BOT_TOKEN"')
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
    const definition = fromPortable(carried as never, 'bundle', {
      name: 'Novum',
      path: '/Users/someone/dev/novum'
    })
    expect((definition.nodes[0].config as Record<string, unknown>).secretsFrom).toBeUndefined()
  })
})
