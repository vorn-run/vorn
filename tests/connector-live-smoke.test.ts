import { describe, expect, it, vi } from 'vitest'
import { checkConnector, defineConnector } from '../packages/connector-sdk/src/index'
import type {
  ActionDefinition,
  ConnectorDefinition,
  PreflightResult
} from '../packages/connector-sdk/src/types'

const read: ActionDefinition = {
  type: 'read',
  label: 'Read',
  description: 'Read something back',
  idempotent: true,
  outputs: [{ key: 'ok', type: 'boolean' }],
  run: () => ({ ok: true })
}

const connector = (over: Partial<ConnectorDefinition> = {}) =>
  defineConnector({
    id: 'acme',
    name: 'Acme',
    description: 'Talks to Acme',
    auth: { rung: 'none' },
    actions: [read],
    ...over
  })

const live = (over: Partial<ConnectorDefinition> = {}) =>
  checkConnector(connector(over), { live: true, config: {} })

describe('a live check, before it trusts anything else', () => {
  it('asks the connector whether it can sign in at all', async () => {
    const preflight = vi.fn(
      (): PreflightResult => ({ ok: false, message: 'Run `acme login` first' })
    )
    const findings = await live({ preflight })

    expect(preflight).toHaveBeenCalled()
    const failure = findings.find((item) => item.code === 'preflight-failed')
    expect(failure?.level).toBe('error')
    expect(failure?.message).toBe('Run `acme login` first')
  })

  it('reports a preflight that threw in the same shape as one that refused', async () => {
    const findings = await live({
      preflight: () => {
        throw new Error('acme is not installed')
      }
    })
    expect(findings.find((item) => item.code === 'preflight-failed')?.message).toContain(
      'acme is not installed'
    )
  })

  it('stops after a failed preflight, rather than blaming every action for it', async () => {
    const run = vi.fn(() => ({ ok: true }))
    const findings = await live({
      preflight: () => ({ ok: false }),
      actions: [{ ...read, run }]
    })

    expect(run).not.toHaveBeenCalled()
    expect(findings.filter((item) => item.code === 'live-action-failed')).toHaveLength(0)
  })

  it('says nothing when there is nothing to ask, which is not the same as passing', async () => {
    const findings = await live()
    expect(findings.map((item) => item.code)).not.toContain('preflight-failed')
  })
})

describe('what a live check does to a real service', () => {
  it('runs an action that says repeating it is safe', async () => {
    const run = vi.fn(() => ({ ok: true }))
    await live({ actions: [{ ...read, run }] })
    expect(run).toHaveBeenCalled()
  })

  it('never runs one that does not, because a smoke test must leave nothing behind', async () => {
    const run = vi.fn(() => ({ id: 'issue-1' }))
    const create: ActionDefinition = {
      type: 'createIssue',
      label: 'Create issue',
      description: 'Opens an issue',
      idempotent: false,
      outputs: [{ key: 'id', type: 'string' }],
      run
    }
    await live({ actions: [create] })
    expect(run).not.toHaveBeenCalled()
  })

  it('leaves an undeclared action alone too, because unknown is not safe', async () => {
    const run = vi.fn(() => ({}))
    const unknown: ActionDefinition = {
      type: 'maybe',
      label: 'Maybe',
      description: 'Nobody said',
      outputs: [{ key: 'ok' }],
      run
    }
    await live({ actions: [unknown] })
    expect(run).not.toHaveBeenCalled()
  })

  it('reports an action that threw against the real service', async () => {
    const findings = await live({
      actions: [
        {
          ...read,
          run: () => {
            throw new Error('403 from Acme')
          }
        }
      ]
    })
    const failure = findings.find((item) => item.code === 'live-action-failed')
    expect(failure?.level).toBe('error')
    expect(failure?.message).toContain('403 from Acme')
  })

  it('runs no action at all when the check was not asked to go live', async () => {
    const run = vi.fn(() => ({ ok: true }))
    await checkConnector(connector({ actions: [{ ...read, run }] }))
    expect(run).not.toHaveBeenCalled()
  })
})
