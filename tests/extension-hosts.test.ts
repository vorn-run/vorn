import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { InstalledConnectorPack } from '@vornrun/shared/types'
import type { SdkLaunch } from '../packages/server/src/connectors/sdk-client'

interface Started {
  launch: SdkLaunch & { name: string }
  key: string
  closed: boolean
  exit: () => void
}

const { started } = vi.hoisted(() => ({ started: [] as Started[] }))

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

vi.mock('../packages/server/src/connectors/sdk-client', async () => {
  const { createChildCache } = await vi.importActual<
    typeof import('../packages/server/src/connectors/stdio-clients')
  >('../packages/server/src/connectors/stdio-clients')
  const connectSdkClient = async (launch: Started['launch'], options: { key: string }) => {
    const listeners: Array<() => void> = []
    const child: Started = {
      launch,
      key: options.key,
      closed: false,
      exit: () => listeners.forEach((listener) => listener())
    }
    started.push(child)
    return {
      close: async () => {
        child.closed = true
      },
      onExit: (listener: () => void) => listeners.push(listener)
    }
  }
  return {
    connectSdkClient,
    createSdkChildCache: (label: string) =>
      createChildCache(label, (spawn: Started['launch']) =>
        connectSdkClient(spawn, { key: spawn.name })
      )
  }
})

const packs: InstalledConnectorPack[] = []

vi.mock('../packages/server/src/connectors/packs', () => ({
  installedPack: (id: string) => packs.find((pack) => pack.id === id),
  installedLaunch: (id: string) =>
    packs.some((pack) => pack.id === id)
      ? { command: 'node', args: [`/packs/${id}/index.js`], protocol: 1 }
      : undefined,
  listInstalledPacks: () => packs
}))

const hosts = await import('../packages/server/src/extensions/hosts')

function extension(id: string): InstalledConnectorPack {
  return {
    id,
    name: `${id} extension`,
    version: '0.1.0',
    kind: 'extension',
    protocol: 1,
    path: `/packs/${id}`,
    installedAt: 0,
    bytes: 0,
    triggers: [],
    actions: [],
    env: [],
    contributes: { footers: [{ id: 'checks', title: 'Checks', every: 30 }] },
    permissions: []
  }
}

beforeEach(() => {
  started.length = 0
  packs.length = 0
  packs.push(extension('review'))
  hosts.setExtensionBridgeOrigin('http://127.0.0.1:8931')
})

afterEach(async () => {
  await hosts.stopAllHosts()
})

describe('the child an extension runs as', () => {
  it('starts one per project as the installed pack, and gives it the bridge to talk back on', async () => {
    await hosts.getOrStartHost('review', '/work/vorn')
    expect(started).toHaveLength(1)
    const { launch, key } = started[0]
    expect(launch).toMatchObject({
      command: 'node',
      args: ['/packs/review/index.js'],
      source: 'pack',
      protocol: 1,
      cwd: '/work/vorn'
    })
    expect(launch.env.VORN_EXTENSION_HOST).toBe('http://127.0.0.1:8931/extensions/review/bridge')
    expect(launch.env.VORN_EXTENSION_TOKEN).toMatch(/^[\w-]{40,}$/)
    expect(Object.keys(launch.env)).toEqual(['VORN_EXTENSION_HOST', 'VORN_EXTENSION_TOKEN'])
    // What it says about itself names the extension, not the cache key.
    expect(key).toBe('review extension')
  })

  it('gives every extension its own token', async () => {
    packs.push(extension('checks'))
    await hosts.getOrStartHost('review', '/work/vorn')
    await hosts.getOrStartHost('checks', '/work/vorn')
    expect(hosts.tokenFor('review', '/work/vorn')).not.toBe(hosts.tokenFor('checks', '/work/vorn'))
  })

  it('shares one child between the sessions of a project', async () => {
    await hosts.getOrStartHost('review', '/work/vorn')
    await hosts.getOrStartHost('review', '/work/vorn')
    expect(started).toHaveLength(1)
  })

  // Recorded before anything suspends, so a second session joins the startup rather than racing it.
  it('shares one child between two sessions starting at once', async () => {
    await Promise.all([
      hosts.getOrStartHost('review', '/work/vorn'),
      hosts.getOrStartHost('review', '/work/vorn')
    ])
    expect(started).toHaveLength(1)
  })

  it('starts a second child for a second project', async () => {
    await hosts.getOrStartHost('review', '/work/vorn')
    await hosts.getOrStartHost('review', '/work/other')
    expect(started).toHaveLength(2)
    expect(hosts.isRunning('review', '/work/other')).toBe(true)
  })

  it('answers a call by the token the child holds, and nothing else', async () => {
    await hosts.getOrStartHost('review', '/work/vorn')
    const token = hosts.tokenFor('review', '/work/vorn') as string
    expect(hosts.hostByToken('review', token)?.projectPath).toBe('/work/vorn')
    expect(hosts.hostByToken('review', 'made-up')).toBeUndefined()
    expect(hosts.hostByToken('checks', token)).toBeUndefined()
  })

  it('stops what a removed pack was running', async () => {
    await hosts.getOrStartHost('review', '/work/vorn')
    await hosts.getOrStartHost('review', '/work/other')
    await hosts.stopHostsForExtension('review')
    expect(hosts.isRunning('review', '/work/vorn')).toBe(false)
    expect(hosts.isRunning('review', '/work/other')).toBe(false)
    expect(started.every((child) => child.closed)).toBe(true)
  })

  it('stops what a project was running when its last session goes', async () => {
    await hosts.getOrStartHost('review', '/work/vorn')
    await hosts.stopHostsForProject('/work/vorn')
    expect(hosts.isRunning('review', '/work/vorn')).toBe(false)
  })

  it('restarts after the child exits on its own', async () => {
    await hosts.getOrStartHost('review', '/work/vorn')
    started[0].exit()
    expect(hosts.isRunning('review', '/work/vorn')).toBe(false)
    await hosts.getOrStartHost('review', '/work/vorn')
    expect(started).toHaveLength(2)
  })

  it('runs nothing that is not an installed extension', async () => {
    await expect(hosts.getOrStartHost('missing', '/work/vorn')).rejects.toThrow(/No extension/)
    packs.push({ ...extension('poller'), kind: 'connector' })
    await expect(hosts.getOrStartHost('poller', '/work/vorn')).rejects.toThrow(/No extension/)
  })

  it('lists the installed extensions, and no connectors', () => {
    packs.push({ ...extension('poller'), kind: 'connector' })
    expect(hosts.installedExtensions().map((pack) => pack.id)).toEqual(['review'])
  })
})
