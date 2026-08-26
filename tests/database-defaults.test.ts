import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
vi.mock('node:fs', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return { ...actual, existsSync: vi.fn(() => true), mkdirSync: vi.fn() }
})

import {
  initTestDatabase,
  loadConfig,
  saveConfig,
  dbInsertSourceConnection,
  dbInsertTaskSourceLink,
  dbGetTaskSourceLink
} from '../packages/server/src/database'
import type { AppConfig } from '../packages/shared/src/types'

/**
 * Saving iterates every key in `defaults`, but loading rebuilds the object
 * from an explicit list. A setting missing from that list writes to the
 * database and then vanishes on read — the toggle in Settings flips back on
 * its own, with nothing in any log to explain it.
 */

let close: (() => void) | null = null

beforeEach(() => {
  close = initTestDatabase()
})

afterEach(() => {
  close?.()
  close = null
})

function configWith(defaults: Partial<AppConfig['defaults']>): AppConfig {
  return {
    version: 1,
    defaults: { shell: '/bin/zsh', fontSize: 13, theme: 'dark', ...defaults },
    projects: []
  } as AppConfig
}

describe('defaults survive a save/load round trip', () => {
  it.each([
    ['domBlockRendering', true],
    ['domBlockRendering', false],
    ['minimalShellPrompt', true],
    ['minimalShellPrompt', false],
    ['reopenSessions', true],
    ['keepSessionsRunning', true],
    ['keepSessionsRunning', false],
    ['widgetEnabled', false]
  ])('%s = %s', (key, value) => {
    saveConfig(configWith({ [key]: value } as Partial<AppConfig['defaults']>))
    const loaded = loadConfig()
    expect(loaded.defaults[key as keyof AppConfig['defaults']]).toBe(value)
  })

  it('defaults the terminal settings on when the user has not chosen', () => {
    saveConfig(configWith({}))
    const loaded = loadConfig()
    expect(loaded.defaults.domBlockRendering).toBe(true)
    expect(loaded.defaults.minimalShellPrompt).toBe(true)
    expect(loaded.defaults.keepSessionsRunning).toBe(true)
  })

  it('keeps a false the user chose, rather than treating it as unset', () => {
    // The bug this guards: `?? true` on a stored `false` would silently turn
    // the setting back on every time it loaded. For keepSessionsRunning that
    // would mean quitting kept the agents alive after the user asked it not to.
    saveConfig(
      configWith({
        domBlockRendering: false,
        minimalShellPrompt: false,
        keepSessionsRunning: false
      })
    )
    const loaded = loadConfig()
    expect(loaded.defaults.domBlockRendering).toBe(false)
    expect(loaded.defaults.minimalShellPrompt).toBe(false)
    expect(loaded.defaults.keepSessionsRunning).toBe(false)
  })
})

describe('the four keys that were declared but never listed', () => {
  // Each was written on save and dropped on the next load, so the setting looked
  // like it worked until a reload. `worktreeRetention` is read server-side, so it
  // always resolved to undefined no matter what the user chose.
  it.each([
    ['updateAutoDownload', false],
    ['headlessStepTimeoutMinutes', 45],
    ['enableHoverPreview', false]
  ])('%s = %s', (key, value) => {
    saveConfig(configWith({ [key]: value } as Partial<AppConfig['defaults']>))

    expect(loadConfig().defaults[key as keyof AppConfig['defaults']]).toBe(value)
  })

  it('worktreeRetention, which the server reads', () => {
    const retention = { mode: 'days', days: 7 } as AppConfig['defaults']['worktreeRetention']
    saveConfig(configWith({ worktreeRetention: retention }))

    expect(loadConfig().defaults.worktreeRetention).toEqual(retention)
  })
})

describe('a key the saving client did not send', () => {
  it('survives, rather than being deleted', () => {
    // The save used to wipe the table and rewrite it from whatever object the
    // client held, so every key that client's build did not know about was
    // destroyed. This cost `serverPort` in practice: it exists so the web
    // client's origin survives a restart, and an ordinary settings save undid it.
    saveConfig(configWith({ serverPort: 61601 }))

    saveConfig(configWith({ theme: 'light' }))

    const loaded = loadConfig()
    expect(loaded.defaults.serverPort).toBe(61601)
    expect(loaded.defaults.theme).toBe('light')
  })

  it('survives even when the client is a whole release behind', () => {
    // The same failure with the gap widened: an older client connected to a newer
    // host would delete every setting its build predates.
    saveConfig(
      configWith({ serverPort: 61601, showHeadlessAgents: true, headlessRetentionMinutes: 30 })
    )

    saveConfig(configWith({}))

    const loaded = loadConfig()
    expect(loaded.defaults.serverPort).toBe(61601)
    expect(loaded.defaults.showHeadlessAgents).toBe(true)
    expect(loaded.defaults.headlessRetentionMinutes).toBe(30)
  })

  it('still lets a setting be cleared on purpose', () => {
    // "Absent means untouched" must not cost the ability to unset something, so an
    // explicit undefined still deletes.
    saveConfig(configWith({ widgetEnabled: true }))

    saveConfig(configWith({ widgetEnabled: undefined }))

    expect(loadConfig().defaults.widgetEnabled).toBeUndefined()
  })
})

