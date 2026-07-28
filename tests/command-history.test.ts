// @vitest-environment jsdom
import { describe, it, expect, beforeEach } from 'vitest'

// Replace Node's experimental localStorage (which needs a file path)
// with an in-memory shim so getItem/setItem/clear work in tests.
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
  recordCommand,
  searchHistory,
  ghostSuggestion,
  resetCommandHistoryCache
} from '../src/renderer/lib/command-history'

describe('command-history', () => {
  beforeEach(() => {
    localStorage.clear()
    resetCommandHistoryCache()
  })

  it('records commands most-recent-first and persists to localStorage', () => {
    recordCommand('git status', 'shell', '/p/a')
    recordCommand('yarn test', 'shell', '/p/a')
    const results = searchHistory('', 'shell', '/p/a')
    expect(results.map((r) => r.text)).toEqual(['yarn test', 'git status'])
    expect(localStorage.getItem('vorn:commandHistory')).toContain('yarn test')
  })

  it('ignores empty or whitespace-only input', () => {
    recordCommand('   ', 'shell')
    expect(searchHistory('', 'shell')).toHaveLength(0)
  })

  it('moves a re-submitted command to the front instead of duplicating', () => {
    recordCommand('git status', 'shell')
    recordCommand('yarn test', 'shell')
    recordCommand('git status', 'shell')
    const results = searchHistory('', 'shell')
    expect(results.map((r) => r.text)).toEqual(['git status', 'yarn test'])
  })

  it('separates shell and agent history', () => {
    recordCommand('git status', 'shell')
    recordCommand('fix the failing tests', 'agent')
    expect(searchHistory('', 'shell').map((r) => r.text)).toEqual(['git status'])
    expect(searchHistory('', 'agent').map((r) => r.text)).toEqual(['fix the failing tests'])
  })

  it('ranks prefix matches above substring and subsequence matches', () => {
    recordCommand('echo git', 'shell') // substring
    recordCommand('grep -rn install thing', 'shell') // subsequence of "git"? g..i..t yes
    recordCommand('git status', 'shell') // prefix
    const results = searchHistory('git', 'shell')
    expect(results[0].text).toBe('git status')
    expect(results[1].text).toBe('echo git')
  })

  it('boosts same-project entries', () => {
    recordCommand('yarn build', 'shell', '/p/other')
    recordCommand('yarn bootstrap', 'shell', '/p/mine')
    // Later entry is more recent, but both are prefix matches; the
    // same-project boost outweighs recency across projects.
    recordCommand('yarn bench', 'shell', '/p/other')
    const results = searchHistory('yarn', 'shell', '/p/mine')
    expect(results[0].text).toBe('yarn bootstrap')
  })

  it('excludes an exact match from suggestions', () => {
    recordCommand('git status', 'shell')
    expect(searchHistory('git status', 'shell')).toHaveLength(0)
  })

  it('ghost suggestion completes from most recent prefix match', () => {
    recordCommand('git push origin main', 'shell')
    recordCommand('git pull --rebase', 'shell')
    expect(ghostSuggestion('git pu', 'shell')).toBe('git pull --rebase')
  })

  it('ghost suggestion prefers the active project', () => {
    recordCommand('yarn test --watch', 'shell', '/p/other')
    recordCommand('yarn typecheck', 'shell', '/p/mine')
    expect(ghostSuggestion('yarn t', 'shell', '/p/mine')).toBe('yarn typecheck')
  })

  it('returns no ghost for empty or multiline input', () => {
    recordCommand('git status', 'shell')
    expect(ghostSuggestion('', 'shell')).toBeNull()
    expect(ghostSuggestion('git\nsta', 'shell')).toBeNull()
  })

  it('caps stored history at 1000 entries', () => {
    for (let i = 0; i < 1010; i++) {
      recordCommand(`cmd-${i}`, 'shell')
    }
    const raw = JSON.parse(localStorage.getItem('vorn:commandHistory') ?? '[]')
    expect(raw).toHaveLength(1000)
    expect(raw[0].text).toBe('cmd-1009')
  })
})
