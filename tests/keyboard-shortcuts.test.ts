import { describe, it, expect, vi } from 'vitest'

// navigator must exist before the module evaluates its top-level const.
// vi.stubGlobal is NOT hoisted, so we use vi.hoisted to run before imports.
// navigator is a read-only getter in Node, so we must use defineProperty.
vi.hoisted(() => {
  Object.defineProperty(globalThis, 'navigator', {
    value: { platform: 'MacIntel' },
    writable: true,
    configurable: true
  })
})

import {
  SHORTCUTS,
  SHORTCUT_CATEGORIES,
  getShortcut
} from '../packages/web/src/renderer/lib/keyboard-shortcuts'

describe('SHORTCUTS', () => {
  it('has no duplicate ids', () => {
    const ids = SHORTCUTS.map((s) => s.id)
    expect(new Set(ids).size).toBe(ids.length)
  })

  it('every shortcut has required fields', () => {
    for (const s of SHORTCUTS) {
      expect(s.id).toBeTruthy()
      expect(s.key).toBeTruthy()
      expect(s.display).toBeTruthy()
    }
  })

  it('every shortcut has a category', () => {
    const validCategories = SHORTCUT_CATEGORIES.map((c) => c.key)
    for (const s of SHORTCUTS) {
      expect(validCategories).toContain(s.category)
    }
  })
})

describe('SHORTCUT_CATEGORIES', () => {
  it('covers all 4 categories', () => {
    expect(SHORTCUT_CATEGORIES.map((c) => c.key).sort()).toEqual([
      'filter',
      'navigation',
      'sessions',
      'view'
    ])
  })
})

describe('getShortcut', () => {
  it('returns correct shortcut for known id', () => {
    const s = getShortcut('new-session')
    expect(s).toBeDefined()
    expect(s!.key).toBe('n')
  })

  it('returns undefined for unknown id', () => {
    expect(getShortcut('nonexistent')).toBeUndefined()
  })
})
