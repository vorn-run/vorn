import { describe, it, expect, vi, afterEach } from 'vitest'
import http, { type IncomingMessage } from 'node:http'
import type { AddressInfo } from 'node:net'
import WebSocket, { WebSocketServer, type RawData } from 'ws'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

import {
  peerAddress,
  relayThroughVornd,
  relaysThroughVornd,
  VORN_PEER_HEADER
} from '../packages/server/src/vornd-relay'

/**
 * A socket from another machine carried to vornd whole: a stand-in vornd on
 * one side, a client on the other, and the relay between them as the server
 * runs it.
 */

const closers: Array<() => void> = []

afterEach(() => {
  for (const close of closers.splice(0)) close()
})

async function listening(server: http.Server): Promise<number> {
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', () => resolve()))
  closers.push(() => server.close())
  return (server.address() as AddressInfo).port
}

/** A stand-in for vornd that echoes, and keeps the upgrade it was given. */
async function standInVornd(
  onSocket?: (ws: WebSocket) => void
): Promise<{ port: number; upgrades: IncomingMessage[]; sockets: WebSocket[] }> {
  const server = http.createServer()
  const wss = new WebSocketServer({ server })
  const upgrades: IncomingMessage[] = []
  const sockets: WebSocket[] = []
  wss.on('connection', (ws, req) => {
    upgrades.push(req)
    sockets.push(ws)
    if (onSocket) return onSocket(ws)
    ws.on('message', (data, isBinary) => ws.send(data, { binary: isBinary }))
  })
  closers.push(() => wss.close())
  return { port: await listening(server), upgrades, sockets }
}

/** The server's side: every socket relayed to `vorndPort`, or served by `direct`. */
async function relayingServer(
  vorndPort: number,
  direct: (ws: WebSocket) => void = (ws) => ws.send('served here')
): Promise<number> {
  const server = http.createServer()
  const wss = new WebSocketServer({ server })
  wss.on('connection', (ws, req) =>
    relayThroughVornd(ws, req, vorndPort, '100.64.0.7', () => direct(ws))
  )
  closers.push(() => wss.close())
  return listening(server)
}

function connect(port: number, headers: Record<string, string> = {}): Promise<WebSocket> {
  const ws = new WebSocket(`ws://127.0.0.1:${port}/ws?topics=sessions`, { headers })
  closers.push(() => ws.terminate())
  return new Promise((resolve, reject) => {
    ws.once('open', () => resolve(ws))
    ws.once('error', reject)
  })
}

function next(ws: WebSocket): Promise<{ data: RawData; isBinary: boolean }> {
  return new Promise((resolve) =>
    ws.once('message', (data, isBinary) => resolve({ data, isBinary }))
  )
}

function closed(ws: WebSocket): Promise<{ code: number; reason: string }> {
  return new Promise((resolve) =>
    ws.once('close', (code, reason) => resolve({ code, reason: reason.toString() }))
  )
}

describe('who goes through vornd', () => {
  it('is a socket from another machine, while vornd is up', () => {
    expect(relaysThroughVornd('100.64.0.7', 47001)).toBe(true)
    expect(relaysThroughVornd('::ffff:192.168.1.20', 47001)).toBe(true)
    expect(relaysThroughVornd('100.64.0.7', null)).toBe(false)
    expect(relaysThroughVornd('127.0.0.1', 47001)).toBe(false)
    expect(relaysThroughVornd('::1', 47001)).toBe(false)
    expect(relaysThroughVornd(undefined, 47001)).toBe(false)
  })

  it('is judged by where it came from, which only this machine may say', () => {
    const relayed = { [VORN_PEER_HEADER]: '100.64.0.7' }
    expect(peerAddress('127.0.0.1', relayed)).toBe('100.64.0.7')
    expect(peerAddress('::ffff:127.0.0.1', relayed)).toBe('100.64.0.7')
    expect(peerAddress('100.64.0.9', relayed)).toBe('100.64.0.9')
    expect(peerAddress('127.0.0.1', {})).toBe('127.0.0.1')
    expect(peerAddress('127.0.0.1', { [VORN_PEER_HEADER]: '' })).toBe('127.0.0.1')
  })
})

