/**
 * The configuration, answered by vornd through a real server and the vornd it keeps.
 *
 * A save through vornd is what every client loads next, from vornd or from a
 * client of the server (which hands the call to vornd), and every client is
 * told of it once. A save from a client that has not seen a row written since
 * does not delete it. Each viewer keeps its own view settings, and none of
 * these calls reaches the server.
 *
 * Runs where vornd and its session holder have been built.
 */
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import type { AppConfig, ProjectConfig } from '../packages/shared/src/types'
import {
  type RealServer,
  Watcher,
  removeRealServerDirs,
  runnable,
  startRealServer,
  stopRealServer,
  until
} from './helpers/real-server'

vi.setConfig({ testTimeout: 60_000, hookTimeout: 120_000 })

interface Health {
  groups: Record<string, { forwarded?: number; native?: number }>
  unexpectedForwards: Record<string, number>
  stillForwarded: Record<string, string>
}

describe.runIf(runnable)('the configuration in vornd', () => {
  let server: RealServer
  let viaVornd: Watcher
  let direct: Watcher

  const health = async (): Promise<Health> =>
    (await (await fetch(`http://127.0.0.1:${server.vornd}/vornd/health`)).json()) as Health

  const project = (name: string): ProjectConfig =>
    ({ name, path: `/tmp/${name}`, preferredAgents: ['claude'] }) as ProjectConfig

  beforeAll(async () => {
    server = await startRealServer()
    viaVornd = await Watcher.open(server.vornd)
    direct = await Watcher.open(server.port)
  })

  afterAll(async () => {
    viaVornd?.close()
    direct?.close()
    if (server) await stopRealServer(server)
    removeRealServerDirs()
  })

  it('round-trips a save to every client, and tells each of them once', async () => {
    const before = await viaVornd.result<AppConfig>('config:load')
    expect(await direct.result<AppConfig>('config:load')).toEqual(before)
    const heard = direct.toldBy('config:changed').length
    const heardViaVornd = viaVornd.toldBy('config:changed').length

    const next: AppConfig = {
      ...before,
      defaults: { ...before.defaults, shell: '/bin/sh', minimalShellPrompt: false },
      projects: [...before.projects, project('cfg-a')]
    }
    expect(await viaVornd.call('config:save', next)).not.toHaveProperty('error')

    const loaded = await direct.result<AppConfig>('config:load')
    expect(loaded.defaults.shell).toBe('/bin/sh')
    expect(loaded.defaults.minimalShellPrompt).toBe(false)
    expect(loaded.projects.map((p) => p.name)).toContain('cfg-a')
    expect(loaded.revision).toBeGreaterThan(before.revision ?? 0)

    await until('both clients to be told', () => {
      return (
        direct.toldBy('config:changed').length > heard &&
        viaVornd.toldBy('config:changed').length > heardViaVornd
      )
    })
    // The server's watcher debounces its own signal; a second tell would come within it.
    await new Promise((r) => setTimeout(r, 600))
    const told = direct.toldBy('config:changed').slice(heard) as AppConfig[]
    expect(told).toHaveLength(1)
    expect(told[0].projects.map((p) => p.name)).toContain('cfg-a')
  })

  it('keeps a row written after the revision a stale client saved from', async () => {
    const stale = await viaVornd.result<AppConfig>('config:load')
    const fresh = await direct.result<AppConfig>('config:load')
    await direct.result('config:save', {
      ...fresh,
      projects: [...fresh.projects, project('cfg-b')]
    })
    // The stale client never saw cfg-b; its save leaves it alone.
    await viaVornd.result('config:save', {
      ...stale,
      defaults: { ...stale.defaults, fontSize: 15 }
    })
    const after = await viaVornd.result<AppConfig>('config:load')
    expect(after.projects.map((p) => p.name)).toEqual(expect.arrayContaining(['cfg-a', 'cfg-b']))
  })

  it('answers a save that is not a configuration with the store’s refusal', async () => {
    const frame = await viaVornd.call('config:save', { defaults: {} })
    expect(frame.error).toMatchObject({ code: -32000 })
  })

  it('keeps each viewer’s own view settings', async () => {
    const minted = await viaVornd.result<{ plaintext: string }>('token:create', { name: 'phone' })
    const phone = await Watcher.open(server.vornd, minted.plaintext)
    try {
      const desktopView = await viaVornd.result<AppConfig>('config:load')
      await viaVornd.result('config:save', {
        ...desktopView,
        defaults: { ...desktopView.defaults, theme: 'light', mainViewMode: 'tasks' }
      })
      const phoneView = await phone.result<AppConfig>('config:load')
      await phone.result('config:save', {
        ...phoneView,
        defaults: { ...phoneView.defaults, theme: 'dark', mainViewMode: 'sessions' }
      })
      const desktopAgain = await viaVornd.result<AppConfig>('config:load')
      const phoneAgain = await phone.result<AppConfig>('config:load')
      expect(desktopAgain.defaults.theme).toBe('light')
      expect(desktopAgain.defaults.mainViewMode).toBe('tasks')
      expect(phoneAgain.defaults.theme).toBe('dark')
      expect(phoneAgain.defaults.mainViewMode).toBe('sessions')
    } finally {
      phone.close()
    }
  })

  it('never hands a configuration call to the server', async () => {
    const { groups, unexpectedForwards, stillForwarded } = await health()
    expect(groups.config?.forwarded ?? 0).toBe(0)
    expect(groups.config?.native ?? 0).toBeGreaterThan(0)
    expect(unexpectedForwards).toEqual({})
    expect(stillForwarded.extension).toBeUndefined()
    expect(stillForwarded.browser).toBe('extension host')
    expect(stillForwarded.config).toBeUndefined()
  })
})
