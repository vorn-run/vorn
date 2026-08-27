import fs from 'fs/promises'
import path from 'path'
import log from '../logger'
import { serializeScreen } from '../terminal-screen'
import { readScrollback } from '../terminal-scrollback'
import { frameBatch, frameOutput, frameResize, writeHeader } from './log'
import { historyDir, writeCheckpoint, LOG_FILE } from './checkpoint'

/**
 * What turns a terminal's output into a checkpoint and a log on disk.
 *
 * `log.ts` says what the bytes look like and `checkpoint.ts` says how one file
 * lands durably. This is the part with state: which sessions are being recorded,
 * what has not reached the disk yet, and when to pay for a checkpoint.
 *
 * ## The failure this is built for
 *
 * A process crash — `kill -9`, an uncaught throw, the OOM killer — not power
 * loss. That distinction decides where the fsyncs go. Appends are ordinary
 * writes: the kernel already holds them once `write` returns, so a process that
 * dies loses nothing that was appended, and paying for a sync on the hottest
 * path in the server to also survive a power cut would be a bad trade against
 * terminal output. The checkpoint is rare, so it syncs, and `checkpoint.ts`
 * syncs the directory too.
 *
 * What a crash does cost is whatever is still in `pending` — at most one tick of
 * output. Bounded and named rather than hidden.
 *
 * ## Why a queue per session
 *
 * Every disk operation for one session runs behind the last, because a
 * checkpoint and a truncate against the same two files cannot interleave. That
 * exclusivity is per directory and buys nothing across directories, so there is
 * no global tail: a session whose disk is slow must not hold up the checkpoint
 * of a session on the same machine that is about to be asked for its history.
 *
 * ## Why the log is truncated at every checkpoint
 *
 * A checkpoint is the screen as of the moment it was taken, so every frame
 * before it is superseded. Rewriting the log rather than appending past it means
 * recovery is `checkpoint + every frame in the log`, with nothing to skip and no
 * seq arithmetic to get wrong — and it is where the on-disk size is bounded,
 * which is what stops `~/.vorn` growing for the life of a session.
 *
 * The generation is what makes that safe. The checkpoint lands first; if the
 * process dies before the log is rewritten, the old log is still sitting there
 * describing what happened before a checkpoint that already includes it.
 * Replaying it would double every byte. It carries the old generation, the
 * checkpoint carries the new one, and recovery refuses the pair.
 */

/**
 * When this writes, in milliseconds.
 *
 * - `tickMs` -- how often accumulated frames are pushed to the log.
 * - `quiesceMs` -- how long a terminal must be quiet before its screen is worth
 *   writing. An agent that finished and is waiting is the case this exists for,
 *   and it should settle a second or two after going quiet rather than waiting
 *   out the interval below.
 * - `checkpointMs` -- the longest a busy terminal goes without one. A terminal
 *   producing output continuously never falls quiet, so this is what bounds how
 *   much log a crash leaves to replay.
 *
 * Passed rather than fixed because the numbers are a trade -- replay length
 * against I/O -- and the only honest way to set them is to measure both sides at
 * several values. The defaults are what the server runs at.
 */
export interface HistoryTiming {
  tickMs: number
  quiesceMs: number
  checkpointMs: number
}

const DEFAULT_TIMING: HistoryTiming = {
  tickMs: 250,
  quiesceMs: 2_000,
  checkpointMs: 30_000
}

let timing: HistoryTiming = DEFAULT_TIMING

/**
 * How large a log may grow before it is folded into a checkpoint.
 *
 * Half of `MAX_CHECKPOINT_BYTES`, so the log can never be the larger half of the
 * pair and a session's history on disk is bounded by the two together. The same
 * shape as the memory bound in `terminal-scrollback`: a cap with the compaction
 * that enforces it, rather than a cap nobody applies.
 *
 * This, not `checkpointMs`, is what actually bounds a busy terminal. A terminal
 * producing a megabyte a second never falls quiet and would otherwise put thirty
 * megabytes on disk between checkpoints, all of it to be replayed through a VT
 * parser on the next start.
 */
