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
