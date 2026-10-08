/**
 * The desktop bridge on a real server and its vornd: main claims its socket
 * with `bridge:identify`, an agent's `browser:*` and `device:*` calls reach
 * main through vornd and come back, and none of them reaches the server.
 *
 * Runs where vornd and vorn-sessiond have been built (`yarn build:core`, or
 * the binaries in `VORN_CONFORMANCE_VORND`), on a Unix.
 */
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import WebSocket from 'ws'
import { spawnsRealServers } from './helpers/one-at-a-time'
import {
  TEST_CREDENTIAL,
  Watcher,
  realServers,
  removeRealServerDirs,
  runnable,
  startRealServer,
  stopRealServer,
  until,
  type RealServer
} from './helpers/real-server'

vi.mock('../packages/server/src/tailscale', () => ({
  getTailscaleStatus: vi.fn(async () => ({ running: false, selfIP: '', selfDNSName: '' })),
  clearBinaryCache: vi.fn()
}))

spawnsRealServers()

interface Frame {
  id?: string | number
  method?: string
  params?: unknown
  result?: unknown
  error?: { message: string }
}

/** The desktop's main process: answers what it is asked, as `ServerBridge` does. */
async function openMain(port: number): Promise<{ ws: WebSocket; asked: Frame[] }> {
  const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`, {
    headers: { authorization: `Bearer ${TEST_CREDENTIAL}` }
  })
  const asked: Frame[] = []
  ws.on('message', (raw, isBinary) => {
    if (isBinary) return
    const frame = JSON.parse(String(raw)) as Frame
    if (!frame.method || frame.id === undefined) return
    asked.push(frame)
    const answer =
      frame.method === 'browser:tabs'
        ? { result: [{ id: 't1', params: frame.params }] }
        : { error: { code: -32000, message: `no ${frame.method}` } }
    ws.send(JSON.stringify({ jsonrpc: '2.0', id: frame.id, ...answer }))
  })
  await new Promise((resolve, reject) => {
    ws.once('open', resolve)
    ws.once('error', reject)
  })
  return { ws, asked }
}

function identify(ws: WebSocket): Promise<Frame> {
  return new Promise((resolve) => {
    const onMessage = (raw: WebSocket.RawData, isBinary: boolean): void => {
      if (isBinary) return
      const frame = JSON.parse(String(raw)) as Frame
      if (frame.id !== 'claim') return
      ws.off('message', onMessage)
      resolve(frame)
    }
    ws.on('message', onMessage)
    ws.send(JSON.stringify({ jsonrpc: '2.0', id: 'claim', method: 'bridge:identify' }))
  })
}

describe.skipIf(!runnable)('the desktop bridge through vornd', () => {
  let server: RealServer
  let agent: Watcher
  let main: { ws: WebSocket; asked: Frame[] }

  beforeAll(async () => {
    try {
      server = await startRealServer()
      main = await openMain(server.vornd)
      agent = await Watcher.open(server.vornd)
    } catch (err) {
      const log = (server ?? realServers.at(-1))?.log.join('') ?? ''
      throw new Error(`${(err as Error).message}\n${log.slice(-4000)}`, { cause: err })
    }
  }, 240_000)

  afterAll(async () => {
    agent?.close()
    main?.ws.close()
    if (server) await stopRealServer(server)
    removeRealServerDirs()
  }, 120_000)

  it('answers no browser call before main claims its socket', async () => {
    const frame = await agent.call('browser:tabs', { sessionId: 's' })
    expect(frame.error).toMatchObject({
      message: 'Vorn app is not running (no main process connected)'
    })
  })

  it('lets main claim its socket once', async () => {
    expect((await identify(main.ws)).result).toEqual({ ok: true })
    const other = await openMain(server.vornd)
    expect((await identify(other.ws)).result).toEqual({ ok: false })
    other.ws.close()
  })

  it("relays an agent's calls to main and its answers back", async () => {
    expect(await agent.result('browser:tabs', { sessionId: 's' })).toEqual([
      { id: 't1', params: { sessionId: 's' } }
    ])
    const frame = await agent.call('device:list')
    expect(frame.error).toMatchObject({ message: 'no device:list' })
    expect(main.asked.map((f) => [typeof f.id, f.method])).toEqual([
      ['string', 'browser:tabs'],
      ['string', 'device:list']
    ])
  })

  it('answers them all in vornd', async () => {
    const health = (await (
      await fetch(`http://127.0.0.1:${server.vornd}/vornd/health`)
    ).json()) as { groups: Record<string, { native?: number; forwarded?: number }> }
    for (const group of ['bridge', 'browser', 'device']) {
      expect({ group, forwarded: health.groups[group]?.forwarded ?? 0 }).toEqual({
        group,
        forwarded: 0
      })
      expect(health.groups[group]?.native).toBeGreaterThan(0)
    }
  })

  it('fails the calls once main has gone', async () => {
    const closed = new Promise((resolve) => main.ws.once('close', resolve))
    main.ws.close()
    await closed
    const notRunning = 'Vorn app is not running (no main process connected)'
    let message: string | undefined
    await until('main to be let go', async () => {
      const frame = await agent.call('browser:tabs', { sessionId: 's' })
      message = (frame.error as { message?: string } | undefined)?.message
      return message === notRunning
    })
    expect(message).toBe(notRunning)
  })
})
