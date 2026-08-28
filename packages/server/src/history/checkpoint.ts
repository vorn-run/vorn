import fs from 'fs'
import fsp from 'fs/promises'
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
 * scratch-then-rename idiom for a socket. The scratch *name* is spelled out here
 * rather than calling that module's `scratchPathFor`: it would couple the
 * history modules to the endpoint, which pulls in `net`, for two lines.
 *
 * The write is asynchronous throughout, and that is not a style preference. A
 * synchronous version measured 5.8 ms of blocked event loop per checkpoint on a
 * 460 KB screen -- fifty sessions in a row is a third of a second in which no
 * terminal output moves, no socket frame is read and no request is answered.
 * That would also have quietly undone the reason `writer.ts` gives a queue to
 * each session rather than one to the server: a slow disk under one terminal
 * would have held up every other terminal on the machine. Awaiting each step in
 * turn preserves the write-sync-rename-sync order exactly, and the per-session
 * queue is what guarantees no two writers meet in one directory.
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
  /**
   * The last batch on disk that this supersedes.
   *
   * Recovery does not read it, and that is not an oversight: a checkpoint
   * replaces the log rather than being written past it, so there is never a
   * prefix to skip. It is here because the generation says *which* log belongs
   * to this checkpoint and this says *how much* of one it stood in for, which is
   * the difference between a file somebody can diagnose and one they cannot.
   */
  seq: number
  /**
   * Whether this was written by a shutdown rather than by the clock.
   *
   * It is the difference between "you closed Vorn" and "something stopped it",
   * and a pane has no other way to tell: a crash runs nothing, so the last
   * checkpoint it leaves is an ordinary periodic one. Only the flush on the way
   * out sets this, which makes its absence the honest answer for every other
   * ending -- a kill, an OOM, a machine losing power.
   */
  closedCleanly?: boolean
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
 * not guaranteed to be one. Encoding is not on its own a guarantee, which is
 * worth saying because it reads like one: `encodeURIComponent('..')` is `'..'`,
 * so an id of two dots would name the data directory itself -- and `reset()`
 * begins by removing the directory it is given. Every id today is a
 * `crypto.randomUUID()`, so nothing reaches that; the check below is what makes
 * that a fact about this function rather than about its callers.
 */
export function historyDir(dataDir: string, sessionId: string): string {
  const root = path.join(dataDir, 'history')
  const at = path.join(root, encodeURIComponent(sessionId))
  if (path.dirname(at) !== root) {
    throw new Error(`a session id that does not name a directory under history: ${sessionId}`)
  }
  return at
}

/**
 * Make what a handle refers to durable.
 *
 * A rename is atomic but not durable: the entry can still be lost to power
 * failure until the directory itself is synced. Best-effort, because some
 * filesystems refuse to open a directory for this and failing to sync is not a
 * reason to fail the write -- but it is a reason to say so once.
 */
async function trySync(handle: fsp.FileHandle, what: string): Promise<void> {
  try {
    await handle.sync()
  } catch (err) {
    log.warn({ err, what }, '[history] could not fsync; this write may not survive power loss')
  }
}

async function syncDir(dir: string): Promise<void> {
  let handle: fsp.FileHandle
  try {
    handle = await fsp.open(dir, 'r')
  } catch (err) {
    log.warn({ err, dir }, '[history] could not open the directory to fsync it')
    return
  }
  try {
    await trySync(handle, 'the history directory')
  } finally {
    await handle.close().catch(() => {
      /* already closed */
    })
  }
}

/** Written, renamed, and synced. Answers whether it landed. */
export async function writeCheckpoint(dir: string, checkpoint: Checkpoint): Promise<boolean> {
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
    await fsp.mkdir(dir, { recursive: true, mode: 0o700 })
    const handle = await fsp.open(scratch, 'w', 0o600)
    try {
      await handle.writeFile(body, 'utf-8')
      // The file's own contents, before the rename makes them reachable.
      // Best-effort, like the directory below: a filesystem that refuses to sync
      // still gives an atomic rename against other readers, and a checkpoint
      // that is merely not power-loss-durable beats no checkpoint at all.
      await trySync(handle, 'the checkpoint file')
    } finally {
      await handle.close()
    }
    await fsp.rename(scratch, target)
    await syncDir(dir)
    return true
  } catch (err) {
    log.warn({ err, dir }, '[history] could not write a checkpoint')
    await fsp.rm(scratch, { force: true }).catch(() => {
      /* a scratch file this process created and could not remove */
    })
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

/** The same, off the event loop, for the recovery sweep that reads many. */
export async function readCheckpointAsync(dir: string): Promise<Checkpoint | null> {
  try {
    const raw: unknown = JSON.parse(await fsp.readFile(path.join(dir, CHECKPOINT_FILE), 'utf-8'))
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
    Number.isInteger(v.seq) &&
    (v.closedCleanly === undefined || typeof v.closedCleanly === 'boolean')
  )
}
