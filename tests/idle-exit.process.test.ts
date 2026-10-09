import { describe, it, expect, afterEach } from 'vitest'
import fs from 'node:fs'
import WebSocket from 'ws'
import { spawnsRealServers } from './helpers/one-at-a-time'
import { builtVornd, startServed, type Served } from './helpers/served'

spawnsRealServers()

/**
 * vornd started as an app starts it, with `--idle-exit` and a short window:
 * it leaves once nothing uses it, and stays while a client keeps talking.
 */

const CREDENTIAL = 'idle-exit-credential'
let served: Served | null = null

afterEach(async () => {
  if (!served) return
  await served.stop()
  for (const dir of [served.dataDir, served.home]) fs.rmSync(dir, { recursive: true, force: true })
  served = null
})

const start = async (): Promise<Served> =>
  (served = await startServed({
    credential: CREDENTIAL,
    args: ['--idle-exit'],
    env: { VORN_IDLE_TIMEOUT_MS: '2000' }
  }))

const exited = (s: Served): boolean => s.child.exitCode !== null || s.child.signalCode !== null

describe.skipIf(!builtVornd)('a server with nothing to do', () => {
  it('exits on its own', async () => {
    const s = await start()
    const deadline = Date.now() + 20_000
    while (!exited(s) && Date.now() < deadline) await new Promise((r) => setTimeout(r, 250))
    expect(exited(s)).toBe(true)
  }, 40_000)

  it('stays up while a websocket client keeps sending frames', async () => {
    const s = await start()
    const ws = new WebSocket(`ws://127.0.0.1:${s.port}/ws`, {
      headers: { authorization: `Bearer ${CREDENTIAL}` }
    })
    await new Promise((resolve, reject) => {
      ws.once('open', resolve)
      ws.once('error', reject)
    })
    // Well past the window, sending the whole time.
    const deadline = Date.now() + 9_000
    let n = 0
    while (Date.now() < deadline) {
      ws.send(JSON.stringify({ jsonrpc: '2.0', id: ++n, method: 'config:load' }))
      await new Promise((r) => setTimeout(r, 500))
    }
    const stillUp = !exited(s)
    ws.close()
    expect(stillUp).toBe(true)
  }, 45_000)
})