export const MAX_LOG_BYTES = 1024 * 1024

/**
 * How much unwritten output a session may hold before it is given up on.
 *
 * Reached only when the disk has stopped taking writes, since a tick drains this
 * every 250ms. Holding it would trade a disk problem for a memory one.
 */
const MAX_PENDING_BYTES = 4 * 1024 * 1024

interface Recorded {
  id: string
  dir: string
  /** Joined once. It cannot change, and it was being rebuilt four times a second. */
  logPath: string
  generation: number
  /** The last batch written to the log. A batch is one flush, not one write. */
  seq: number
  pending: Buffer[]
  pendingBytes: number
  logBytes: number
  /** Whether anything has been recorded since the last checkpoint. */
  changed: boolean
  /**
   * Set when the log on disk can no longer be trusted to be whole -- an append
   * that failed, or output dropped because the disk stopped taking it. Appending
   * past either would leave a hole in the middle of a file that replays
   * forwards, which is worse than a stale screen. Cleared by the next
   * checkpoint, which replaces the log rather than adding to it.
   */
  broken: boolean
  lastRecordAt: number
  lastCheckpointAt: number
  /** The per-session queue. One operation at a time, in the order asked for. */
  tail: Promise<void>
  queued: number
}

const recorded = new Map<string, Recorded>()

let dataDir: string | null = null
let ticker: NodeJS.Timeout | null = null

/**
 * Sealed once the server is shutting down.
 *
 * `shutdown()` checkpoints every session and only then kills the PTYs, and a
 * dying PTY runs the same teardown as one that exited on its own -- which
 * removes a session's history, because a terminal whose process is gone has
 * none worth keeping. Without this the last act of a clean shutdown would be to
 * delete every checkpoint it had just written.
 */
let sealed = false

/**
 * Where history is written, or nowhere.
 *
 * Explicit rather than `getDataDir()` at each call, which throws before the
 * database is open. Until this is called every entry point below is a no-op, so
 * nothing that runs early has to know about the ordering.
 */
export function configureHistory(dir: string, over: Partial<HistoryTiming> = {}): void {
  dataDir = dir
  timing = { ...DEFAULT_TIMING, ...over }
  sealed = false
}

/**
 * Begin recording a terminal, replacing any history left under its id.
 *
 * Not `stopHistory` first, which is what this used to do and which raced itself.
 * That enqueues a removal on the *old* record's queue and then this builds a new
 * record with a queue of its own -- two queues over one directory, so the old
 * removal could land after the new log had been written and take it away. The
 * symptom was a respawned session whose history quietly stopped until the next
 * checkpoint rebuilt the directory.
 *
 * The new record inherits the old one's tail instead. That keeps every operation
 * on this directory in one order, including any append still in flight from the
 * PTY that just went, and `reset` below removes what was there anyway.
 */
export function startHistory(id: string): void {
  if (!dataDir || sealed) return
  const previous = recorded.get(id)
  recorded.delete(id)

  const held: Recorded = {
    id,
    dir: historyDir(dataDir, id),
    logPath: path.join(historyDir(dataDir, id), LOG_FILE),
    generation: 1,
    seq: 0,
    pending: [],
    pendingBytes: 0,
    logBytes: 0,
    changed: false,
    broken: false,
    lastRecordAt: Date.now(),
    lastCheckpointAt: Date.now(),
    tail: previous ? previous.tail.catch(() => undefined) : Promise.resolve(),
    queued: 0
  }
  recorded.set(id, held)
  enqueue(held, () => reset(held))
}

/**
 * Record output, on the flush that already coalesces it.
 *
 * Called from `flushBuffer` rather than from `onData` for the same reason
 * `feedScreen` is: node-pty emits a few bytes at a time while somebody types,
 * and this way a burst of keystrokes is one frame rather than thirty.
 */
