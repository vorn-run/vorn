import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import type { AddressInfo } from 'node:net'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

import { FakeVornd } from './helpers/fake-vornd'
import { until } from './helpers/vornd-sessions'
import { VorndSessions } from '../packages/server/src/vornd-sessions'
import { linkReach, relayPairing } from '../packages/server/src/vornd-reach'

/**
 * What vornd asks of the server once it holds pairing and writes tokens, and
 * the phone's pairing requests the server hands it.
 */

describe('vornd asking the server', () => {
  let dataDir: string
  let fake: FakeVornd
  let sessions: VorndSessions
  const broadcast = vi.fn()
  const disconnectToken = vi.fn(() => 1)
  let host = '127.0.0.1'

  beforeEach(async () => {
    dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vornd-reach-'))
    fake = new FakeVornd(dataDir)
    await fake.start()
    sessions = new VorndSessions()
    broadcast.mockClear()
    disconnectToken.mockClear()
    host = '127.0.0.1'
  })

  afterEach(async () => {
    sessions.close()
    await fake.stop()
    fs.rmSync(dataDir, { recursive: true, force: true })
  })

  it('says where the server is bound when it subscribes, and again when told', async () => {
    const reach = linkReach({ channel: sessions, broadcast, disconnectToken, host: () => host })
    expect(await sessions.tell('vornd:reach', { host })).toBe(false)
    await sessions.connect(fake.endpoint)
    await until('the host', () => fake.made('vornd:reach').length === 1)
    expect(fake.made('vornd:reach')).toEqual([{ host: '127.0.0.1' }])

    host = '0.0.0.0'
    reach.hostChanged()
    await until('the new host', () => fake.made('vornd:reach').length === 2)
    expect(fake.made('vornd:reach')[1]).toEqual({ host: '0.0.0.0' })
  })

  it('broadcasts what pairing and the configuration announce, and nothing else', async () => {
    linkReach({ channel: sessions, broadcast, disconnectToken, host: () => host })
    await sessions.connect(fake.endpoint)
    const request = { requestId: 'r1', deviceName: 'Pixel' }
    fake.send('vornd:broadcast', { method: 'pairing:requested', params: request })
    fake.send('vornd:broadcast', { method: 'terminal:data', params: {} })
    fake.send('vornd:broadcast', { method: 'config:changed', params: {} })
    fake.send('vornd:broadcast', { method: 'pairing:collected', params: { requestId: 'r1' } })
    await until('the broadcasts', () => broadcast.mock.calls.length === 3)
    expect(broadcast.mock.calls).toEqual([
      ['pairing:requested', request, undefined],
      ['config:changed', {}, undefined],
      ['pairing:collected', { requestId: 'r1' }, undefined]
    ])
  })

  it('broadcasts an extension push to the clients of its session', async () => {
    linkReach({ channel: sessions, broadcast, disconnectToken, host: () => host })
    await sessions.connect(fake.endpoint)
    const readings = { sessionId: 's1', readings: [] }
    fake.send('vornd:broadcast', { method: 'extension:footerItems', params: readings, scope: 's1' })
    fake.send('vornd:broadcast', { method: 'extension:selectionRequest', params: {}, scope: 7 })
    await until('the broadcasts', () => broadcast.mock.calls.length === 2)
    expect(broadcast.mock.calls).toEqual([
      ['extension:footerItems', readings, 's1'],
      ['extension:selectionRequest', {}, undefined]
    ])
  })

  it('asks the desktop for vornd and hands back its answer or its failure', async () => {
    const bridge = {
      request: vi.fn(async (method: string) => {
        if (method === 'session:check') return { signedIn: true }
        if (method === 'session:forget') throw 'gone'
        throw new Error('no desktop')
      })
    }
    linkReach({ channel: sessions, broadcast, disconnectToken, host: () => host, bridge })
    await sessions.connect(fake.endpoint)
    fake.send('vornd:ask', {
      id: 1,
      method: 'session:check',
      params: { connectionId: 'c' },
      timeoutMs: 5
    })
    fake.send('vornd:ask', { id: 2, method: 'session:fetch', params: {} })
    fake.send('vornd:ask', { id: 'x', method: 'session:fetch' })
    fake.send('vornd:ask', { id: 3, method: 'session:forget', params: 'c' })
    await until('the answers', () => fake.made('vornd:answer').length === 3)
    expect(bridge.request).toHaveBeenCalledWith('session:check', { connectionId: 'c' }, 5)
    expect(fake.made('vornd:answer')).toEqual([
      { id: 1, result: { signedIn: true } },
      { id: 2, error: 'no desktop' },
      { id: 3, error: 'gone' }
    ])
  })

  it('closes the sockets of a token vornd revoked', async () => {
    linkReach({ channel: sessions, broadcast, disconnectToken, host: () => host })
    await sessions.connect(fake.endpoint)
    fake.send('vornd:tokenRevoked', { tokenId: 7 })
    fake.send('vornd:tokenRevoked', { tokenId: 'tok-1' })
    await until('the revocation', () => disconnectToken.mock.calls.length === 1)
    expect(disconnectToken).toHaveBeenCalledWith('tok-1')
  })
})

describe("relaying a phone's pairing request to vornd", () => {
  let server: http.Server
  let port: number
  const seen: Array<{ url?: string; headers: http.IncomingHttpHeaders; body: string }> = []

  beforeEach(async () => {
    seen.length = 0
    server = http.createServer((req, res) => {
      let body = ''
      req.on('data', (c) => (body += c))
      req.on('end', () => {
        seen.push({ url: req.url, headers: req.headers, body })
        res.writeHead(400, { 'content-type': 'application/json' })
        res.end(JSON.stringify({ error: 'unknown' }))
      })
    })
    await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', () => resolve()))
    port = (server.address() as AddressInfo).port
  })

  afterEach(async () => {
    await new Promise<void>((resolve) => server.close(() => resolve()))
  })

  const request = {
    url: '/api/pair/redeem',
    body: { code: 'ABCD-EFGH', deviceName: 'Pixel' },
    ip: '192.168.1.9',
    fromVornd: false
  }

  it("answers vornd's status and body, naming the phone's address", async () => {
    expect(await relayPairing(request, port)).toEqual({ status: 400, body: { error: 'unknown' } })
    expect(seen).toHaveLength(1)
    expect(seen[0].url).toBe('/api/pair/redeem')
    expect(seen[0].headers['x-vorn-peer']).toBe('192.168.1.9')
    expect(seen[0].headers['content-type']).toBe('application/json')
    expect(JSON.parse(seen[0].body)).toEqual(request.body)
  })

  it('leaves it to the server when vornd does not hold pairing or sent it', async () => {
    expect(await relayPairing(request, null)).toBeNull()
    expect(await relayPairing({ ...request, fromVornd: true }, port)).toBeNull()
    expect(seen).toHaveLength(0)
  })

  it('leaves it to the server when vornd cannot be reached', async () => {
    await new Promise<void>((resolve) => server.close(() => resolve()))
    expect(await relayPairing({ ...request, body: undefined }, port)).toBeNull()
    server = http.createServer()
    await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', () => resolve()))
  })
})
