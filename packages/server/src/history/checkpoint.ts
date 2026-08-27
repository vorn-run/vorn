import fs from 'fs'
import path from 'path'
import crypto from 'crypto'
import log from '../logger'

/**
 * The state a terminal can be restored to, written so a crash cannot half-write
 * it.
 *
 * A checkpoint is the expensive half of the pair: the log is appended to
 * constantly and this is written rarely, so this is where the durability
 * discipline goes. Three steps, and the third is the one implementations forget:
 *
 *   1. Write a scratch file beside the target.
 *   2. `rename` it into place — atomic, so a reader sees the old file or the new
 *      one and never a half of either.
 *   3. **fsync the parent directory.** The rename is atomic with respect to
 *      other processes; it is not durable with respect to power loss until the
 *      directory entry itself has been synced. Skip it and the atomic rename
 *      buys nothing against the failure it was chosen for.
 *
 * Nothing else in this repo has fsynced anything, so step three is new ground
 * and is the reason this module exists rather than a `writeFileSync` at the call
 * site. The rename shape follows `endpoint.ts`, which uses the same
 * scratch-then-rename idiom for a socket.
 */

export interface Checkpoint {
  /** Escape sequences that reproduce the screen. */
  screen: string
  /** The bytes behind it, which carry scrollback the screen model does not. */
  scrollback: string
  cols: number
  rows: number
  title: string
  cwd: string
  /** Ties the log beside it to this checkpoint. */
  generation: number
  /** The last batch this includes. Replay starts after it. */
  seq: number
}

export const CHECKPOINT_FILE = 'checkpoint.json'
export const LOG_FILE = 'output.log'

/**
 * How large a checkpoint may be before it is skipped.
 *
 * Skipped whole rather than truncated: half a JSON document is not a smaller
 * checkpoint, it is an unparseable one, and a session whose screen will not fit
 * is better off restoring from an older checkpoint plus a longer log than from
 * something that cannot be read at all.
 */
export const MAX_CHECKPOINT_BYTES = 2 * 1024 * 1024

/**
 * Where a session's history lives.
 *
 * Under a subdirectory of the data dir, never in it: `config-manager` puts an
 * `fs.watch` on the data directory itself, and although it filters by filename
 * the callback still fires per event -- an append-only log written beside the
 * database would wake that watcher on every chunk of terminal output.
 *
 * The id is encoded because it ends up as a path segment, and a session id is
 * not guaranteed to be one.
 */
export function historyDir(dataDir: string, sessionId: string): string {
  return path.join(dataDir, 'history', encodeURIComponent(sessionId))
}

/**
 * Make a directory's contents durable.
 *
 * A rename is atomic but not durable: the entry can still be lost to power
 * failure until the directory itself is synced. Best-effort, because some
 * filesystems refuse to open a directory for this and failing to sync is not a
 * reason to fail the write -- but it is a reason to say so once.
 */
function trySync(fd: number, what: string): void {
  try {
    fs.fsyncSync(fd)
  } catch (err) {
    log.warn({ err, what }, '[history] could not fsync; this write may not survive power loss')
  }
}

function syncDir(dir: string): void {
  let fd: number
  try {
    fd = fs.openSync(dir, 'r')
  } catch (err) {
    log.warn({ err, dir }, '[history] could not open the directory to fsync it')
    return
  }
  try {
    trySync(fd, 'the history directory')
  } finally {
    try {
      fs.closeSync(fd)
    } catch {
      /* already closed */
    }
  }
}

/** Written, renamed, and synced. Returns whether it landed. */
export function writeCheckpoint(dir: string, checkpoint: Checkpoint): boolean {
  const body = JSON.stringify(checkpoint)
  if (Buffer.byteLength(body) > MAX_CHECKPOINT_BYTES) {
    log.warn(
      { dir, bytes: Buffer.byteLength(body) },
      '[history] screen too large to checkpoint; skipping this one whole'
    )
    return false
  }

  // Random rather than a fixed `.tmp`, so two writers -- which should not exist,
  // but a second server on one data dir is a thing this codebase has had to
  // reason about before -- cannot land on each other's scratch file.
  const scratch = path.join(dir, `.${CHECKPOINT_FILE}.${crypto.randomBytes(6).toString('hex')}`)
  const target = path.join(dir, CHECKPOINT_FILE)

  try {
    fs.mkdirSync(dir, { recursive: true, mode: 0o700 })
    const fd = fs.openSync(scratch, 'w', 0o600)
    try {
      fs.writeFileSync(fd, body, 'utf-8')
      // The file's own contents, before the rename makes them reachable.
      // Best-effort, like the directory below: a filesystem that refuses to sync
      // still gives an atomic rename against other readers, and a checkpoint
      // that is merely not power-loss-durable beats no checkpoint at all.
      trySync(fd, 'the checkpoint file')
    } finally {
      fs.closeSync(fd)
    }
    fs.renameSync(scratch, target)
    syncDir(dir)
    return true
  } catch (err) {
    log.warn({ err, dir }, '[history] could not write a checkpoint')
    try {
      fs.rmSync(scratch, { force: true })
    } catch {
      /* a scratch file this process created and could not remove */
    }
    return false
  }
}

/**
 * Read a checkpoint, or answer null.
 *
 * Null for absent, unreadable, unparseable and wrong-shaped alike, because the
 * caller's response to all of them is identical: there is nothing to restore
 * from, so start clean. A checkpoint that fails to parse is not worth a throw --
 * it is the ordinary residue of a crash.
 */
export function readCheckpoint(dir: string): Checkpoint | null {
  try {
    const raw: unknown = JSON.parse(fs.readFileSync(path.join(dir, CHECKPOINT_FILE), 'utf-8'))
    return isCheckpoint(raw) ? raw : null
  } catch {
    return null
  }
}

function isCheckpoint(value: unknown): value is Checkpoint {
  if (!value || typeof value !== 'object') return false
  const v = value as Partial<Checkpoint>
  return (
    typeof v.screen === 'string' &&
    typeof v.scrollback === 'string' &&
    Number.isInteger(v.cols) &&
    Number.isInteger(v.rows) &&
    typeof v.title === 'string' &&
    typeof v.cwd === 'string' &&
    Number.isInteger(v.generation) &&
    Number.isInteger(v.seq)
  )
}
