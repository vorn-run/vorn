import { describe, it, expect } from 'vitest'
import {
  connectorSuggestions,
  mergeRequirements,
  requirementAction,
  requirementsOfDefinition,
  requirementsWithBindings,
  templateRequirements,
  templateIsReady,
  templateSeed,
  type TemplateRequirement
} from '../src/renderer/lib/template-requirements'
import type { ConnectorListing } from '../src/renderer/lib/connector-browse'
import { TEMPLATE_SEED } from '../packages/server/src/connectors/template-seed'
import type {
  ConnectorManifest,
  SourceConnection,
  WorkflowNode,
  WorkflowTemplate
} from '../packages/shared/src/types'

function connection(overrides: Partial<SourceConnection> = {}): SourceConnection {
  return {
    id: 'conn-1',
    name: 'reporting API',
    connectorId: 'http',
    filters: {},
    syncIntervalMinutes: 5,
    statusMapping: {},
    createdAt: '2026-09-01T00:00:00Z',
    ...overrides
  }
}

function template(id: string): WorkflowTemplate {
  return TEMPLATE_SEED.find((t) => t.id === id)!
}

const PROJECT = { name: 'Novum', path: '/Users/someone/dev/novum' }

describe('what a template still needs', () => {
  it('reports an unanswered requirement', () => {
    const [entry] = templateRequirements(template('webhook-to-report'), [])
    expect(entry.connectionId).toBeUndefined()
    expect(entry.requirement).toMatchObject({ kind: 'httpProfile', name: 'reporting API' })
    expect(templateIsReady(template('webhook-to-report'), [])).toBe(false)
  })

  it('answers it from the one profile this machine has', () => {
    const [entry] = templateRequirements(template('webhook-to-report'), [connection()])
    expect(entry.connectionId).toBe('conn-1')
    expect(templateIsReady(template('webhook-to-report'), [connection()])).toBe(true)
  })

  it('calls a template with nothing to connect ready', () => {
    expect(templateRequirements(template('morning-digest'), [])).toEqual([])
    expect(templateIsReady(template('morning-digest'), [])).toBe(true)
  })
})

describe('what a connection already knows how to build', () => {
  function manifest(defaults?: ConnectorManifest['defaultWorkflows']): ConnectorManifest {
    return { defaultWorkflows: defaults } as ConnectorManifest
  }

  const seeded = [
    {
      name: 'New issues to tasks',
      event: 'issueCreated',
      defaultCronFromMinutes: 5,
      downstream: 'createTaskFromItem' as const
    }
  ]

  it('offers one row per workflow the connector ships', () => {
    const suggestions = connectorSuggestions(
      [connection({ id: 'c1', name: 'workspace-eng', connectorId: 'github' })],
      [{ id: 'github', manifest: manifest(seeded) }]
    )
    expect(suggestions).toEqual([
      {
        key: 'c1:issueCreated',
        connectionId: 'c1',
        connectionName: 'workspace-eng',
        event: 'issueCreated',
        name: 'New issues to tasks'
      }
    ])
  })

  it('finds a packaged connector by its manifest id rather than mcp', () => {
    const packaged = connection({
      id: 'c2',
      connectorId: 'mcp',
      filters: { sdkConnectorId: 'packdemo' }
    })
    const suggestions = connectorSuggestions(
      [packaged],
      [{ id: 'packdemo', manifest: manifest(seeded) }]
    )
    expect(suggestions.map((s) => s.connectionId)).toEqual(['c2'])
  })

  it('offers nothing for a connector that ships no workflow', () => {
    expect(
      connectorSuggestions(
        [connection({ connectorId: 'github' })],
        [{ id: 'github', manifest: manifest() }]
      )
    ).toEqual([])
  })

  it('offers nothing for a connection whose connector is not installed', () => {
    expect(connectorSuggestions([connection({ connectorId: 'github' })], [])).toEqual([])
  })
})