describe('the relay', () => {
  it('carries frames both ways, text and bytes, in order', async () => {
    const vornd = await standInVornd()
    const client = await connect(await relayingServer(vornd.port))
    const got: Array<{ data: RawData; isBinary: boolean }> = []
    client.on('message', (data, isBinary) => got.push({ data, isBinary }))
    // Sent before vornd has answered the relay's upgrade: held, then sent in order.
    client.send('one')
    client.send(Buffer.from([1, 2, 3]))
    await vi.waitFor(() => expect(got).toHaveLength(2))
    expect(got[0]!.data.toString()).toBe('one')
    expect(got[0]!.isBinary).toBe(false)
    expect(got[1]!.isBinary).toBe(true)
    expect([...(got[1]!.data as Buffer)]).toEqual([1, 2, 3])
  })

  it('opens on vornd with what the client said and where it came from', async () => {
    const vornd = await standInVornd()
    await connect(await relayingServer(vornd.port), {
      Authorization: 'Bearer device-token',
      Origin: 'http://100.64.0.1:50091',
      Host: '100.64.0.1:50091',
      Cookie: 'a=b'
    })
    await vi.waitFor(() => expect(vornd.upgrades).toHaveLength(1))
    const seen = vornd.upgrades[0]!
    expect(seen.url).toBe('/ws?topics=sessions')
    expect(seen.headers.authorization).toBe('Bearer device-token')
    expect(seen.headers.origin).toBe('http://100.64.0.1:50091')
    expect(seen.headers.host).toBe('100.64.0.1:50091')
    expect(seen.headers.cookie).toBe('a=b')
    expect(seen.headers[VORN_PEER_HEADER]).toBe('100.64.0.7')
  })

  it('passes on the close the server gave, a rejected credential included', async () => {
    const vornd = await standInVornd((ws) => ws.close(4002, 'credential rejected'))
    const client = await connect(await relayingServer(vornd.port))
    expect(await closed(client)).toEqual({ code: 4002, reason: 'credential rejected' })
  })

  it('closes vornd’s side when the client leaves', async () => {
    const vornd = await standInVornd(() => {})
    const client = await connect(await relayingServer(vornd.port))
    await vi.waitFor(() => expect(vornd.sockets).toHaveLength(1))
    const gone = closed(vornd.sockets[0]!)
    client.close(1000, 'bye')
    expect((await gone).code).toBe(1000)
  })

  it('tells the client when vornd goes away under it', async () => {
    const vornd = await standInVornd(() => {})
    const client = await connect(await relayingServer(vornd.port))
    await vi.waitFor(() => expect(vornd.sockets).toHaveLength(1))
    const ended = closed(client)
    vornd.sockets[0]!.terminate()
    expect((await ended).code).toBeGreaterThan(0)
  })

  it('refuses the client when the server refused the upgrade', async () => {
    const server = http.createServer()
    server.on('upgrade', (_req, socket) => {
      socket.end('HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n')
    })
    const client = await connect(await relayingServer(await listening(server)))
    expect(await closed(client)).toEqual({ code: 1008, reason: 'forbidden' })
  })

  it('serves the client here, with what it already sent, when vornd is not there', async () => {
    // A port nothing listens on, reserved and given back.
    const free = http.createServer()
    const port = await listening(free)
    free.close()
    const seen: string[] = []
    const client = await connect(
      await relayingServer(port, (ws) => {
        ws.on('message', (data) => seen.push(data.toString()))
        ws.send('served here')
      })
    )
    const reply = next(client)
    client.send('early')
    expect((await reply).data.toString()).toBe('served here')
    await vi.waitFor(() => expect(seen).toEqual(['early']))
  })

  it('serves the client here when what answers is not vornd', async () => {
    const server = http.createServer((_req, res) => res.end('not a websocket'))
    server.on('upgrade', (_req, socket) => {
      socket.end('HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n')
    })
    const client = await connect(await relayingServer(await listening(server)))
    expect((await next(client)).data.toString()).toBe('served here')
  })
})
