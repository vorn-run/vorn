import { describe, it, expect } from 'vitest'
import type {
  ConnectorConfigField,
  SourceConnection,
  WorkflowDefinition,
  WorkflowNode
} from '../packages/shared/src/types'
import {
  envNamesOf,
  listKeys,
  maskSecret,
  passwordFields,
  usageCounts
} from '../packages/server/src/connectors/keys'

const field = (key: string, type: ConnectorConfigField['type']): ConnectorConfigField => ({
  key,
  label: key,
  type
})

const connection = (over: Partial<SourceConnection> = {}): SourceConnection => ({
  id: 'conn-1',
  connectorId: 'http',
  name: 'reporting API',
  filters: { secret: 'ciphertext' },
  syncIntervalMinutes: 5,
  statusMapping: {},
  createdAt: '2026-09-02T00:00:00Z',
  ...over
})

const node = (
  id: string,
  type: WorkflowNode['type'],
  config: Record<string, unknown>
): WorkflowNode =>
  ({ id, type, label: id, config, position: { x: 0, y: 0 } }) as unknown as WorkflowNode

const workflow = (id: string, nodes: WorkflowNode[]): WorkflowDefinition =>
  ({
    id,
    name: id,
    icon: 'Zap',
    iconColor: '#fff',
    enabled: true,
    nodes,
    edges: []
  }) as unknown as WorkflowDefinition

describe('which fields are secrets', () => {
  it('takes the connector at its word, and nothing else', () => {
    const auth = [field('baseUrl', 'text'), field('secret', 'password')]
    expect(passwordFields(auth).map((f) => f.key)).toEqual(['secret'])
    expect(passwordFields(undefined)).toEqual([])
  })
})

describe('how much of a secret travels', () => {
  it('names the service and the mode, then the last four to tell keys apart', () => {
    expect(maskSecret('sk_live_51abcdefgh4242')).toBe('sk_live_••••4242')
    expect(maskSecret('xoxb-1111-2222-abcd')).toBe('xoxb-••••abcd')
  })

  it('gives up no opening at all for a token it does not recognize', () => {
    // A fixed slice of an unknown key is a slice of the secret.
    expect(maskSecret('a1b2c3d4e5f6g7h8')).toBe('••••g7h8')
  })

  it('shows nothing of a short value, which a tail would mostly give away', () => {
    expect(maskSecret('short')).toBe('••••')
    expect(maskSecret('sk_live_abc')).toBe('••••')
  })

  it('says nothing when there is nothing to read', () => {
    expect(maskSecret(undefined)).toBe('')
  })

  it('never carries the middle of a key, whatever the key looks like', () => {
    const secret = 'sk_live_MIDDLEPARTffff'
    expect(maskSecret(secret)).not.toContain('MIDDLEPART')
  })
})

describe('the env a blob carries', () => {
  it('names them without disclosing their values', () => {
    expect(envNamesOf('{"SLACK_BOT_TOKEN":"xoxb-secret","PORT":"1"}')).toEqual([
      'SLACK_BOT_TOKEN',
      'PORT'
    ])
  })

  it('answers nothing for a blob it cannot read', () => {
    expect(envNamesOf('not json')).toEqual([])
    expect(envNamesOf(undefined)).toEqual([])
    expect(envNamesOf('[1,2]')).toEqual([])
  })
})

describe('what rotating a key would touch', () => {
  const workflows = [
    workflow('wf-1', [
      node('a', 'callConnectorAction', { connectionId: 'conn-1' }),
      node('b', 'httpRequest', { profileConnectionId: 'conn-1' }),
      node('c', 'script', { scriptContent: '' })
    ]),
    workflow('wf-2', [
      node('d', 'trigger', { triggerType: 'connectorPoll', connectionId: 'conn-2' }),
      // A trigger that runs on nothing external names no connection.
      node('e', 'trigger', { triggerType: 'manual' })
    ])
  ]

  it('counts the steps, not the workflows', () => {
    const counts = usageCounts(workflows)
    expect(counts.get('conn-1')).toBe(2)
    expect(counts.get('conn-2')).toBe(1)
  })

  it('ignores a step whose connection was never chosen', () => {
    const counts = usageCounts([workflow('wf-3', [node('a', 'callConnectorAction', {})])])
    expect(counts.size).toBe(0)
  })

  it('counts a script that borrows a key, which is a use like any other', () => {
    const counts = usageCounts([
      workflow('wf-4', [
        node('smoke', 'script', { scriptContent: '', secretsFrom: 'conn-1' }),
        node('plain', 'script', { scriptContent: '' })
      ])
    ])
    expect(counts.get('conn-1')).toBe(1)
  })

  it('counts a step that both runs against a connection and borrows another', () => {
    const counts = usageCounts([
      workflow('wf-5', [node('a', 'script', { scriptContent: '', secretsFrom: 'conn-2' })])
    ])
    expect(counts.get('conn-2')).toBe(1)
  })
})

describe('the keys this machine holds', () => {
  const auth = new Map([
    ['http', [field('baseUrl', 'text'), field('secret', 'password')]],
    ['mcp', [field('command', 'text'), field('secretEnv', 'password')]]
  ])

  it('describes a single-value key by its opening and its use', () => {
    const keys = listKeys(
      [connection()],
      auth,
      [workflow('wf-1', [node('a', 'httpRequest', { profileConnectionId: 'conn-1' })])],
      () => ({ secret: 'sk_live_51abcdefgh4242' })
    )
    expect(keys).toEqual([
      {
        connectionId: 'conn-1',
        name: 'reporting API',
        connectorId: 'http',
        usageCount: 1,
        fields: [{ key: 'secret', label: 'secret', readable: true, hint: 'sk_live_••••4242' }]
      }
    ])
  })

  it('describes a packaged connector by the env it carries, under its own id', () => {
    const packaged = connection({
      id: 'conn-2',
      connectorId: 'mcp',
      name: 'Slack',
      filters: { secretEnv: 'ciphertext', sdkConnectorId: 'slack' }
    })
    const keys = listKeys([packaged], auth, [], () => ({
      secretEnv: '{"SLACK_BOT_TOKEN":"xoxb-secret"}'
    }))
    expect(keys[0]).toMatchObject({
      connectorId: 'slack',
      usageCount: 0,
      fields: [{ key: 'secretEnv', readable: true, envNames: ['SLACK_BOT_TOKEN'] }]
    })
    // The value itself must never be part of the answer.
    expect(JSON.stringify(keys)).not.toContain('xoxb-secret')
  })

  it('lists a key this build cannot read, because it is still one to rotate', () => {
    const keys = listKeys([connection()], auth, [], () => undefined)
    expect(keys[0].fields[0]).toEqual({
      key: 'secret',
      label: 'secret',
      readable: false,
      hint: ''
    })
  })

  it('leaves out a connection holding no secret at all', () => {
    const empty = connection({ id: 'conn-3', filters: { baseUrl: 'https://x.test' } })
    expect(listKeys([empty], auth, [], () => ({}))).toEqual([])
  })

  it('leaves out a connector that declares no secret field', () => {
    const builtIn = connection({ id: 'conn-4', connectorId: 'github', filters: { repo: 'x/y' } })
    expect(listKeys([builtIn], auth, [], () => ({}))).toEqual([])
  })
})