describe('what a requirement can do about itself', () => {
  const listing = (over: Partial<ConnectorListing> = {}): ConnectorListing => ({
    key: 'catalog:slack',
    id: 'slack',
    name: 'Slack',
    capabilities: ['actions'],
    category: 'Chat',
    source: 'catalog',
    keywords: [],
    connectedCount: 0,
    ...over
  })

  const needs = (connectorId: string): TemplateRequirement => ({
    requirement: { kind: 'connection', nodeId: 'n1', connectorId, name: 'workspace' }
  })

  it('offers the install when the connector is published but not on disk', () => {
    expect(requirementAction(needs('slack'), [listing()])).toMatchObject({ kind: 'install' })
  })

  it('offers the connection once the pack is installed', () => {
    const pack = { id: 'slack', name: 'Slack', version: '1.2.0' } as ConnectorListing['pack']
    expect(requirementAction(needs('slack'), [listing({ pack })])).toMatchObject({
      kind: 'addConnection'
    })
  })

  it('offers the connection straight away for a built-in', () => {
    const builtIn = listing({ key: 'github', id: 'github', name: 'GitHub', source: 'builtin' })
    expect(requirementAction(needs('github'), [builtIn])).toMatchObject({ kind: 'addConnection' })
  })

  it('offers a profile form for an HTTP profile, which needs no install', () => {
    const action = requirementAction(
      { requirement: { kind: 'httpProfile', nodeId: 'n1', name: 'reporting API' } },
      []
    )
    expect(action).toEqual({ kind: 'createProfile', name: 'reporting API' })
  })

  it('offers nothing for a requirement this machine already answers', () => {
    const met = { ...needs('slack'), connectionId: 'conn-9' }
    expect(requirementAction(met, [listing()])).toEqual({ kind: 'none' })
  })

  it('offers nothing when the file could not name the connector', () => {
    expect(requirementAction(needs(''), [listing()])).toEqual({ kind: 'none' })
  })

  it('offers nothing for a connector no catalog here has heard of', () => {
    expect(requirementAction(needs('obscure'), [listing()])).toEqual({ kind: 'none' })
  })

  // A listed server is a command rather than a package: the form the panel
  // would open has no manifest to probe and no pack to install.
  it('offers nothing for an MCP server, which is wired up by hand', () => {
    const server = listing({ key: 'mcp:playwright', id: 'playwright', source: 'mcp' })
    expect(requirementAction(needs('playwright'), [server])).toEqual({ kind: 'none' })
  })
})

describe('what a template puts on the canvas', () => {
  it('resolves the project tokens against the project in view', () => {
    const seed = templateSeed(template('morning-digest'), PROJECT, [])
    const script = seed.nodes.find((n) => n.id === 'gather')!.config as Record<string, unknown>
    expect(script.projectPath).toBe(PROJECT.path)
    expect(script.projectName).toBe('Novum')
  })

  it('leaves the project blank rather than literal when there is no project', () => {
    const seed = templateSeed(template('morning-digest'), undefined, [])
    const script = seed.nodes.find((n) => n.id === 'gather')!.config as Record<string, unknown>
    expect(script.projectPath).toBe('')
  })

  it('mints a webhook token, so two installs never share one', () => {
    let n = 0
    const first = templateSeed(template('webhook-to-report'), PROJECT, [], () => `token-${++n}`)
    const second = templateSeed(template('webhook-to-report'), PROJECT, [], () => `token-${++n}`)

    const tokenOf = (seed: typeof first): unknown =>
      (seed.nodes.find((node) => node.id === 'trigger')!.config as Record<string, unknown>).token
    expect(tokenOf(first)).toBe('token-1')
    expect(tokenOf(second)).toBe('token-2')
  })

  it('replaces a token the catalog carried rather than trusting it', () => {
    // A published token would be one secret shared by every install that used
    // the template, so what the file says is never what gets used.
    const published = template('webhook-to-report')
    const carried = {
      ...published,
      portable: {
        ...published.portable,
        nodes: published.portable.nodes.map((node) =>
          node.id === 'trigger'
            ? { ...node, config: { ...node.config, token: 'from-the-catalog' } }
            : node
        )
      }
    }

    const seed = templateSeed(carried, PROJECT, [], () => 'mine')
    const trigger = seed.nodes.find((node) => node.id === 'trigger')!.config as Record<
      string,
      unknown
    >
    expect(trigger.token).toBe('mine')
  })

  it('binds the step when this machine can answer, and says so when it cannot', () => {
    const bound = templateSeed(template('webhook-to-report'), PROJECT, [connection()])
    const report = bound.nodes.find((n) => n.id === 'report')!.config as Record<string, unknown>
    expect(report.profileConnectionId).toBe('conn-1')
    expect(bound.unresolved).toEqual([])

    const unbound = templateSeed(template('webhook-to-report'), PROJECT, [])
    expect(unbound.unresolved).toHaveLength(1)
    expect(
      (unbound.nodes.find((n) => n.id === 'report')!.config as Record<string, unknown>)
        .profileConnectionId
    ).toBeUndefined()
  })

  it('keeps the graph the template drew', () => {
    const seed = templateSeed(template('webhook-to-report'), PROJECT, [])
    expect(seed.nodes).toHaveLength(3)
    expect(seed.edges.find((e) => e.conditionBranch === 'true')).toBeTruthy()
    expect(seed.name).toBe('Webhook to report')
  })
})

