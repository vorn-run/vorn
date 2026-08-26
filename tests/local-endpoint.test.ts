import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import WebSocket from 'ws'
import { openLocalEndpoint, type LocalEndpoint } from '../packages/server/src/local-endpoint'
import { endpointUrl } from '../src/main/server/server-adoption'

/**
 * Bringing up the endpoint, in this process.
 *
 * `endpoint-race.process.test.ts` proves the behaviour with real spawned servers,
 * which is the only way to prove a handover. It cannot exercise this module's own
 * branches — a separate process is a separate world — so the wiring is tested
 * here: what happens when the name is free, when it is taken, when the directory
 * will not have it, and whether letting go actually lets go.
 */

let dir: string
const open: LocalEndpoint[] = []

beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-local-'))
  fs.chmodSync(dir, 0o700)
})

afterEach(async () => {
  for (const endpoint of open.splice(0)) await endpoint.close()
  fs.rmSync(dir, { recursive: true, force: true })
})

async function hold(onUpgraded = (): void => {}): Promise<LocalEndpoint> {
  const outcome = await openLocalEndpoint(dir, onUpgraded)
  if (outcome.kind !== 'held') throw new Error(`expected to hold it, got ${outcome.kind}`)
  open.push(outcome.endpoint)
  return outcome.endpoint
}

describe('bringing up the endpoint', () => {
  it('claims a free name and serves it', async () => {
    const endpoint = await hold()

    expect(endpoint.path).toBe(path.join(dir, 'vorn.sock'))
    expect(fs.lstatSync(endpoint.path).isSocket()).toBe(true)
    expect(endpoint.holds()).toBe(true)
  })

  it('leaves no scratch name behind', async () => {
    await hold()
    // Every name this process created is its own to remove, and it removes them.
    // What is left is the endpoint and nothing else -- there is no sweeper, so a
    // leak here would be permanent.
    expect(fs.readdirSync(dir)).toEqual(['vorn.sock'])
  })

  it('stands down when the name is already held', async () => {
    await hold()
    const second = await openLocalEndpoint(dir, () => {})

    expect(second.kind).toBe('lost')
    if (second.kind === 'lost') expect(second.because).toContain('alive')
  })

  it('carries on without one where a socket cannot go', async () => {
    // A path too long for `sun_path`. No failure either way: the socket is
    // additive, and a server nobody can reach would be worse than a race nobody
    // has lost yet.
    const deep = path.join(dir, 'z'.repeat(90))
    fs.mkdirSync(deep, { recursive: true, mode: 0o700 })
    const outcome = await openLocalEndpoint(deep, () => {})

    expect(outcome.kind).toBe('unavailable')
    expect(fs.existsSync(path.join(deep, 'vorn.sock'))).toBe(false)
  })
})

describe('what the endpoint answers', () => {
  it('accepts a websocket and greets it', async () => {
    const upgraded = vi.fn()
    const endpoint = await hold(upgraded)

    const ws = new WebSocket(endpointUrl(endpoint.path))
    const greeting = await new Promise<string>((resolve, reject) => {
      ws.once('message', (raw: Buffer) => resolve(raw.toString()))
      ws.once('error', reject)
    })

    expect(JSON.parse(greeting)).toMatchObject({ method: expect.any(String) })
    expect(upgraded).toHaveBeenCalled()
    ws.close()
  })

  it.each(['/somewhere-else', '/ws-and-more', '/wsx', '/'])(
    'refuses the route %s',
    async (route) => {
      // `/ws-and-more` is the one worth naming: a `startsWith` check accepted it,
      // which is a wider door than a listener carrying one route means to open.
      const endpoint = await hold()

      const ws = new WebSocket(`ws+unix://${endpoint.path}:${route}`)
      await expect(
        new Promise((resolve, reject) => {
          ws.once('open', resolve)
          ws.once('error', reject)
        })
      ).rejects.toThrow()
    }
  )

  it('accepts the route with a query on it', async () => {
    const endpoint = await hold()

    const ws = new WebSocket(`ws+unix://${endpoint.path}:/ws?topics=all`)
    await expect(
      new Promise((resolve, reject) => {
        ws.once('open', resolve)
        ws.once('error', reject)
      })
    ).resolves.toBeUndefined()
    ws.close()
  })
})

describe('letting go', () => {
  it('closes even while a client is still attached', async () => {
    // The bug this pins: `ws` does not terminate tracked clients on close, it
    // waits for them, and so does the http server behind it. A half-open client
    // -- the case the idle watch's duration clock exists to tolerate -- would
    // hold this for ever, stalling shutdown into its deadline and leaving on
    // exit(1) after killAll had already run.
    const endpoint = await hold()
    const ws = new WebSocket(endpointUrl(endpoint.path))
    await new Promise((resolve, reject) => {
      ws.once('open', resolve)
      ws.once('error', reject)
    })

    await expect(
      Promise.race([
        endpoint.close().then(() => 'closed'),
        new Promise((r) => setTimeout(() => r('hung'), 5_000))
      ])
    ).resolves.toBe('closed')
    open.length = 0
  })

  it('leaves the endpoint on disk for the next publisher', async () => {
    const endpoint = await hold()
    await endpoint.close()
    open.length = 0

    // Never removed on the way out: this listener bound a scratch name that no
    // longer exists, so libuv's unlink at close has nothing to find. The dead
    // entry is what the next start replaces in one rename.
    expect(fs.lstatSync(endpoint.path).isSocket()).toBe(true)
  })

  it('stops claiming to hold a name that became somebody else’s', async () => {
    const endpoint = await hold()
    expect(endpoint.holds()).toBe(true)

    // Another server renames its own socket into place, which is exactly what a
    // claim does. Asked rather than remembered, so this notices.
    const usurper = path.join(dir, 'other.sock')
    const { default: net } = await import('node:net')
    const server = net.createServer()
    await new Promise<void>((resolve) => server.listen(usurper, resolve))
    fs.renameSync(usurper, endpoint.path)

    expect(endpoint.holds()).toBe(false)
    await new Promise<void>((resolve) => server.close(() => resolve()))
  })
})