describe('collections survive a save that did not carry them', () => {
  // The tables were wiped and rewritten from the client's snapshot, so a save
  // from one client deleted rows another had just added — and fired every
  // foreign-key cascade hanging off them on the way through.
  function withCollections(over: Partial<AppConfig>): AppConfig {
    return {
      version: 1,
      defaults: { shell: '/bin/zsh', fontSize: 13, theme: 'dark' },
      projects: [],
      ...over
    } as AppConfig
  }

  const task = (id: string, title: string) => ({
    id,
    projectName: 'vorn',
    title,
    description: '',
    status: 'todo',
    order: 0,
    createdAt: '2026-08-17T00:00:00.000Z',
    updatedAt: '2026-08-17T00:00:00.000Z'
  })

  it('updates a task without cascading away what references it', () => {
    // The real cost of wipe-and-rewrite, and the reason a title change looked
    // harmless: `task_source_links.task_id` is ON DELETE CASCADE, so deleting the
    // row severed the task from the external issue it was imported from. The link
    // never came back, and nothing said so.
    saveConfig(withCollections({ tasks: [task('t1', 'first')] } as Partial<AppConfig>))
    dbInsertSourceConnection({
      id: 'conn1',
      connectorId: 'github',
      name: 'repo',
      filters: {},
      syncIntervalMinutes: 15,
      statusMapping: {},
      createdAt: '2026-08-17T00:00:00.000Z'
    } as never)
    dbInsertTaskSourceLink({
      taskId: 't1',
      connectionId: 'conn1',
      connectorId: 'github',
      externalId: '42',
      externalUrl: 'https://example.test/42',
      sourceStatusRaw: 'open',
      sourceUpdatedAt: '2026-08-17T00:00:00.000Z',
      lastSyncedAt: '2026-08-17T00:00:00.000Z',
      conflictState: 'none'
    } as never)

    saveConfig(withCollections({ tasks: [task('t1', 'renamed')] } as Partial<AppConfig>))

    expect(loadConfig().tasks?.[0].title).toBe('renamed')
    expect(dbGetTaskSourceLink('t1')).not.toBeNull()
  })

  it('still removes a task the client actually dropped', () => {
    // Pruning has to stay real, or deleting a task would never take effect.
    saveConfig(
      withCollections({ tasks: [task('t1', 'first'), task('t2', 'second')] } as Partial<AppConfig>)
    )

    saveConfig(withCollections({ tasks: [task('t1', 'first')] } as Partial<AppConfig>))

    expect(loadConfig().tasks?.map((t) => t.id)).toEqual(['t1'])
  })

  it('keeps a workspace across an unrelated save', () => {
    const workspaces = [
      { id: 'personal', name: 'Personal', order: 0 },
      { id: 'work', name: 'Work', order: 1 }
    ]
    saveConfig(withCollections({ workspaces } as Partial<AppConfig>))

    saveConfig(withCollections({ workspaces } as Partial<AppConfig>))

    expect(
      loadConfig()
        .workspaces?.map((w) => w.id)
        .sort()
    ).toEqual(['personal', 'work'])
  })

  it('updates a project in place, since tasks reference it by name', () => {
    const project = { name: 'vorn', path: '/a', preferredAgents: [] }
    saveConfig(withCollections({ projects: [project] } as Partial<AppConfig>))

    saveConfig(withCollections({ projects: [{ ...project, path: '/b' }] } as Partial<AppConfig>))

    const loaded = loadConfig()
    expect(loaded.projects).toHaveLength(1)
    expect(loaded.projects[0].path).toBe('/b')
  })
})

describe('two clients saving against the same server', () => {
  // The failure this prevents: Vorn open on a laptop and a phone, both holding a
  // config loaded moments apart. Whichever saved second sent a snapshot that
  // predated the other's addition, and the prune removed it. No error, no log —
  // the task was simply gone.
  function base(over: Partial<AppConfig>): AppConfig {
    return {
      version: 1,
      defaults: { shell: '/bin/zsh', fontSize: 13, theme: 'dark' },
      projects: [],
      ...over
    } as AppConfig
  }

  const task = (id: string) => ({
    id,
    projectName: 'vorn',
    title: id,
    description: '',
    status: 'todo',
    order: 0,
    createdAt: '2026-08-17T00:00:00.000Z',
    updatedAt: '2026-08-17T00:00:00.000Z'
  })

  it('keeps a task added by the client that saved first', () => {
    saveConfig(base({ tasks: [task('shared')] } as Partial<AppConfig>))
    const laptop = loadConfig()
    const phone = loadConfig()

    // Phone adds a task and saves.
    saveConfig({ ...phone, tasks: [...(phone.tasks ?? []), task('from-phone')] })
    // Laptop saves a settings change from the snapshot it loaded earlier, which
    // knows nothing about the phone's task.
    saveConfig({ ...laptop, defaults: { ...laptop.defaults, fontSize: 15 } })

    const after = loadConfig()
    expect(after.tasks?.map((t) => t.id).sort()).toEqual(['from-phone', 'shared'])
    expect(after.defaults.fontSize).toBe(15)
  })

  it('still deletes a task the client actually removed', () => {
    // The other half: pruning has to keep working for a real deletion, or nothing
    // could ever be removed.
    saveConfig(base({ tasks: [task('a'), task('b')] } as Partial<AppConfig>))
    const client = loadConfig()

    saveConfig({ ...client, tasks: (client.tasks ?? []).filter((t) => t.id !== 'b') })

    expect(loadConfig().tasks?.map((t) => t.id)).toEqual(['a'])
  })

  it('prunes everything absent when the caller tracks no revision', () => {
    // The CLI, a test, or the server persisting its own port send no revision, and
    // must keep the old semantics rather than silently never deleting anything.
    saveConfig(base({ tasks: [task('a'), task('b')] } as Partial<AppConfig>))

    saveConfig(base({ tasks: [task('a')] } as Partial<AppConfig>))

    expect(loadConfig().tasks?.map((t) => t.id)).toEqual(['a'])
  })

  it('hands back a revision that moves forward', () => {
    saveConfig(base({}))
    const first = loadConfig().revision ?? 0

    saveConfig(base({}))

    expect(loadConfig().revision).toBeGreaterThan(first)
  })
})