export function recordOutput(id: string, data: string): void {
  if (!data) return
  // Looked up before the frame is built, not after. Encoding the chunk and
  // running a checksum over it only to find that nothing is recording this
  // session is a full pass over every byte, thrown away -- and that is the
  // ordinary case in every process that never called `configureHistory`.
  const held = recorded.get(id)
  if (held) push(held, frameOutput(data))
}

/** Record a resize, with the numbers node-pty was given. */
export function recordResize(id: string, cols: number, rows: number): void {
  const held = recorded.get(id)
  if (held) push(held, frameResize(cols, rows))
}

function push(held: Recorded, frame: Buffer): void {
  held.lastRecordAt = Date.now()
  held.changed = true
  // Still accumulated while broken, because the next checkpoint replaces the log
  // wholesale and these frames belong after it. Only the appending stops.
  held.pending.push(frame)
  held.pendingBytes += frame.length

  if (held.pendingBytes > MAX_PENDING_BYTES) {
    if (!held.broken) {
      log.warn(
        { id: held.id },
        '[history] the disk is not keeping up; dropping this log until the next checkpoint'
      )
    }
    held.broken = true
    take(held)
  }

  ensureTicking()
}

/**
 * Stop recording a terminal and remove what was written for it.
 *
 * A PTY that has exited leaves nothing worth restoring -- the same reasoning
 * that already drops its scrollback and its screen model. Refused once sealed,
 * so a shutdown does not undo its own work.
 */
export function stopHistory(id: string): void {
  const held = recorded.get(id)
  if (!held) return
  recorded.delete(id)
  if (sealed) return
  held.pending = []
  held.pendingBytes = 0
  enqueue(held, () => fs.rm(held.dir, { recursive: true, force: true }))
}

/**
 * Write every session's screen out, for a server that is going down.
 *
 * Seals first, so the PTY teardown that follows cannot remove what this writes.
 *
 * The wait is bounded and the work is not: an operation past the deadline runs
 * to completion and this simply stops waiting on it. Cancelling would leave a
 * half-written log to save time on a path where `SHUTDOWN_DEADLINE_MS` is
 * already the backstop.
 */
export async function flushHistory(): Promise<void> {
  sealed = true
  stopTicking()

  const all = Promise.all(
    [...recorded.values()].map((held) => enqueue(held, () => checkpoint(held)))
  )
  let timer: NodeJS.Timeout | undefined
  const deadline = new Promise<void>((resolve) => {
    timer = setTimeout(() => {
      log.warn('[history] gave up waiting for checkpoints; they are still running')
      resolve()
    }, FLUSH_DEADLINE_MS)
  })
  try {
    await Promise.race([all.then(() => undefined), deadline])
  } finally {
    if (timer) clearTimeout(timer)
  }
}

const FLUSH_DEADLINE_MS = 5_000

/** Test-only, mirroring `resetScrollback` and `resetScreens`. */
export function resetHistory(): void {
  stopTicking()
  recorded.clear()
  dataDir = null
  timing = DEFAULT_TIMING
  sealed = false
}

/** What a session is holding, so the bounds above are observable. */
export function historyState(
  id: string
): { pendingBytes: number; logBytes: number; broken: boolean } | null {
  const held = recorded.get(id)
  if (!held) return null
  const { pendingBytes, logBytes, broken } = held
  return { pendingBytes, logBytes, broken }
}

/** Run every pending operation to completion. For tests, and for nothing else. */
export async function settleHistory(): Promise<void> {
  // Looped, because an operation can enqueue another -- a checkpoint that finds
  // no screen falls back to a flush. Bounded so a test hangs on an assertion
  // rather than on this.
  for (let round = 0; round < 16; round++) {
    const tails = [...recorded.values()].map((held) => held.tail)
    if (!tails.length) return
    await Promise.all(tails)
    if (![...recorded.values()].some((held) => held.queued > 0)) return
  }
}

/**
 * Put one operation at the end of this session's queue.
 *
 * Rejections are swallowed here rather than at each call site: every operation
 * below already reports its own failures, and an escaped rejection from a timer
 * has no caller to reach.
 */
