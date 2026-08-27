import fs from 'fs/promises'
import path from 'path'
import log from '../logger'
import { createScreen, feedScreen, resizeScreen } from '../terminal-screen'
import { seedScrollback } from '../terminal-scrollback'
import { readHeader, readFrames, type Frame, type StopReason } from './log'
import { readCheckpointAsync, LOG_FILE, CHECKPOINT_FILE } from './checkpoint'

/**
 * Putting back what a crash interrupted.
 *
 * The checkpoint is the screen as of the moment it was written and the log is
 * everything that happened after. Replaying one over the other is the whole
 * mechanism, and the two rules that keep it honest are both refusals:
 *
 *   - **A log whose generation does not match the checkpoint is ignored.** It
 *     describes what happened before a checkpoint that already includes it, and
 *     replaying it would print every byte twice. This is the case a crash
 *     between the checkpoint landing and the log being replaced produces, which
 *     is a window the writer cannot close.
 *   - **A frame that fails its length or its checksum ends the replay.** Not
 *     skipped: the bytes after a frame nobody can vouch for have no established
 *     meaning, and a screen assembled from them is wrong in ways nobody can
 *     bound. `readFrames` already stops; this just does not argue with it.
 *
 * A log with no checkpoint beside it is still worth replaying. `startHistory`
 * writes a fresh header and appends from the beginning, so a session that
 * crashed before its first checkpoint has a complete log -- which is exactly the
 * short-lived session a checkpoint interval was never going to cover.
 *
 * Sessions are restored a few at a time rather than one after another. Each
 * touches only its own directory, its own screen model and its own scrollback
 * key, so there is nothing to serialize them for -- and reading them one at a
 * time meant up to a hundred and fifty megabytes of file reads that the thread
 * pool could have overlapped. Bounded rather than unbounded: fifty terminals
 * built at once is fifty emulators and every checkpoint resident together.
 *
 * ## What this does not do
 *
 * It does not hand anything to a client. Nothing asks: `terminal:readScrollback`
 * exists and no caller uses it, and replaying into a live pane needs a sequence
 * number the live path does not carry. This restores the server's own two models
 * so that the pane, when something is built to ask, has something to be given.
 */

export interface Recovered {
  id: string
  /** Frames replayed from the log on top of the checkpoint. */
  replayed: number
  /** Why the replay stopped. Anything but `end` is damage worth knowing about. */
  stopped: StopReason
  /** Whether a checkpoint was found to replay onto. */
  fromCheckpoint: boolean
}

export interface RecoverableSession {
  id: string
}

/**
 * The geometry to rebuild at when there is no checkpoint to take one from.
 *
 * Not a guess. A log with no checkpoint beside it begins where `startHistory`
 * opened it, which is the spawn -- and a PTY is spawned at exactly these
 * numbers. Any resize the terminal saw afterwards is a frame further down that
 * same log, so the replay arrives at the right size by the same route the
 * original did.
 *
 * The session record is not consulted: the `sessions` table carries no geometry,
 * so a term reading `session.cols` would look like a source of truth and always
 * be undefined.
 */
const FALLBACK_COLS = 80
const FALLBACK_ROWS = 24

/**
 * How many terminals are rebuilt.
 *
 * Each one is a headless emulator with its buffers, held for the life of the
 * process -- nothing disposes a recovered model, because nothing has attached to
 * it yet. Fifty was measured at about eight megabytes, which is the same ceiling
 * fifty live sessions carry, so this bounds the recovered set at no more than
 * the running one. What is skipped keeps its files: it is not restored, it is
 * not read yet, and a later start can have it.
 */
const MAX_RESTORED = 50

export interface RecoveryReport {
  recovered: Recovered[]
  /** Directories removed because nothing can name them any more. */
  swept: number
}

/** How many sessions are rebuilt at once. Enough to overlap the reads, few
 * enough that peak memory stays near one terminal's worth times this. */
const AT_ONCE = 8

/**
 * Restore every session the database still knows about, and sweep the rest.
 *
 * The sweep is safe for one reason: history is keyed by session id, and an id
 * the database has no record of cannot be reached by anything in the product.
 * `getPreviousSessions` is the only way a pane ever names one. So a directory
 * with no session behind it is not history somebody might want, it is history
 * nobody can ask for -- ordinary residue from a crash that ran none of the
 * teardown that normally removes it.
 *
 * That reasoning holds only while the list is known to be complete, which is why
 * `sessions` may be null. `getPreviousSessions` answers `[]` both when there are
 * none and when it could not read them, and those two are the difference between
 * removing nothing and removing every terminal's history on the strength of one
 * transient database error. Null means "could not read": restore nothing, sweep
 * nothing, leave it all for a start that can.
 */
