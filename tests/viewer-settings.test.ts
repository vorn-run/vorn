import { describe, it, expect, beforeEach, vi } from 'vitest'
import type { AppConfig } from '../packages/shared/src/types'
import {
  VIEWER_SETTING_KEYS,
  applyViewerSettings,
  extractViewerSettings,
  isViewerSettingKey
} from '../packages/shared/src/config-scope'

/**
 * Settings used to be global to a server and broadcast to every client, so a
 * laptop and a phone pointed at one Vorn fought over the view mode, the font size
 * and the active workspace. Switching to Tasks on one switched it on the other.
 */

function config(defaults: Partial<AppConfig['defaults']>): AppConfig {
  return {
    version: 1,
    defaults: { shell: '/bin/zsh', fontSize: 13, theme: 'dark', ...defaults },
    projects: []
  } as AppConfig
}

describe('which settings belong to the viewer', () => {
  it('claims the ones only the renderer reads', () => {
    expect(isViewerSettingKey('mainViewMode')).toBe(true)
    expect(isViewerSettingKey('layoutMode')).toBe(true)
    expect(isViewerSettingKey('fontSize')).toBe(true)
    expect(isViewerSettingKey('activeWorkspace')).toBe(true)
  })

  it('leaves the ones the server acts on', () => {
    // `shell` and `minimalShellPrompt` are read when spawning a PTY;
    // `networkAccessEnabled` and `serverPort` decide the bind. Two clients
    // disagreeing about these would be a real conflict, not two right answers.
    expect(isViewerSettingKey('shell')).toBe(false)
    expect(isViewerSettingKey('minimalShellPrompt')).toBe(false)
    expect(isViewerSettingKey('networkAccessEnabled')).toBe(false)
    expect(isViewerSettingKey('serverPort')).toBe(false)
  })

  it('leaves defaultAgent shared', () => {
    // Read in the renderer, but it decides what gets spawned on the server, so it
    // describes the setup rather than the person looking at it.
    expect(isViewerSettingKey('defaultAgent')).toBe(false)
  })

  it('leaves the updater and widget to the machine that has them', () => {
    expect(isViewerSettingKey('updateChannel')).toBe(false)
    expect(isViewerSettingKey('widgetEnabled')).toBe(false)
  })
})

describe('overlaying a device onto a server', () => {
  it('lets this device win', () => {
    const merged = applyViewerSettings(config({ mainViewMode: 'sessions', fontSize: 13 }), {
      mainViewMode: 'tasks',
      fontSize: 18
    })

    expect(merged.defaults.mainViewMode).toBe('tasks')
    expect(merged.defaults.fontSize).toBe(18)
  })

  it('falls back to the server when the device has said nothing', () => {
    // What makes a first run seed itself rather than reset to a hardcoded default.
    const merged = applyViewerSettings(config({ mainViewMode: 'tasks' }), {})

    expect(merged.defaults.mainViewMode).toBe('tasks')
  })

  it('never touches a server-owned key', () => {
    const merged = applyViewerSettings(config({ shell: '/bin/bash', serverPort: 61601 }), {
      fontSize: 18
    } as never)

    expect(merged.defaults.shell).toBe('/bin/bash')
    expect(merged.defaults.serverPort).toBe(61601)
  })

  it('returns the config untouched when there is nothing stored', () => {
    const original = config({})

    expect(applyViewerSettings(original, {})).toBe(original)
  })
})

describe('extracting what to remember', () => {
  it('takes the viewer keys and nothing else', () => {
    const extracted = extractViewerSettings(
      config({ mainViewMode: 'tasks', fontSize: 18, shell: '/bin/bash', serverPort: 61601 })
    )

    expect(extracted).toEqual({ mainViewMode: 'tasks', fontSize: 18, theme: 'dark' })
  })

  it('skips a key the config does not carry', () => {
    const extracted = extractViewerSettings(config({}))

    expect(extracted).not.toHaveProperty('mainViewMode')
  })
})

