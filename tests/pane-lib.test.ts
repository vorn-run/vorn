// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import {
  filesPaneId,
  editorPaneId,
  browserPaneId,
  devicePaneId,
  paneIdFor,
  panesOwnedBy,
  parsePaneId,
  paneKind,
  paneOwnerId,
  isTerminalPane
} from '../src/renderer/lib/pane-id'
import {
  dirtyRefFor,
  isEditorDirty,
  clearDirty,
  confirmDiscard
} from '../src/renderer/lib/editor-dirty'

/**
 * Pane ids are opaque strings everywhere except here — the grid, dock, tab strip
 * and layout persistence all just pass them around. That only holds if the
 * prefix scheme round-trips exactly, so these assertions pin it down.
 */
describe('pane-id', () => {
  it('builds and parses child pane ids', () => {
    expect(filesPaneId('abc')).toBe('files:abc')
    expect(editorPaneId('abc')).toBe('editor:abc')
    expect(parsePaneId('files:abc')).toEqual({ kind: 'files', sessionId: 'abc' })
    expect(parsePaneId('editor:abc')).toEqual({ kind: 'editor', sessionId: 'abc' })
    expect(browserPaneId('abc')).toBe('browser:abc')
    expect(parsePaneId('browser:abc')).toEqual({ kind: 'browser', sessionId: 'abc' })
  })

  it('treats a bare terminal id as its own owner', () => {
    // Callers ask "which session owns this pane" without knowing the kind, so a
    // terminal has to answer with itself rather than null.
    expect(parsePaneId('abc')).toEqual({ kind: 'terminal', sessionId: 'abc' })
    expect(paneOwnerId('abc')).toBe('abc')
    expect(paneOwnerId('files:abc')).toBe('abc')
    expect(paneOwnerId('editor:abc')).toBe('abc')
    expect(paneOwnerId('browser:abc')).toBe('abc')
  })

  it('reports kind without allocating the owner string', () => {
    expect(paneKind('files:abc')).toBe('files')
    expect(paneKind('editor:abc')).toBe('editor')
    expect(paneKind('browser:abc')).toBe('browser')
    expect(paneKind('abc')).toBe('terminal')
  })

  it('distinguishes session panes from child panes', () => {
    expect(isTerminalPane('abc')).toBe(true)
    expect(isTerminalPane('files:abc')).toBe(false)
    expect(isTerminalPane('editor:abc')).toBe(false)
    expect(isTerminalPane('browser:abc')).toBe(false)
  })

  it('builds a pane id from a kind held as data', () => {
    // The pane column carries kinds, not ids, so it needs the inverse of
    // parsePaneId — and the two have to agree, or a promoted pane would be
    // skipped in the column under one id and drawn in the grid under another.
    for (const kind of ['files', 'editor', 'browser', 'device'] as const) {
      const id = paneIdFor(kind, 'abc')
      expect(parsePaneId(id)).toEqual({ kind, sessionId: 'abc' })
    }
    expect(paneIdFor('device', 'abc')).toBe(devicePaneId('abc'))
  })

  it('picks out the panes one session owns, and never a session itself', () => {
    // This is what places a promoted pane next to its owner in the grid. A
    // terminal id leaking through would place a session twice.
    const ids = ['files:a', 'editor:a', 'browser:b', 'a', 'b']
    expect(panesOwnedBy(ids, 'a')).toEqual(['files:a', 'editor:a'])
    expect(panesOwnedBy(ids, 'b')).toEqual(['browser:b'])
    expect(panesOwnedBy(ids, 'c')).toEqual([])
  })

  it('survives session ids that themselves contain a colon', () => {
    // Session ids come from the server; a colon in one must not be mistaken for
    // a pane prefix, or a session would silently render as somebody's file tree.
    const weird = 'host:1234'
    expect(parsePaneId(weird)).toEqual({ kind: 'terminal', sessionId: weird })
    expect(parsePaneId(filesPaneId(weird))).toEqual({ kind: 'files', sessionId: weird })
    expect(paneOwnerId(editorPaneId(weird))).toBe(weird)
    expect(paneOwnerId(browserPaneId(weird))).toBe(weird)
  })
})

/**
 * The editor lives in one pane while the actions that discard its buffer live in
 * others (picking a file in the tree, closing from the card header). This
 * registry is the only channel between them, so losing it means losing edits.
 */
describe('editor-dirty', () => {
  beforeEach(() => {
    clearDirty('s1')
    clearDirty('s2')
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('starts clean and tracks a session-scoped dirty flag', () => {
    expect(isEditorDirty('s1')).toBe(false)
    dirtyRefFor('s1').current = true
    expect(isEditorDirty('s1')).toBe(true)
    // Sessions are independent: one dirty editor must not block another's.
    expect(isEditorDirty('s2')).toBe(false)
  })

  it('returns the same ref for a session so the editor can keep it in sync', () => {
    expect(dirtyRefFor('s1')).toBe(dirtyRefFor('s1'))
    expect(dirtyRefFor('s1')).not.toBe(dirtyRefFor('s2'))
  })

  it('clears a flag', () => {
    dirtyRefFor('s1').current = true
    clearDirty('s1')
    expect(isEditorDirty('s1')).toBe(false)
  })

  it('proceeds without prompting when the buffer is clean', () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false)
    expect(confirmDiscard('s1')).toBe(true)
    expect(confirm).not.toHaveBeenCalled()
  })

  it('prompts when dirty and proceeds only on confirmation', () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true)
    dirtyRefFor('s1').current = true

    expect(confirmDiscard('s1')).toBe(true)
    expect(confirm).toHaveBeenCalledOnce()
    // Confirming discards the buffer, so the flag must not linger and prompt
    // again on the next action.
    expect(isEditorDirty('s1')).toBe(false)
  })

  it('blocks and keeps the buffer when the user cancels', () => {
    vi.spyOn(window, 'confirm').mockReturnValue(false)
    dirtyRefFor('s1').current = true

    expect(confirmDiscard('s1')).toBe(false)
    expect(isEditorDirty('s1')).toBe(true)
  })
})
