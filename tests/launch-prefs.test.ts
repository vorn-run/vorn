// @vitest-environment jsdom
import { describe, it, expect, beforeEach } from 'vitest'

// Node's experimental localStorage needs a file path; swap in an in-memory
// shim, matching the other suites that touch storage.
{
  const store = new Map<string, string>()
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (k: string) => (store.has(k) ? store.get(k)! : null),
      setItem: (k: string, v: string) => store.set(k, String(v)),
      removeItem: (k: string) => void store.delete(k),
      clear: () => store.clear(),
      key: (i: number) => Array.from(store.keys())[i] ?? null,
      get length() {
        return store.size
      }
    }
  })
}
import {
  getPreferredAgent,
  loadLaunchSettings,
  setPreferredAgent
} from '../src/renderer/lib/launch-prefs'

const KEY = 'vorn:lastLaunchSettings'

describe('launch-prefs', () => {
  beforeEach(() => localStorage.clear())

  it('round-trips the preferred agent', () => {
    setPreferredAgent('codex')
    expect(getPreferredAgent()).toBe('codex')
  })

  it('falls back when nothing is stored', () => {
    expect(getPreferredAgent('gemini')).toBe('gemini')
  })

  it.each([
    ['a number', '5'],
    ['a string', '"claude"'],
    ['null', 'null'],
    ['an array', '["claude"]'],
    ['malformed json', '{not json']
  ])('ignores stored %s rather than spreading it back', (_label, raw) => {
    // Valid JSON is not necessarily the shape we wrote; spreading a string
    // would persist indexed characters back into storage.
    localStorage.setItem(KEY, raw)
    expect(loadLaunchSettings()).toEqual({})
    setPreferredAgent('claude')
    const stored = JSON.parse(localStorage.getItem(KEY) as string)
    expect(stored).toEqual({ agent: 'claude' })
  })
})