describe('the device store', () => {
  beforeEach(() => {
    vi.resetModules()
    const store = new Map<string, string>()
    vi.stubGlobal('localStorage', {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => store.set(k, v),
      removeItem: (k: string) => store.delete(k)
    })
  })

  it('round-trips a save through to the next load', async () => {
    const { captureViewerSettings, withViewerSettings } =
      await import('../packages/shared/src/viewer-settings-store')

    captureViewerSettings(config({ mainViewMode: 'tasks', fontSize: 18 }))

    // The server still reports what some other client last wrote.
    const fromServer = config({ mainViewMode: 'sessions', fontSize: 13 })
    expect(withViewerSettings(fromServer).defaults.mainViewMode).toBe('tasks')
    expect(withViewerSettings(fromServer).defaults.fontSize).toBe(18)
  })

  it('keeps earlier keys when a later save carries fewer', async () => {
    const { captureViewerSettings, withViewerSettings } =
      await import('../packages/shared/src/viewer-settings-store')

    captureViewerSettings(config({ mainViewMode: 'tasks', fontSize: 18 }))
    // A config that genuinely lacks the key, rather than carrying a default for it.
    captureViewerSettings({
      version: 1,
      defaults: { shell: '/bin/zsh', mainViewMode: 'workflows' },
      projects: []
    } as unknown as AppConfig)

    const merged = withViewerSettings(config({}))
    expect(merged.defaults.mainViewMode).toBe('workflows')
    expect(merged.defaults.fontSize).toBe(18)
  })

  it('survives storage that refuses to write', async () => {
    // Private browsing throws on setItem. Forgetting a view preference is fine;
    // failing to start is not.
    vi.stubGlobal('localStorage', {
      getItem: () => null,
      setItem: () => {
        throw new Error('QuotaExceededError')
      },
      removeItem: () => {}
    })
    const { captureViewerSettings, withViewerSettings } =
      await import('../packages/shared/src/viewer-settings-store')

    expect(() => captureViewerSettings(config({ mainViewMode: 'tasks' }))).not.toThrow()
    expect(withViewerSettings(config({ mainViewMode: 'sessions' })).defaults.mainViewMode).toBe(
      'sessions'
    )
  })

  it('ignores stored junk rather than crashing the client', async () => {
    vi.stubGlobal('localStorage', {
      getItem: () => '["not", "an", "object"]',
      setItem: () => {},
      removeItem: () => {}
    })
    const { withViewerSettings } = await import('../packages/shared/src/viewer-settings-store')

    expect(withViewerSettings(config({ mainViewMode: 'tasks' })).defaults.mainViewMode).toBe(
      'tasks'
    )
  })

  it('covers every declared viewer key', () => {
    // A key added to the list but never stored would silently stay global.
    const all = Object.fromEntries(VIEWER_SETTING_KEYS.map((k) => [k, 'x'])) as never
    const extracted = extractViewerSettings(config(all))

    expect(Object.keys(extracted).sort()).toEqual([...VIEWER_SETTING_KEYS].sort())
  })
})

describe('device storage as untrusted input', () => {
  // localStorage is writable by any script on the origin and outlives the build
  // that wrote it, so what comes back is not necessarily what this build stored.
  // Spreading it unfiltered let it set `shell`, which is read when spawning a PTY,
  // and those values then round-tripped to the server because every save call site
  // sends the whole config.
  it('ignores server-owned keys in a crafted overlay', () => {
    const merged = applyViewerSettings(
      config({ shell: '/bin/zsh', networkAccessEnabled: false, serverPort: 61601 }),
      {
        mainViewMode: 'tasks',
        shell: '/bin/attacker',
        networkAccessEnabled: true,
        serverPort: 1
      } as never
    )

    expect(merged.defaults.shell).toBe('/bin/zsh')
    expect(merged.defaults.networkAccessEnabled).toBe(false)
    expect(merged.defaults.serverPort).toBe(61601)
    // The legitimate half still applies.
    expect(merged.defaults.mainViewMode).toBe('tasks')
  })

  it('drops unknown keys rather than passing them through', () => {
    const merged = applyViewerSettings(config({}), { somethingNew: 'x' } as never)

    expect(merged.defaults).not.toHaveProperty('somethingNew')
  })

  it('filters what it reads back from storage, not only what it merges', async () => {
    vi.stubGlobal('localStorage', {
      getItem: () =>
        JSON.stringify({ mainViewMode: 'tasks', shell: '/bin/attacker', serverPort: 1 }),
      setItem: () => {},
      removeItem: () => {}
    })
    const { readViewerSettings, withViewerSettings } =
      await import('../packages/shared/src/viewer-settings-store')

    expect(readViewerSettings()).toEqual({ mainViewMode: 'tasks' })
    const merged = withViewerSettings(config({ shell: '/bin/zsh', serverPort: 61601 }))
    expect(merged.defaults.shell).toBe('/bin/zsh')
    expect(merged.defaults.serverPort).toBe(61601)
  })
})