export async function recoverHistory(
  dataDir: string,
  sessions: RecoverableSession[] | null
): Promise<RecoveryReport> {
  const root = path.join(dataDir, 'history')
  let entries: string[]
  try {
    entries = await fs.readdir(root)
  } catch {
    // No history directory is the ordinary first run, not a failure.
    return { recovered: [], swept: 0 }
  }

  if (sessions === null) {
    log.warn('[history] the session list could not be read; leaving every terminal alone')
    return { recovered: [], swept: 0 }
  }

  const known = new Map(sessions.map((s) => [s.id, s]))
  const report: RecoveryReport = { recovered: [], swept: 0 }
  const restorable: Array<{ at: string; session: RecoverableSession }> = []

  for (const entry of entries) {
    // Named once. Rebuilding the path from the decoded id would re-encode what
    // was just decoded, and `encodeURIComponent(decodeURIComponent(x)) === x`
    // does not hold for every string a directory listing can produce.
    const at = path.join(root, entry)
    const id = decode(entry)
    const session = id === null ? undefined : known.get(id)
    if (session) {
      restorable.push({ at, session })
      continue
    }
    try {
      await fs.rm(at, { recursive: true, force: true })
      report.swept += 1
    } catch (err) {
      log.warn({ err, entry }, '[history] could not remove history for a session that is gone')
    }
  }

  if (restorable.length > MAX_RESTORED) {
    // Said rather than done quietly. A cap that nobody is told about reads as
    // "everything was restored" on the one start where it was not.
    log.warn(
      { found: restorable.length, restoring: MAX_RESTORED },
      '[history] more terminals on disk than are rebuilt at once; the rest keep their files'
    )
    restorable.length = MAX_RESTORED
  }

  for (let from = 0; from < restorable.length; from += AT_ONCE) {
    const batch = restorable.slice(from, from + AT_ONCE)
    const done = await Promise.all(
      batch.map(async ({ at, session }) => {
        try {
          return await restore(at, session)
        } catch (err) {
          log.warn({ err, id: session.id }, '[history] could not restore this terminal')
          return null
        }
      })
    )
    for (const one of done) if (one) report.recovered.push(one)
  }

  if (report.recovered.length || report.swept) {
    const damaged = report.recovered.filter((r) => r.stopped !== 'end')
    log.info(
      {
        restored: report.recovered.length,
        swept: report.swept,
        damaged: damaged.length,
        // A session restored without one crashed before its first checkpoint,
        // so its whole history came from the log. Worth being able to see.
        fromLogAlone: report.recovered.filter((r) => !r.fromCheckpoint).length
      },
      '[history] restored terminals from the last run'
    )
    for (const one of damaged) {
      log.warn({ id: one.id, stopped: one.stopped }, '[history] this log did not read to the end')
    }
  }

  return report
}

/**
 * Remove scratch files a crash left behind.
 *
 * `writeCheckpoint` names its scratch file randomly and removes it only when the
 * write fails; a process that dies mid-write removes nothing, and up to two
 * megabytes of it sits there under a name nothing will ever reuse. Cleared here
 * because this is the one moment it is certainly safe -- no writer for this
 * session exists yet.
 */
async function sweepScratch(dir: string): Promise<void> {
  try {
    for (const entry of await fs.readdir(dir)) {
      if (entry.startsWith(`.${CHECKPOINT_FILE}.`)) {
        await fs.rm(path.join(dir, entry), { force: true })
      }
    }
  } catch {
    /* nothing there, or nothing removable; neither stops a restore */
  }
}

/**
 * A directory name back into a session id.
 *
 * Null rather than a throw for something that is not one: `decodeURIComponent`
 * rejects a lone percent, and a directory nobody wrote could be anything.
 */
function decode(entry: string): string | null {
  try {
    return decodeURIComponent(entry)
  } catch {
    return null
  }
}

async function restore(dir: string, session: RecoverableSession): Promise<Recovered | null> {
  await sweepScratch(dir)
  const checkpoint = await readCheckpointAsync(dir)
  const { frames, stopped } = await readLog(dir, checkpoint?.generation)
  if (!checkpoint && !frames.length) return null

  // The geometry the checkpoint was taken at, not the one the session record
  // remembers: the screen is being rebuilt from bytes that wrapped at those
  // columns, and rebuilding it at any other width moves every line after the
  // first wrap. A resize frame further down the log moves it afterwards, which
  // is what the client did at the time too.
  const cols = checkpoint?.cols ?? FALLBACK_COLS
  const rows = checkpoint?.rows ?? FALLBACK_ROWS

  createScreen(session.id, cols, rows)
  let scrollback = checkpoint?.scrollback ?? ''
  if (checkpoint?.screen) feedScreen(session.id, checkpoint.screen)

  for (const frame of frames) {
    await apply(session.id, frame)
    if (frame.kind === 'output') scrollback += frame.data
  }

  seedScrollback(session.id, scrollback)
  return {
    id: session.id,
    replayed: frames.length,
    stopped,
    fromCheckpoint: checkpoint !== null
  }
}

async function readLog(
  dir: string,
  generation: number | undefined
): Promise<{ frames: Frame[]; stopped: StopReason }> {
  let buf: Buffer
  try {
    buf = await fs.readFile(path.join(dir, LOG_FILE))
  } catch {
    return { frames: [], stopped: 'end' }
  }

  const header = readHeader(buf)
  if (!header) return { frames: [], stopped: 'malformed' }
  // The refusal this whole scheme rests on. Not a warning and a replay anyway:
  // a log from before the checkpoint beside it is not a shorter history, it is a
  // second copy of one already restored.
  if (generation !== undefined && header.generation !== generation) {
    log.warn(
      { dir, log: header.generation, checkpoint: generation },
      '[history] this log belongs to an earlier checkpoint; ignoring it'
    )
    return { frames: [], stopped: 'end' }
  }

  const read = readFrames(buf)
  return { frames: read.frames, stopped: read.reason }
}

async function apply(id: string, frame: Frame): Promise<void> {
  switch (frame.kind) {
    case 'output':
      feedScreen(id, frame.data)
      return
    case 'resize':
      await resizeScreen(id, frame.cols, frame.rows)
      return
    case 'batch':
      // A boundary, not a thing that happened. It marks where one flush ended so
      // a torn tail can be found; there is nothing to apply.
      return
  }
}