describe('what the canvas itself is still missing', () => {
  const node = (id: string, type: string, config: Record<string, unknown>): WorkflowNode =>
    ({ id, type, label: id, config, position: { x: 0, y: 0 } }) as WorkflowNode

  const unbound = node('n1', 'callConnectorAction', {
    nodeType: 'callConnectorAction',
    connectionId: '',
    connectorId: 'slack',
    action: 'post',
    args: {}
  })

  it('asks for the connector a step was picked from', () => {
    expect(requirementsOfDefinition([unbound])).toEqual([
      { kind: 'connection', nodeId: 'n1', connectorId: 'slack', name: '' }
    ])
  })

  it('says nothing about a step that is already bound', () => {
    const bound = node('n2', 'callConnectorAction', {
      nodeType: 'callConnectorAction',
      connectionId: 'conn-1',
      action: 'post',
      args: {}
    })
    expect(requirementsOfDefinition([bound])).toEqual([])
  })

  it('leaves a request to a public URL alone', () => {
    const request = node('n3', 'httpRequest', {
      nodeType: 'httpRequest',
      method: 'GET',
      url: 'https://example.test',
      headers: {},
      body: ''
    })
    expect(requirementsOfDefinition([request])).toEqual([])
  })

  it('asks about a profile that was chosen and then cleared', () => {
    const emptied = node('n5', 'httpRequest', {
      nodeType: 'httpRequest',
      method: 'GET',
      url: '/report',
      headers: {},
      body: '',
      profileConnectionId: ''
    })
    expect(requirementsOfDefinition([emptied])).toEqual([
      { kind: 'httpProfile', nodeId: 'n5', name: '' }
    ])
  })

  it('says nothing it could not offer a fix for', () => {
    const anonymous = node('n4', 'callConnectorAction', {
      nodeType: 'callConnectorAction',
      connectionId: '',
      action: 'post',
      args: {}
    })
    const strippedTrigger = node('n6', 'trigger', {
      triggerType: 'connectorPoll',
      connectionId: '',
      event: 'issueCreated'
    })
    // Both are fixed in the step's own panel, where the connector is chosen; a row here could offer nothing.
    expect(requirementsOfDefinition([anonymous, strippedTrigger])).toEqual([])
  })

  it('keeps what only the file knew about a step the canvas also names', () => {
    const fromFile = {
      kind: 'connection' as const,
      nodeId: 'n1',
      connectorId: 'slack',
      name: 'workspace-eng',
      event: 'message'
    }
    const merged = mergeRequirements(requirementsOfDefinition([unbound]), [fromFile])

    expect(merged).toEqual([fromFile])
  })

  it('falls back to the canvas for a field the file left empty', () => {
    const vague = { kind: 'connection' as const, nodeId: 'n1', connectorId: '', name: '' }
    const [merged] = mergeRequirements(requirementsOfDefinition([unbound]), [vague])

    expect(merged.kind === 'connection' && merged.connectorId).toBe('slack')
  })

  it('keeps a requirement for a step the canvas says nothing about', () => {
    const orphan = { kind: 'httpProfile' as const, nodeId: 'gone', name: 'reporting API' }
    expect(mergeRequirements([], [orphan])).toEqual([orphan])
  })

  it('binds a requirement the moment this machine has one answer for it', () => {
    const slack = connection({
      id: 'slack-1',
      name: 'workspace',
      connectorId: 'mcp',
      filters: { sdkConnectorId: 'slack' }
    })

    expect(requirementsWithBindings(requirementsOfDefinition([unbound]), [])).toEqual([
      { requirement: { kind: 'connection', nodeId: 'n1', connectorId: 'slack', name: '' } }
    ])
    expect(
      requirementsWithBindings(requirementsOfDefinition([unbound]), [slack])[0].connectionId
    ).toBe('slack-1')
  })
})
