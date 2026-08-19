import { describe, it, expect, vi, beforeEach, afterAll } from 'vitest'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, realpathSync, renameSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'

vi.mock('./logger', () => ({ default: { debug: vi.fn(), warn: vi.fn(), info: vi.fn() } }))

import {
  watchArtifact,
  stopWatching,
  stopAllWatching,
  watchedPath,
  setArtifactNotify
} from '../src/main/artifact-watcher'
import { setFileRoot, resetFileRoots } from '../src/main/browser-file-scope'

/**
 * Noticing that a design changed.
 *
 * The failures worth testing are the ones that look like nothing happening: a
 * watcher aimed at a file outside the session, one that never fires because the
 * editor wrote through a temp file, and one left open after its pane closed. A
 * design that silently stops repainting reads as the feature being broken, not
 * as a watcher having quietly died.
 */

const tmp = realpathSync(mkdtempSync(join(tmpdir(), 'vorn-watch-')))
const project = join(tmp, 'project')
const outside = join(tmp, 'outside')
mkdirSync(project)
mkdirSync(outside)
mkdirSync(join(project, 'design'))

const DESIGN = join(project, 'design', 'budget.dc.html')
writeFileSync(DESIGN, '<h1>v1</h1>')
writeFileSync(join(outside, 'other.dc.html'), 'no')

afterAll(() => {
  stopAllWatching()
  rmSync(tmp, { recursive: true, force: true })
})

/**
 * The renderer names a design by url, not by path: it has no `fileURLToPath`,
 * and deriving one by hand gets Windows drive letters wrong.
 */
const asUrl = (p: string): string => pathToFileURL(p).href

/** Resolves with the url the watcher reported, or null if it stayed quiet. */
function nextChange(timeoutMs = 1200): Promise<string | null> {
  return new Promise((res) => {
    const timer = setTimeout(() => {
      setArtifactNotify(() => {})
      res(null)
    }, timeoutMs)
    setArtifactNotify((_sessionId, path) => {
      clearTimeout(timer)
      setArtifactNotify(() => {})
      res(path)
    })
  })
}

beforeEach(() => {
  stopAllWatching()
  resetFileRoots()
  setArtifactNotify(() => {})
  setFileRoot('sess', project)
})

describe('what a session may watch', () => {
  it('watches a design inside its own root', () => {
    watchArtifact('sess', asUrl(DESIGN))
    expect(watchedPath('sess')).toBe(DESIGN)
  })

  it('refuses a file outside the root', () => {
    // The renderer is not a trust boundary. A bug there could otherwise aim a
    // watcher anywhere on the machine and learn when it changed.
    watchArtifact('sess', asUrl(join(outside, 'other.dc.html')))
    expect(watchedPath('sess')).toBeNull()
  })

  it('refuses everything for a session with no root', () => {
    resetFileRoots()
    watchArtifact('sess', asUrl(DESIGN))
    expect(watchedPath('sess')).toBeNull()
  })

  it('stops when handed nothing, which is how a pane says it left', () => {
    watchArtifact('sess', asUrl(DESIGN))
    watchArtifact('sess', null)
    expect(watchedPath('sess')).toBeNull()
  })

  it('keeps one watcher when re-aimed at the same file', () => {
    // A design repaints often, and each repaint re-reads the manifest. Opening
    // a descriptor per repaint would leak one per paint.
    watchArtifact('sess', asUrl(DESIGN))
    watchArtifact('sess', asUrl(DESIGN))
    watchArtifact('sess', asUrl(DESIGN))
    expect(watchedPath('sess')).toBe(DESIGN)
  })

  it('follows the pane to another design', () => {
    const second = join(project, 'design', 'other.dc.html')
    writeFileSync(second, 'x')
    watchArtifact('sess', asUrl(DESIGN))
    watchArtifact('sess', asUrl(second))
    expect(watchedPath('sess')).toBe(second)
  })

  it('survives a directory that does not exist', () => {
    watchArtifact('sess', asUrl(join(project, 'nope', 'ghost.dc.html')))
    expect(watchedPath('sess')).toBeNull()
  })
})

describe('reporting a change', () => {
  it('reports an ordinary write', async () => {
    watchArtifact('sess', asUrl(DESIGN))
    const seen = nextChange()
    writeFileSync(DESIGN, '<h1>v2</h1>')
    expect(await seen).toBe(asUrl(DESIGN))
  })

  it('reports a save written through a temp file and renamed', async () => {
    // How most editors and many tools write. The rename fires under the *new*
    // name, so a watcher matching only on change events for the original would
    // never hear about the save that matters.
    watchArtifact('sess', asUrl(DESIGN))
    const seen = nextChange()
    const temp = `${DESIGN}.tmp`
    writeFileSync(temp, '<h1>v3</h1>')
    renameSync(temp, DESIGN)
    expect(await seen).toBe(asUrl(DESIGN))
  })

  it('stays quiet about a sibling in the same directory', async () => {
    // The whole directory is watched because that is what `fs.watch` offers;
    // reporting every file in it would repaint the pane for unrelated work.
    watchArtifact('sess', asUrl(DESIGN))
    const seen = nextChange(400)
    writeFileSync(join(project, 'design', 'unrelated.txt'), 'noise')
    expect(await seen).toBeNull()
  })

  it('stays quiet once its pane has gone', async () => {
    watchArtifact('sess', asUrl(DESIGN))
    stopWatching('sess')
    const seen = nextChange(400)
    writeFileSync(DESIGN, '<h1>v4</h1>')
    expect(await seen).toBeNull()
  })

  it('does not report a file that a save left absent', async () => {
    // A replace-in-place leaves a moment with no file. Repainting the pane onto
    // nothing is worse than skipping the beat and waiting for the write.
    const doomed = join(project, 'design', 'doomed.dc.html')
    writeFileSync(doomed, 'x')
    watchArtifact('sess', asUrl(doomed))
    const seen = nextChange(500)
    rmSync(doomed)
    expect(await seen).toBeNull()
  })
})
