// @vitest-environment jsdom
import { describe, it, expect, beforeEach } from 'vitest'
import {
  forgetDraft,
  hasMoved,
  pruneDrafts,
  readDraft,
  writeDraft
} from '../src/renderer/lib/editor-drafts'

const KEY = 'vorn:drafts'
const BASE = { size: 120, mtimeMs: 1_700_000_000_000 }

beforeEach(() => {
  localStorage.clear()
})

describe('an edit that was never saved', () => {
  it('comes back for the pane that was holding it', () => {
    writeDraft('editor:term-1', { filePath: '/repo/a.ts', text: 'edited', base: BASE })
    expect(readDraft('editor:term-1', '/repo/a.ts')?.text).toBe('edited')
  })

  it('is not offered for a different file in the same pane', () => {
    writeDraft('editor:term-1', { filePath: '/repo/a.ts', text: 'edited', base: BASE })
    expect(readDraft('editor:term-1', '/repo/b.ts')).toBeNull()
  })

  it('is not shared between two panes on one file', () => {
    writeDraft('editor:term-1', { filePath: '/repo/a.ts', text: 'mine', base: BASE })
    expect(readDraft('card:term-1:2', '/repo/a.ts')).toBeNull()
  })

  it('is let go of once it has been saved or discarded', () => {
    writeDraft('editor:term-1', { filePath: '/repo/a.ts', text: 'edited', base: BASE })
    forgetDraft('editor:term-1')
    expect(readDraft('editor:term-1', '/repo/a.ts')).toBeNull()
  })

  it('remembers what the file was when the edit started', () => {
    writeDraft('editor:term-1', { filePath: '/repo/a.ts', text: 'edited', base: BASE })
    expect(readDraft('editor:term-1', '/repo/a.ts')?.base).toEqual(BASE)
  })

  it('goes when its pane does', () => {
    writeDraft('editor:term-1', { filePath: '/repo/a.ts', text: 'a', base: BASE })
    writeDraft('card:term-2:1', { filePath: '/repo/b.ts', text: 'b', base: BASE })
    pruneDrafts(new Set(['editor:term-1']))
    expect(readDraft('editor:term-1', '/repo/a.ts')).not.toBeNull()
    expect(readDraft('card:term-2:1', '/repo/b.ts')).toBeNull()
  })
})

describe('reading a record that has gone wrong', () => {
  it('survives a value that is not the shape it expects', () => {
    for (const raw of ['{', '[]', 'null', '"x"', '{"p":{"text":"t"}}', '{"p":{"filePath":"/a"}}']) {
      localStorage.setItem(KEY, raw)
      expect(() => readDraft('p', '/a')).not.toThrow()
      expect(readDraft('p', '/a')).toBeNull()
    }
  })

  it('keeps a draft whose stamp is unusable, without the stamp', () => {
    localStorage.setItem(
      KEY,
      JSON.stringify({ p: { filePath: '/a', text: 't', base: { size: 'big' } } })
    )
    expect(readDraft('p', '/a')).toEqual({ filePath: '/a', text: 't', base: null, savedAt: 0 })
  })

  it('keeps an empty draft, which is a real edit', () => {
    writeDraft('p', { filePath: '/a', text: '', base: BASE })
    expect(readDraft('p', '/a')?.text).toBe('')
  })
})

describe('deciding whether the file moved under the draft', () => {
  it('says no when it is byte for byte the same', () => {
    expect(hasMoved(BASE, { ...BASE })).toBe(false)
  })

  it('says yes on a different size', () => {
    expect(hasMoved(BASE, { ...BASE, size: 121 })).toBe(true)
  })

  it('says yes on a different mtime, which is how a same-length edit shows up', () => {
    expect(hasMoved(BASE, { ...BASE, mtimeMs: BASE.mtimeMs + 1000 })).toBe(true)
  })

  it('says yes when the file can no longer be stamped at all', () => {
    expect(hasMoved(BASE, null)).toBe(true)
  })

  it('says no when nothing stamped the file to begin with', () => {
    // Otherwise every save on a surface that cannot stamp becomes a conflict.
    expect(hasMoved(null, null)).toBe(false)
    expect(hasMoved(null, BASE)).toBe(false)
  })
})
