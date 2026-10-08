/**
 * What vornd says of itself and this machine, through a real server and the
 * vornd it keeps: the core that answers, the PATH programs get, and why a
 * remote host cannot be logged in to. None of these calls reaches the server.
 *
 * Runs where vornd and its session holder have been built.
 */
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import {
  type RealServer,
  Watcher,
  removeRealServerDirs,
  runnable,
  startRealServer,
  stopRealServer
} from './helpers/real-server'

vi.setConfig({ testTimeout: 60_000, hookTimeout: 120_000 })

describe.runIf(runnable)('this machine, from vornd', () => {
  let server: RealServer
  let client: Watcher

  beforeAll(async () => {
    server = await startRealServer()
    client = await Watcher.open(server.vornd)
  })

  afterAll(async () => {
    client?.close()
    if (server) await stopRealServer(server)
    removeRealServerDirs()
  })

  it('says the core is loaded, whole', async () => {
    expect(await client.result('core:status')).toEqual({
      loaded: true,
      version: expect.any(String),
      error: null,
      missing: []
    })
  })

  it('answers the PATH programs get', async () => {
    const answer = await client.result<{ path: string | null; resolved: boolean }>('env:path')
    expect(answer.path).toEqual(expect.stringContaining('/'))
    expect(typeof answer.resolved).toBe('boolean')
  })

  it('says why a host cannot be logged in to', async () => {
    const tested = await client.result<{ success: boolean; message: string; durationMs: number }>(
      'ssh:testConnection',
      { id: 'h', label: 'Nowhere', hostname: 'nowhere.invalid', user: 'me', port: 22 }
    )
    expect(tested.success).toBe(false)
    expect(tested.message.length).toBeGreaterThan(0)
    expect(tested.durationMs).toBeGreaterThanOrEqual(0)
  })
})
