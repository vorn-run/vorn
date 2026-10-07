import http from 'node:http'
import type { AddressInfo } from 'node:net'
import Fastify, { type FastifyInstance } from 'fastify'
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

import { registerWorkRoutes } from '../packages/server/src/vornd-routes'
import { relayWorkCall } from '../packages/server/src/workflow-triggers'

/**
 * The work model's pages on the server's port, relayed to vornd, which
 * answers them: the addresses already handed out keep working. A webhook is
 * taken from this machine only.
 */

interface Seen {
  method?: string
  url?: string
  headers: http.IncomingHttpHeaders
  body: string
}

let vornd: http.Server
let vorndPort: number
let seen: Seen
let app: FastifyInstance
let port: number | null

beforeAll(async () => {
  vornd = http.createServer((req, res) => {
    let body = ''
    req.on('data', (c) => (body += c))
    req.on('end', () => {
      seen = { method: req.method, url: req.url, headers: req.headers, body }
      res.writeHead(req.url?.startsWith('/wf-hooks') ? 202 : 200, {
        'content-type': 'text/html; charset=utf-8',
        'content-security-policy': "default-src 'none'"
      })
      res.end(req.url?.startsWith('/wf-hooks') ? '{"accepted":true}' : '<h1>page</h1>')
    })
  })
  await new Promise<void>((r) => vornd.listen(0, '127.0.0.1', r))
  vorndPort = (vornd.address() as AddressInfo).port
  app = Fastify()
  registerWorkRoutes(app, () => port)
  await app.ready()
})

afterAll(async () => {
  await app.close()
  await new Promise((r) => vornd.close(r))
})

beforeEach(() => {
  port = vorndPort
  seen = { headers: {}, body: '' }
})

describe('the work pages on the server', () => {
  it('hands an artifact and a review page to vornd, with what they need', async () => {
    for (const url of ['/artifact/a1/2?t=tok', '/gate-view/r1/g1?t=tok']) {
      const res = await app.inject({
        method: 'GET',
        url,
        headers: { 'x-trace': 'kept', cookie: 'no' }
      })
      expect(res.statusCode).toBe(200)
      expect(res.body).toBe('<h1>page</h1>')
      expect(res.headers['content-security-policy']).toBe("default-src 'none'")
      expect(seen.url).toBe(url)
      expect(seen.headers['x-trace']).toBe('kept')
      expect(seen.headers.cookie).toBeUndefined()
      expect(seen.headers['x-vorn-peer']).toBe('127.0.0.1')
    }
  })

  it('hands a webhook to vornd with its body and its delivery key', async () => {
    const res = await app.inject({
      method: 'POST',
      url: '/wf-hooks/wf-1/tok?q=1',
      headers: { 'content-type': 'application/json', 'idempotency-key': 'k1' },
      payload: { n: 7 }
    })
    expect(res.statusCode).toBe(202)
    expect(res.json()).toEqual({ accepted: true })
    expect(seen).toMatchObject({ method: 'POST', url: '/wf-hooks/wf-1/tok?q=1', body: '{"n":7}' })
    expect(seen.headers['idempotency-key']).toBe('k1')
    expect(seen.headers['content-type']).toContain('application/json')
  })

  it('takes a webhook from this machine only', async () => {
    const res = await app.inject({
      method: 'GET',
      url: '/wf-hooks/wf-1/tok',
      remoteAddress: '10.0.0.9'
    })
    expect(res.statusCode).toBe(403)
    expect(seen.url).toBeUndefined()
  })

  it('says so when vornd is not running, or does not answer', async () => {
    port = null
    expect((await app.inject({ method: 'GET', url: '/artifact/a/1?t=x' })).statusCode).toBe(503)
    port = 1
    expect((await app.inject({ method: 'GET', url: '/artifact/a/1?t=x' })).statusCode).toBe(502)
  })
})

describe('a work call made to the server', () => {
  it('is vornd’s answer to it', async () => {
    const ask = vi.fn(async () => ({ result: [{ id: 'wf' }] }))
    const relay = relayWorkCall({ ask: ask as never })
    await expect(relay('workflow:list', undefined)).resolves.toEqual([{ id: 'wf' }])
    expect(ask).toHaveBeenCalledWith(
      'vornd:work',
      { method: 'workflow:list', params: undefined },
      60_000
    )
  })

  it('waits for vornd while it starts', async () => {
    let calls = 0
    const ask = async (): Promise<unknown> => (++calls < 3 ? null : { result: 'late' })
    const relay = relayWorkCall({ ask: ask as never }, 5_000)
    await expect(relay('scheduler:getLog', 'w')).resolves.toBe('late')
    expect(calls).toBe(3)
  })

  it('leaves every other call to the server, and fails without vornd', async () => {
    const relay = relayWorkCall({ ask: (async () => null) as never }, 50)
    expect(relay('task:list', undefined)).toBeUndefined()
    await expect(relay('artifact:get', { artifactId: 'a' })).rejects.toThrow(/vornd is not running/)
  })
})
