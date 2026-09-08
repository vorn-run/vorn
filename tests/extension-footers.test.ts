import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { IPC } from '@vornrun/shared/types'
import type { InstalledConnectorPack, TerminalSession } from '@vornrun/shared/types'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

const broadcasts: Array<{ method: string; params: unknown; scope?: string }> = []
vi.mock('../packages/server/src/broadcast', () => ({
  clientRegistry: {
    broadcast: (method: string, params: unknown, scope?: string) =>
      broadcasts.push({ method, params, scope }),
    size: 1
  }
}))

const packs: InstalledConnectorPack[] = []
const calls: Array<{ name: string; arguments: unknown }> = []
let answer: () => unknown = () => ({ structuredContent: { items: [] } })

vi.mock('../packages/server/src/extensions/hosts', () => ({
  installedExtensions: () => packs,
  getOrStartHost: async () => ({
    callTool: async (request: { name: string; arguments: unknown }) => {
      calls.push(request)
      return answer()
    }
  })
}))

const footers = await import('../packages/server/src/extensions/footers')

const temps: string[] = []
let worktreePath: string

function extension(over: Partial<InstalledConnectorPack> = {}): InstalledConnectorPack {
  return {
    id: 'checks',
    name: 'Checks',
    version: '0.1.0',
    kind: 'extension',
    path: '/packs/checks',
    installedAt: 0,
    bytes: 0,
    triggers: [],
    actions: [],
    env: [],
    contributes: { footers: [{ id: 'checks', title: 'Checks', every: 5 }] },
    permissions: ['terminal.read'],
    ...over
  }
}

function session(): TerminalSession {
  return {
    id: 's1',
    agentType: 'claude',
    projectName: 'vorn',
    projectPath: worktreePath,
    worktreePath,
    status: 'running',
    createdAt: 0,
    pid: 1
  } as TerminalSession
}

/** Let the interval's first run, which starts before the timer, settle. */
const settle = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0))

beforeEach(() => {
  worktreePath = mkdtempSync(join(tmpdir(), 'vorn-footers-'))
  temps.push(worktreePath)
  mkdirSync(worktreePath, { recursive: true })
  writeFileSync(join(worktreePath, 'package.json'), '{}')
  packs.length = 0
  packs.push(extension())
  calls.length = 0
  broadcasts.length = 0
  answer = () => ({ structuredContent: { items: [{ label: 'tests', value: '345 passed' }] } })
})

afterEach(() => {
  footers.stopAllFooters()
  while (temps.length > 0) rmSync(temps.pop() as string, { recursive: true, force: true })
})

describe('a footer band', () => {
  it('computes once as soon as it shows, rather than after its first interval', async () => {
    footers.syncFooters(session())
    await settle()
    expect(calls).toHaveLength(1)
    expect(calls[0].name).toBe('vorn_footer_checks')
    expect(calls[0].arguments).toMatchObject({
      sessionId: 's1',
      worktreePath,
      agent: 'claude'
    })
    expect(footers.footerReadings('s1')[0]).toMatchObject({
      extensionId: 'checks',
      footerId: 'checks',
      items: [{ label: 'tests', value: '345 passed' }]
    })
  })

  it('tells the windows drawing the card, scoped to it', async () => {
    footers.syncFooters(session())
    await settle()
    const pushed = broadcasts.filter((one) => one.method === IPC.EXTENSION_FOOTER_ITEMS)
    expect(pushed).toHaveLength(1)
    expect(pushed[0].scope).toBe('s1')
  })

  it('says nothing when the reading has not moved', async () => {
    footers.syncFooters(session())
    await settle()
    broadcasts.length = 0
    footers.stopFooters('s1')
    footers.syncFooters(session())
    await settle()
    // Recomputed from scratch after a stop, so the first reading is new again.
    expect(broadcasts).toHaveLength(1)
    broadcasts.length = 0
    footers.syncFooters(session())
    await settle()
    expect(broadcasts).toHaveLength(0)
  })

  it('keeps the last good reading beside the error when a run fails', async () => {
    footers.syncFooters(session())
    await settle()
    answer = () => {
      throw new Error('git is not on the path')
    }
    footers.stopFooters('s1')
    footers.syncFooters(session())
    await settle()
    answer = () => {
      throw new Error('git is not on the path')
    }
    footers.syncFooters(session())
    await settle()
    const reading = footers.footerReadings('s1')[0]
    expect(reading.error).toContain('git is not on the path')
  })

  it('refuses items a band cannot draw', async () => {
    answer = () => ({ structuredContent: { items: [{ label: 'tests' }] } })
    footers.syncFooters(session())
    await settle()
    expect(footers.footerReadings('s1')[0].error).toContain('label and a value')
  })

  it('refuses an item linking somewhere a click should not go', async () => {
    answer = () => ({
      structuredContent: {
        items: [{ label: 'ci', value: 'green', href: 'javascript:alert(1)' }]
      }
    })
    footers.syncFooters(session())
    await settle()
    expect(footers.footerReadings('s1')[0].error).toBeDefined()
  })

  it('draws nothing for a footer whose rule excludes this session', async () => {
    packs[0] = extension({
      contributes: {
        footers: [
          { id: 'checks', title: 'Checks', every: 5, when: { workspaceContains: ['Cargo.toml'] } }
        ]
      }
    })
    footers.syncFooters(session())
    await settle()
    expect(calls).toHaveLength(0)
    expect(footers.footerReadings('s1')).toEqual([])
  })

  it('stops what a session no longer shows, and what it leaves behind', async () => {
    footers.syncFooters(session())
    await settle()
    expect(footers.footerReadings('s1')).toHaveLength(1)
    packs.length = 0
    footers.syncFooters(session())
    expect(footers.footerReadings('s1')).toEqual([])
  })

  it('holds everything for a session until it ends', async () => {
    footers.syncFooters(session())
    await settle()
    footers.stopFooters('s1')
    expect(footers.footerReadings('s1')).toEqual([])
  })

  // Otherwise the reading arrives after the card is gone and nothing ever clears it.
  it('says nothing about a session that ended while it was computing', async () => {
    let release: (() => void) | undefined
    answer = () =>
      new Promise((resolve) => {
        release = () =>
          resolve({ structuredContent: { items: [{ label: 'tests', value: 'passing' }] } })
      })
    footers.syncFooters(session())
    await settle()
    footers.stopFooters('s1')
    broadcasts.length = 0

    release?.()
    await settle()
    expect(footers.footerReadings('s1')).toEqual([])
    expect(broadcasts.filter((one) => one.method === IPC.EXTENSION_FOOTER_ITEMS)).toEqual([])
  })

  // An upgraded pack is a different footer; leaving the old timer would keep the old interval.
  it('starts again when the pack it came from changed', async () => {
    footers.syncFooters(session())
    await settle()
    expect(calls).toHaveLength(1)

    // Same footer, same interval, so nothing should be restarted.
    footers.syncFooters(session())
    await settle()
    expect(calls).toHaveLength(1)

    packs[0] = extension({
      version: '0.2.0',
      contributes: { footers: [{ id: 'checks', title: 'Checks', every: 30 }] }
    })
    footers.syncFooters(session())
    await settle()
    expect(calls).toHaveLength(2)
  })
})
