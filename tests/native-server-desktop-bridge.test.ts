/**
 * The desktop bridge on a real server and its vornd: main claims its socket
 * with `bridge:identify`, once; an agent's `browser:*` and `device:*` calls
 * are answered by the Browser and Device built-ins, which refuse them while
 * switched off, and none of them reaches the server or is relayed to main.
 *
 * Runs where vornd and vorn-sessiond have been built (`yarn build:core`), on
 * a Unix.
 */
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
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
    const answer = { error: { code: -32000, message: `no ${frame.method}` } }
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

  it('lets main claim its socket once', async () => {
    expect((await identify(main.ws)).result).toEqual({ ok: true })
    const other = await openMain(server.vornd)
    expect((await identify(other.ws)).result).toEqual({ ok: false })
    other.ws.close()
  })

  it("answers an agent's browser and device calls in the built-ins, not in main", async () => {
    for (const method of ['browser:tabs', 'device:list']) {
      const frame = await agent.call(method, { sessionId: 's' })
      expect(frame.error).toMatchObject({ message: 'No session s in this app.' })
    }
    expect(main.asked.map((f) => f.method)).not.toContain('browser:tabs')
    expect(main.asked.map((f) => f.method)).not.toContain('device:list')
  })

  it('refuses them while the built-ins are off', async () => {
    for (const [id, method, name] of [
      ['vorn.browser', 'browser:tabs', 'Browser'],
      ['vorn.device', 'device:claim', 'Device']
    ]) {
      await agent.result('extension:setBuiltin', { id, on: false })
      const frame = await agent.call(method, { sessionId: 's' })
      expect(frame.error).toMatchObject({
        message: `${name} is off. Turn it on in Settings \u2192 Extensions.`
      })
      await agent.result('extension:setBuiltin', { id, on: true })
    }
  })

  it('frees the bridge once main has gone', async () => {
    const closed = new Promise((resolve) => main.ws.once('close', resolve))
    main.ws.close()
    await closed
    const next = await openMain(server.vornd)
    await until('main to be let go', async () => {
      const claimed = (await identify(next.ws)).result as { ok?: boolean } | undefined
      return claimed?.ok === true
    })
    next.ws.close()
  })
})