function enqueue(held: Recorded, op: () => Promise<unknown>): Promise<void> {
  held.queued += 1
  held.tail = held.tail
    .then(op)
    .catch((err) => {
      log.warn({ err, id: held.id }, '[history] a write failed')
    })
    .then(() => {
      held.queued -= 1
    })
  return held.tail
}

/**
 * Take everything not yet written, and leave the record empty.
 *
 * One function because `pending` and `pendingBytes` have to move together, and
 * the five places that used to do it by hand each restated that invariant
 * without enforcing it -- a sixth that forgot the byte count would have broken
 * the `MAX_PENDING_BYTES` bound silently.
 */
function take(held: Recorded): Buffer[] {
  const frames = held.pending
  held.pending = []
  held.pendingBytes = 0
  return frames
}

/**
 * Replace the log, opening a generation.
 *
 * Opening one is four things that are only correct together: write a header
 * carrying it, adopt it, restart `seq`, and trust the file again. Three call
 * sites used to do those by hand, in three slightly different orders.
 *
 * The body is assembled here rather than passed in because the layout -- a
 * header, then whole batches, and a fresh generation opening on batch one -- is
 * the format's rule, and a caller building it from format primitives is a caller
 * that has to remember the rule.
 */
async function openGeneration(
  held: Recorded,
  generation: number,
  carried: Buffer[]
): Promise<void> {
  const body = carried.length
    ? Buffer.concat([writeHeader(generation), frameBatch(1), ...carried])
    : writeHeader(generation)
  try {
    await fs.writeFile(held.logPath, body, { mode: 0o600 })
    held.generation = generation
    held.logBytes = body.length
    held.seq = carried.length ? 1 : 0
    held.broken = false
  } catch (err) {
    // Left untrusted rather than appended to over an unknown prefix: a partial
    // rewrite is a file whose remaining bytes have no established length.
    log.warn({ err, id: held.id }, '[history] could not replace the log')
    held.broken = true
    throw err
  }
}

async function reset(held: Recorded): Promise<void> {
  await fs.rm(held.dir, { recursive: true, force: true })
  await fs.mkdir(held.dir, { recursive: true, mode: 0o700 })
  await openGeneration(held, held.generation, [])
}

/**
 * Append what has accumulated, as one batch.
 *
 * The batch marker goes in here rather than beside every write because a batch
 * is what reaches the disk together, and that is the unit a torn tail cuts.
 */
async function flushPending(held: Recorded): Promise<void> {
  if (!held.pending.length || held.broken) return

  held.seq += 1
  const prefix = frameBatch(held.seq)
  const frames = take(held)
  // The total is passed rather than summed, and the marker is unshifted rather
  // than spread. `pending` is bounded by bytes and not by count, so with small
  // frames it holds a great many -- and a spread of that array is an argument
  // list of that length.
  const total = frames.reduce((n, f) => n + f.length, prefix.length)
  frames.unshift(prefix)
  const body = Buffer.concat(frames, total)

  try {
    await fs.appendFile(held.logPath, body)
    held.logBytes += body.length
  } catch (err) {
    // Not put back. A retry that succeeded after a later batch had been written
    // would put the log out of order, and holding it grows without bound against
    // a disk that has stopped answering.
    log.warn(
      { err, id: held.id },
      '[history] could not append; this log waits for the next checkpoint'
    )
    held.broken = true
  }
}

/**
 * Fold a log that has outgrown its cap into a checkpoint.
 *
 * A checkpoint already replaces the log rather than appending past it, so
 * compaction is not a separate mechanism -- it is the ordinary checkpoint,
 * asked for by size instead of by time.
 *
 * When one cannot be written the log is thrown away instead, and the generation
 * is moved so that what is left cannot be replayed onto the checkpoint still
 * sitting beside it. That loses history, which is the point: a session whose
 * screen model has faulted would otherwise append for the life of the server
 * with nothing able to bound it, and the last good checkpoint is still there to
 * restore from.
 */
