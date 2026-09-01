import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'
import Fastify, { type FastifyInstance } from 'fastify'
import type { WorkflowDefinition } from '../packages/shared/src/types'
import {
  dbClaimConnectorInbox,
  dbInsertWorkflow,
  dbRetryConnectorInbox,
  initTestDatabase,
  MAX_INBOX_ATTEMPTS
} from '../packages/server/src/database'
import { registerWebhookRoute } from '../packages/server/src/webhook-trigger'

let teardown: () => void
let app: FastifyInstance
let onEnqueued: Mock<() => void>

const webhookWorkflow = (overrides: Partial<WorkflowDefinition> = {}): WorkflowDefinition => ({
  id: 'wf-hook',
  name: 'Webhook workflow',
  icon: 'Workflow',
  iconColor: '#fff',
  enabled: true,
  nodes: [
    {
      id: 'trigger-1',
      type: 'trigger',
      label: 'Trigger',
      config: { triggerType: 'webhook', method: 'POST', token: 'sekret-token' },
      position: { x: 0, y: 0 }
    }
  ],
  edges: [],
  ...overrides
})

const claimAll = () =>
  dbClaimConnectorInbox({
    now: new Date().toISOString(),
    leaseUntil: new Date(Date.now() + 60_000).toISOString(),
    limit: 10
  })

beforeEach(async () => {
  teardown = initTestDatabase()
  onEnqueued = vi.fn<() => void>()
  app = Fastify()
  registerWebhookRoute(app, onEnqueued)
  await app.ready()
})

afterEach(async () => {
  await app.close()
  teardown()
})

