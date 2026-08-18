import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
vi.mock('node:fs', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return { ...actual, existsSync: vi.fn(() => true), mkdirSync: vi.fn() }
})

import { initTestDatabase, loadConfig, saveConfig } from '../packages/server/src/database'
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
  })

  it('keeps a false the user chose, rather than treating it as unset', () => {
    // The bug this guards: `?? true` on a stored `false` would silently turn
    // the setting back on every time it loaded.
    saveConfig(configWith({ domBlockRendering: false, minimalShellPrompt: false }))
    const loaded = loadConfig()
    expect(loaded.defaults.domBlockRendering).toBe(false)
    expect(loaded.defaults.minimalShellPrompt).toBe(false)
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
