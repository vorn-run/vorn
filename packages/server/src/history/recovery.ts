import fs from 'fs'
import path from 'path'
import log from '../logger'
import { createScreen, feedScreen, resizeScreen } from '../terminal-screen'
import { seedScrollback } from '../terminal-scrollback'
import { readHeader, readFrames, type Frame, type StopReason } from './log'
import { readCheckpoint, historyDir, LOG_FILE } from './checkpoint'

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
  /** What the session record remembers, which for an older row is nothing. */
  cols?: number
  rows?: number
}

/**
 * The geometry to rebuild at when neither the checkpoint nor the session record
 * says. The same numbers a PTY is spawned at, so a screen restored without one
 * wraps where the program was writing.
 */
const FALLBACK_COLS = 80
const FALLBACK_ROWS = 24

export interface RecoveryReport {
  recovered: Recovered[]
  /** Directories removed because nothing can name them any more. */
  swept: number
}

/**
 * Restore every session the database still knows about, and sweep the rest.
 *
 * The sweep is safe for one reason: history is keyed by session id, and an id
 * the database has no record of cannot be reached by anything in the product.
 * `getPreviousSessions` is the only way a pane ever names one. So a directory
 * with no session behind it is not history somebody might want, it is history
 * nobody can ask for -- ordinary residue from a crash that ran none of the
 * teardown that normally removes it.
 */
export async function recoverHistory(
  dataDir: string,
  sessions: RecoverableSession[]
): Promise<RecoveryReport> {
  const root = path.join(dataDir, 'history')
  let entries: string[]
  try {
    entries = fs.readdirSync(root)
  } catch {
    // No history directory is the ordinary first run, not a failure.
    return { recovered: [], swept: 0 }
  }

  const known = new Map(sessions.map((s) => [s.id, s]))
  const report: RecoveryReport = { recovered: [], swept: 0 }

  for (const entry of entries) {
    const id = decode(entry)
    const session = id === null ? undefined : known.get(id)
    if (!session) {
      try {
        fs.rmSync(path.join(root, entry), { recursive: true, force: true })
        report.swept += 1
      } catch (err) {
        log.warn({ err, entry }, '[history] could not remove history for a session that is gone')
      }
      continue
    }

    try {
      const one = await restore(historyDir(dataDir, session.id), session)
      if (one) report.recovered.push(one)
    } catch (err) {
      log.warn({ err, id: session.id }, '[history] could not restore this terminal')
    }
  }

  if (report.recovered.length || report.swept) {
    const damaged = report.recovered.filter((r) => r.stopped !== 'end')
    log.info(
      { restored: report.recovered.length, swept: report.swept, damaged: damaged.length },
      '[history] restored terminals from the last run'
    )
    for (const one of damaged) {
      log.warn({ id: one.id, stopped: one.stopped }, '[history] this log did not read to the end')
    }
  }

  return report
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
  const checkpoint = readCheckpoint(dir)
  const { frames, stopped } = readLog(dir, checkpoint?.generation)
  if (!checkpoint && !frames.length) return null

  // The geometry the checkpoint was taken at, not the one the session record
  // remembers: the screen is being rebuilt from bytes that wrapped at those
  // columns, and rebuilding it at any other width moves every line after the
  // first wrap. A resize frame further down the log moves it afterwards, which
  // is what the client did at the time too.
  const cols = checkpoint?.cols ?? session.cols ?? FALLBACK_COLS
  const rows = checkpoint?.rows ?? session.rows ?? FALLBACK_ROWS

  createScreen(session.id, cols, rows)
  let scrollback = checkpoint?.scrollback ?? ''
  if (checkpoint?.screen) feedScreen(session.id, checkpoint.screen)

  for (const frame of frames) {
    await apply(session.id, frame, cols, rows)
    if (frame.kind === 'output') scrollback += frame.data
    if (frame.kind === 'clear') scrollback = ''
  }

  seedScrollback(session.id, scrollback)
  return {
    id: session.id,
    replayed: frames.length,
    stopped,
    fromCheckpoint: checkpoint !== null
  }
}

function readLog(
  dir: string,
  generation: number | undefined
): { frames: Frame[]; stopped: StopReason } {
  let buf: Buffer
  try {
    buf = fs.readFileSync(path.join(dir, LOG_FILE))
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

async function apply(id: string, frame: Frame, cols: number, rows: number): Promise<void> {
  switch (frame.kind) {
    case 'output':
      feedScreen(id, frame.data)
      return
    case 'resize':
      await resizeScreen(id, frame.cols, frame.rows)
      return
    case 'clear':
      createScreen(id, cols, rows)
      return
    case 'batch':
      // A boundary, not a thing that happened. It marks where one flush ended so
      // a torn tail can be found; there is nothing to apply.
      return
  }
}
