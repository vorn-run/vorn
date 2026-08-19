import fs from 'node:fs'
import { dirname, basename, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { containsPath, fileRootFor } from './browser-file-scope'
import log from './logger'

/**
 * Notice when the design a pane is showing changes on disk.
 *
 * This is the half of the loop that makes a design feel live: the agent writes
 * the file, and the pane repaints without anyone touching it. Everything else —
 * opening the file, reading its manifest, pointing at it — already worked.
 *
 * One watcher per session, aimed at whichever file that session's pane is
 * showing. The renderer says which, because it is the side that knows: it read
 * the manifest and decided the page is a design. Main does not go looking.
 *
 * Deliberately not a recursive watch on the session's root. A worktree can hold
 * a hundred thousand files, `fs.watch`'s recursive mode is unsupported on Linux,
 * and a design is one file — watching its own directory answers the only
 * question being asked.
 */

/** How long to let writes settle. An editor saving can fire several events. */
const DEBOUNCE_MS = 120

interface Watch {
  /** The file this session's pane is showing, absolute and resolved. */
  path: string
  /** The url the renderer named it by, reported back so the pane can match. */
  url: string
  watcher: fs.FSWatcher
  timer?: NodeJS.Timeout
}

const watches = new Map<string, Watch>()

/** Told when a watched file changes, so the pane can be repainted. */
type Notify = (sessionId: string, url: string) => void
let notify: Notify = () => {}
export function setArtifactNotify(fn: Notify): void {
  notify = fn
}

/**
 * Watch one file for a session, or stop watching.
 *
 * The path is checked against the session's own root before anything is opened.
 * The renderer is not a trust boundary — a bug or a compromised page that
 * reached this could otherwise aim a watcher at any directory on the machine
 * and learn when its contents changed.
 */
export function watchArtifact(sessionId: string, url: string | null): void {
  const existing = watches.get(sessionId)

  if (!url) {
    stopWatching(sessionId)
    return
  }

  // The renderer sends the url, not a path: it has no `fileURLToPath`, and a
  // hand-rolled conversion gets Windows drive letters wrong. Converting here
  // means the watcher and `allowsFileUrl` agree about what a url names.
  let path: string
  try {
    path = fileURLToPath(url)
  } catch {
    stopWatching(sessionId)
    return
  }

  const root = fileRootFor(sessionId)
  if (!root || !containsPath(root, path)) {
    // Same answer the pane gives for a file outside the root: nothing happens,
    // and the session simply has no watcher.
    stopWatching(sessionId)
    return
  }

  const target = resolve(path)
  // Already aimed here. Re-opening on every load would churn a descriptor per
  // repaint, and a design repaints often by design.
  if (existing?.path === target) return
  stopWatching(sessionId)

  const dir = dirname(target)
  const name = basename(target)
  try {
    const watcher = fs.watch(dir, (_event, filename) => {
      // A rename fires with the *new* name, and an atomic save (write temp,
      // rename over) is how most tools write — so this matches on the name
      // rather than assuming a change event for the file itself.
      if (filename && basename(filename) !== name) return
      const w = watches.get(sessionId)
      if (!w) return
      if (w.timer) clearTimeout(w.timer)
      w.timer = setTimeout(() => {
        // Existence is checked at fire time rather than at watch time: a save
        // that replaces the file leaves a moment where it is briefly absent,
        // and repainting a pane onto nothing is worse than skipping a beat.
        if (!fs.existsSync(w.path)) return
        notify(sessionId, w.url)
      }, DEBOUNCE_MS)
    })
    // A watcher whose directory is removed emits an error; without a handler
    // that reaches the process as an uncaught exception and takes main down.
    watcher.on('error', (err) => {
      log.debug({ err }, `[artifact] watch on ${dir} ended`)
      stopWatching(sessionId)
    })
    watches.set(sessionId, { path: target, url, watcher })
  } catch (err) {
    // The directory may not exist, or the platform may refuse another watch.
    // A design that does not repaint by itself is a smaller loss than a throw
    // out of an IPC handler.
    log.debug({ err }, `[artifact] could not watch ${dir}`)
  }
}

export function stopWatching(sessionId: string): void {
  const w = watches.get(sessionId)
  if (!w) return
  watches.delete(sessionId)
  if (w.timer) clearTimeout(w.timer)
  try {
    w.watcher.close()
  } catch {
    /* already gone */
  }
}

/** The file a session is watching, if any. Exported for tests. */
export function watchedPath(sessionId: string): string | null {
  return watches.get(sessionId)?.path ?? null
}

/** Only for tests, which must not leak descriptors between cases. */
export function stopAllWatching(): void {
  for (const id of [...watches.keys()]) stopWatching(id)
}
