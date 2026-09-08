import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { InstalledConnectorPack } from '@vornrun/shared/types'

const transports: Array<{ opts: Record<string, unknown>; onclose?: () => void }> = []
const closed: string[] = []

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

vi.mock('@modelcontextprotocol/sdk/client/index.js', () => ({
  Client: class {
    private transport: { opts: Record<string, unknown> } | undefined
    async connect(transport: { opts: Record<string, unknown> }): Promise<void> {
      this.transport = transport
    }
    async close(): Promise<void> {
      closed.push(String(this.transport?.opts.args))
    }
  }
}))

vi.mock('@modelcontextprotocol/sdk/client/stdio.js', () => ({
  StdioClientTransport: class {
    readonly opts: Record<string, unknown>
    onclose: (() => void) | undefined
    onerror: ((err: unknown) => void) | undefined
    constructor(opts: Record<string, unknown>) {
      this.opts = opts
      transports.push(this as never)
    }
    async close(): Promise<void> {}
  }
}))

const packs: InstalledConnectorPack[] = []

vi.mock('../packages/server/src/connectors/packs', () => ({
  installedPack: (id: string) => packs.find((pack) => pack.id === id),
  installedLaunch: (id: string) =>
    packs.some((pack) => pack.id === id)
      ? { command: 'node', args: [`/packs/${id}/index.js`] }
      : undefined,
  listInstalledPacks: () => packs
}))

vi.mock('../packages/server/src/process-utils', () => ({
  getSafeEnv: () => ({ PATH: '/usr/bin' })
}))

const hosts = await import('../packages/server/src/extensions/hosts')

function extension(id: string): InstalledConnectorPack {
  return {
    id,
    name: id,
    version: '0.1.0',
    kind: 'extension',
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
  transports.length = 0
  closed.length = 0
  packs.length = 0
  packs.push(extension('review'))
  hosts.setExtensionBridgeOrigin('http://127.0.0.1:8931')
})

afterEach(async () => {
  await hosts.stopAllHosts()
})

describe('the child an extension runs as', () => {
  it('starts one per project, and gives it the bridge to talk back on', async () => {
    await hosts.getOrStartHost('review', '/work/vorn')
    expect(transports).toHaveLength(1)
    const env = transports[0].opts.env as Record<string, string>
    expect(env.VORN_EXTENSION_HOST).toBe('http://127.0.0.1:8931/extensions/review/bridge')
    expect(env.VORN_EXTENSION_TOKEN).toMatch(/^[\w-]{40,}$/)
    expect(env.PATH).toBe('/usr/bin')
    expect(transports[0].opts.cwd).toBe('/work/vorn')
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
    expect(transports).toHaveLength(1)
  })

  // Recorded before anything suspends, so a second session joins the startup rather than racing it.
  it('shares one child between two sessions starting at once', async () => {
    await Promise.all([
      hosts.getOrStartHost('review', '/work/vorn'),
      hosts.getOrStartHost('review', '/work/vorn')
    ])
    expect(transports).toHaveLength(1)
  })

  it('starts a second child for a second project', async () => {
    await hosts.getOrStartHost('review', '/work/vorn')
    await hosts.getOrStartHost('review', '/work/other')
    expect(transports).toHaveLength(2)
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
    expect(closed).toHaveLength(2)
  })

  it('stops what a project was running when its last session goes', async () => {
    await hosts.getOrStartHost('review', '/work/vorn')
    await hosts.stopHostsForProject('/work/vorn')
    expect(hosts.isRunning('review', '/work/vorn')).toBe(false)
  })

  it('restarts after the child exits on its own', async () => {
    await hosts.getOrStartHost('review', '/work/vorn')
    transports[0].onclose?.()
    expect(hosts.isRunning('review', '/work/vorn')).toBe(false)
    await hosts.getOrStartHost('review', '/work/vorn')
    expect(transports).toHaveLength(2)
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