async function fold(held: Recorded): Promise<void> {
  await checkpoint(held)
  if (held.logBytes <= MAX_LOG_BYTES) return

  log.warn(
    { id: held.id, bytes: held.logBytes },
    '[history] could not checkpoint a log past its cap; dropping it'
  )
  take(held)
  await openGeneration(held, held.generation + 1, []).catch(() => {
    /* already reported, and already marked untrusted */
  })
}

/**
 * Write the screen, then replace the log with whatever arrived while it was
 * being written.
 *
 * The first four statements are deliberately one synchronous block, and the
 * order inside it is the correctness argument. `serializeScreen` queues an empty
 * write behind everything already given to xterm and resolves when the parser
 * reaches it, so the screen it returns is exactly the output recorded before
 * this line ran -- but only if nothing is recorded between taking `seq` and
 * placing that marker. Nothing can be: there is no await between them.
 */
async function checkpoint(held: Recorded): Promise<void> {
  const cutSeq = held.seq
  const scrollback = readScrollback(held.id)
  const drained = serializeScreen(held.id)
  const supersededBytes = held.pendingBytes
  const superseded = take(held)

  const snapshot = await drained
  const generation = held.generation + 1
  const landed =
    snapshot !== null &&
    (await writeCheckpoint(held.dir, {
      screen: snapshot.screen,
      scrollback,
      cols: snapshot.cols,
      rows: snapshot.rows,
      title: snapshot.title,
      cwd: snapshot.cwd,
      generation,
      seq: cutSeq
    }))

  if (!landed) {
    // Either there was no model to checkpoint from -- it faulted, or the session
    // is gone -- or the write did not land. Those frames are still the truth
    // about what happened, so they go back at the front and the log carries on.
    restore(held, superseded, supersededBytes)
    await flushPending(held)
    return
  }

  // From here the frames in `superseded` are inside the checkpoint and must not
  // be replayed over it. Only what arrived during the serialize is carried.
  held.lastCheckpointAt = Date.now()
  const carried = take(held)
  held.changed = carried.length > 0
  await openGeneration(held, generation, carried).catch(() => {
    // The checkpoint is already durable, so the screen survives; what is lost is
    // the little that came after it. Reported and marked there.
  })
}

/**
 * Put frames back at the front, with the byte count they came with.
 *
 * Carried rather than recomputed: it was `held.pendingBytes` at the moment they
 * were taken, so walking the whole array to work it out again is a pass over
 * everything unwritten for a number already known.
 */
function restore(held: Recorded, superseded: Buffer[], bytes: number): void {
  if (!superseded.length) return
  for (let i = superseded.length - 1; i >= 0; i--) held.pending.unshift(superseded[i]!)
  held.pendingBytes += bytes
}

/**
 * One timer for every session, started on demand and stopped when there is
 * nothing left to write.
 *
 * A timer per session would be a hundred timers for a hundred terminals, and an
 * interval that runs for the life of the server would tick four times a second
 * through every idle night. Unref'd either way, so it is never the reason this
 * process is still alive.
 */
function ensureTicking(): void {
  if (ticker) return
  ticker = setInterval(tick, timing.tickMs)
  ticker.unref?.()
}

function stopTicking(): void {
  if (!ticker) return
  clearInterval(ticker)
  ticker = null
}

function tick(): void {
  const now = Date.now()
  let working = false

  for (const held of recorded.values()) {
    // Anything unwritten keeps the timer alive, including a session that is
    // merely not quiet yet -- it is owed a checkpoint, and stopping here would
    // leave it owed one until its terminal next produced output.
    if (!held.changed && !held.pending.length) continue
    working = true
    if (held.queued > 0) continue

    if (held.pending.length) {
      void enqueue(held, async () => {
        await flushPending(held)
        if (held.logBytes > MAX_LOG_BYTES) await fold(held)
      })
      continue
    }

    const quiet = now - held.lastRecordAt >= timing.quiesceMs
    const overdue = now - held.lastCheckpointAt >= timing.checkpointMs
    if (quiet || overdue) void enqueue(held, () => checkpoint(held))
  }

  if (!working) stopTicking()
}