describe('the webhook route', () => {
  it('accepts a matching request and lands it in the durable inbox', async () => {
    dbInsertWorkflow(webhookWorkflow())
    const res = await app.inject({
      method: 'POST',
      url: '/wf-hooks/wf-hook/sekret-token',
      headers: { 'content-type': 'application/json', 'x-event': 'deploy' },
      payload: { n: 1 }
    })
    expect(res.statusCode).toBe(202)
    expect(onEnqueued).toHaveBeenCalledOnce()

    const [item] = claimAll()
    expect(item.workflowId).toBe('wf-hook')
    expect(item.connectorId).toBe('webhook')
    const raw = item.connectorItem.raw as {
      body: unknown
      headers: Record<string, string>
      method: string
    }
    expect(raw.body).toEqual({ n: 1 })
    expect(raw.method).toBe('POST')
    expect(raw.headers['x-event']).toBe('deploy')
  })

  it('keeps auth-bearing headers out of the recorded event', async () => {
    dbInsertWorkflow(webhookWorkflow())
    await app.inject({
      method: 'POST',
      url: '/wf-hooks/wf-hook/sekret-token',
      headers: { 'content-type': 'application/json', authorization: 'Bearer topsecret' },
      payload: {}
    })
    const [item] = claimAll()
    const raw = item.connectorItem.raw as { headers: Record<string, string> }
    expect(raw.headers.authorization).toBeUndefined()
    expect(JSON.stringify(item)).not.toContain('topsecret')
  })

  it('drops every auth-bearing header, not just authorization', async () => {
    dbInsertWorkflow(webhookWorkflow())
    await app.inject({
      method: 'POST',
      url: '/wf-hooks/wf-hook/sekret-token',
      headers: {
        cookie: 'sid=1',
        'x-api-key': 'k',
        'proxy-authorization': 'p',
        'x-event': 'deploy'
      },
      payload: {}
    })
    const [item] = claimAll()
    const raw = item.connectorItem.raw as { headers: Record<string, string> }
    expect(raw.headers.cookie).toBeUndefined()
    expect(raw.headers['x-api-key']).toBeUndefined()
    expect(raw.headers['proxy-authorization']).toBeUndefined()
    expect(raw.headers['x-event']).toBe('deploy')
  })

  it('answers 404 for a wrong token, an unknown workflow, and a disabled one alike', async () => {
    dbInsertWorkflow(webhookWorkflow())
    dbInsertWorkflow(webhookWorkflow({ id: 'wf-off', enabled: false }))

    const wrongToken = await app.inject({ method: 'POST', url: '/wf-hooks/wf-hook/nope' })
    const unknown = await app.inject({ method: 'POST', url: '/wf-hooks/wf-none/sekret-token' })
    const disabled = await app.inject({ method: 'POST', url: '/wf-hooks/wf-off/sekret-token' })

    for (const res of [wrongToken, unknown, disabled]) {
      expect(res.statusCode).toBe(404)
      expect(res.json()).toEqual({ error: 'Not found' })
    }
    expect(onEnqueued).not.toHaveBeenCalled()
    expect(claimAll()).toHaveLength(0)
  })

  it('answers 404 when the workflow trigger is not a webhook', async () => {
    dbInsertWorkflow(
      webhookWorkflow({
        id: 'wf-manual',
        nodes: [
          {
            id: 'trigger-1',
            type: 'trigger',
            label: 'Trigger',
            config: { triggerType: 'manual' },
            position: { x: 0, y: 0 }
          }
        ]
      })
    )
    const res = await app.inject({ method: 'POST', url: '/wf-hooks/wf-manual/sekret-token' })
    expect(res.statusCode).toBe(404)
  })

  it('answers the same 404 when only the method mismatches', async () => {
    dbInsertWorkflow(webhookWorkflow())
    const res = await app.inject({ method: 'GET', url: '/wf-hooks/wf-hook/sekret-token' })
    expect(res.statusCode).toBe(404)
    expect(res.json()).toEqual({ error: 'Not found' })
    expect(claimAll()).toHaveLength(0)
  })

  it('serves a GET trigger end to end with its query string captured', async () => {
    dbInsertWorkflow(
      webhookWorkflow({
        id: 'wf-get',
        nodes: [
          {
            id: 'trigger-1',
            type: 'trigger',
            label: 'Trigger',
            config: { triggerType: 'webhook', method: 'GET', token: 'sekret-token' },
            position: { x: 0, y: 0 }
          }
        ]
      })
    )
    const res = await app.inject({
      method: 'GET',
      url: '/wf-hooks/wf-get/sekret-token?ref=deploy&sha=abc123',
      headers: { 'x-github-event': 'push' }
    })
    expect(res.statusCode).toBe(202)
    const [item] = claimAll()
    const raw = item.connectorItem.raw as {
      query: Record<string, string>
      headers: Record<string, string>
      method: string
    }
    expect(raw.method).toBe('GET')
    expect(raw.query).toEqual({ ref: 'deploy', sha: 'abc123' })
    // Real provider headers come through; only auth-bearing names are dropped.
    expect(raw.headers['x-github-event']).toBe('push')
  })

  it('rejects requests that do not come from this machine', async () => {
    dbInsertWorkflow(webhookWorkflow())
    const res = await app.inject({
      method: 'POST',
      url: '/wf-hooks/wf-hook/sekret-token',
      remoteAddress: '192.168.1.20'
    })
    expect(res.statusCode).toBe(403)
    expect(claimAll()).toHaveLength(0)
  })
})

describe('the inbox retry cap', () => {
  it('marks a persistently failing event dead instead of retrying forever', async () => {
    dbInsertWorkflow(webhookWorkflow())
    await app.inject({ method: 'POST', url: '/wf-hooks/wf-hook/sekret-token', payload: {} })

    // A virtual clock that jumps past each round's backoff (capped at 1h).
    const base = Date.now()
    const HOURS_3 = 3 * 60 * 60_000
    for (let attempt = 1; attempt <= MAX_INBOX_ATTEMPTS; attempt++) {
      const now = base + attempt * HOURS_3
      const claimed = dbClaimConnectorInbox({
        now: new Date(now).toISOString(),
        leaseUntil: new Date(now + 60_000).toISOString(),
        limit: 10
      })
      expect(claimed).toHaveLength(1)
      const ok = dbRetryConnectorInbox({
        id: claimed[0].id,
        leaseToken: claimed[0].leaseToken,
        error: 'renderer exploded',
        now: new Date(now).toISOString()
      })
      expect(ok).toBe(true)
    }

    const after = dbClaimConnectorInbox({
      now: new Date(base + 100 * HOURS_3).toISOString(),
      leaseUntil: new Date(base + 101 * HOURS_3).toISOString(),
      limit: 10
    })
    expect(after).toHaveLength(0)
  })
})
