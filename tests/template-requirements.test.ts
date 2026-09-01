import { describe, it, expect } from 'vitest'
import {
  templateRequirements,
  templateIsReady,
  templateSeed
} from '../src/renderer/lib/template-requirements'
import { TEMPLATE_SEED } from '../packages/server/src/connectors/template-seed'
import type { SourceConnection, WorkflowTemplate } from '../packages/shared/src/types'

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
