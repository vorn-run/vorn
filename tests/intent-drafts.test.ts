// @vitest-environment jsdom
import { describe, it, expect, beforeEach } from 'vitest'
import {
  readIntentDraft,
  writeIntentDraft,
  forgetIntentDraft,
  pruneIntentDrafts
} from '../src/renderer/lib/intent-drafts'

beforeEach(() => localStorage.clear())

describe('a command typed and not sent', () => {
  it('comes back for the same session', () => {
    writeIntentDraft('s1', 'git rebase main')
    expect(readIntentDraft('s1')?.text).toBe('git rebase main')
    expect(readIntentDraft('s2')).toBeNull()
  })

  it('is forgotten when emptied, so a sent command leaves nothing behind', () => {
    writeIntentDraft('s1', 'ls')
    writeIntentDraft('s1', '')
    expect(readIntentDraft('s1')).toBeNull()
  })

  it('can be discarded outright', () => {
    writeIntentDraft('s1', 'ls')
    forgetIntentDraft('s1')
    expect(readIntentDraft('s1')).toBeNull()
  })

  it('goes with its session', () => {
    writeIntentDraft('gone', 'ls')
    writeIntentDraft('here', 'pwd')
    pruneIntentDrafts(new Set(['here']))
    expect(readIntentDraft('gone')).toBeNull()
    expect(readIntentDraft('here')?.text).toBe('pwd')
  })

  it('treats a corrupt store as empty rather than throwing at render', () => {
    localStorage.setItem('vorn:intentDrafts', '{not json')
    expect(readIntentDraft('s1')).toBeNull()
    localStorage.setItem('vorn:intentDrafts', JSON.stringify({ s1: { text: 42 } }))
    expect(readIntentDraft('s1')).toBeNull()
  })
})
