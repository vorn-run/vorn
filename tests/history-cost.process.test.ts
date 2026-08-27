import { describe, it, expect } from 'vitest'
import { runMeasurement } from './helpers/run-measurement'
import { spawnsRealServers } from './helpers/one-at-a-time'

/**
 * What keeping history costs, in the three places it is paid.
 *
 * The interval this runs at is a trade — how much log a crash leaves to replay,
 * against how much work an idle machine does — and picking a number by reasoning
 * about which sounds heavier is how a server ends up checkpointing fifty
 * terminals every second. So the three costs are measured: what an append adds
 * to the path every byte already takes, what checkpointing every session at once
 * costs, and what a restart spends reading it back.
 *
 * In a child process for the same reason as the other two measurements: this
 * worker holds the module graph and the rest of the suite, which is noise larger
 * than most of what is being measured.
 *
 * The assertions are relationships between numbers from the same run, not
 * ceilings on any one of them. Wall-clock on a loaded CI box is not a thing to
 * assert against a constant, and a flaky performance test gets deleted rather
 * than investigated — which leaves nothing. The absolute figures belong in the
 * commit message, where they are a record rather than a trap.
 */

spawnsRealServers()

interface Measurement {
  chunks: number
  sessions: number
  /** The write path as it was before history: byte buffer and screen model. */
  withoutHistoryMs: number
  /** The same path with a frame recorded per chunk. */
  withHistoryMs: number
  /** Building a frame per chunk, timed on its own. */
  frameOnlyMs: number
  /** Building one per flush instead, which is what the server does. */
  frameCoalescedMs: number
  perFlush: number
  /** Pushing those frames to disk, which happens behind the path, not on it. */
  appendToDiskMs: number
  /** Checkpointing every session at once, by session count. */
  checkpointMs: Record<string, number>
  /** Restoring all of them on start. */
  recoverMs: number
  bytesPerSession: number
}

describe('what a terminal that survives a crash costs', () => {
  const r = runMeasurement<Measurement>('measure-history.ts', { timeoutMs: 300_000 })

  it('adds little to the path every byte of output already takes', () => {
    // The question that decides whether this belongs on the flush at all: a
    // length, a checksum over the payload and a push, against a full VT parse of
    // the same bytes that was already there.
    //
    // Measured directly rather than as a difference. Subtracting one whole-path
    // timing from another put the answer inside this machine's noise -- two ~35ms
    // samples whose gap came out anywhere from three per cent to negative -- so
    // the number that used to be asserted here said nothing. This times the frame
    // construction itself: 1.5ms against a 33ms path, for the same bytes.
    expect(
      r.frameCoalescedMs,
      `${r.frameCoalescedMs}ms of framing on a ${r.withoutHistoryMs}ms path, ${r.chunks} chunks`
    ).toBeLessThan(r.withoutHistoryMs / 4)
  })

  it('is affordable because of the flush, not in spite of it', () => {
    // The reason `recordOutput` is called from `flushBuffer` rather than from
    // `onData`. A frame per chunk costs 13.5ms for these bytes; one per flush,
    // over the same bytes, costs 1.5ms -- one encode and one checksum pass
    // instead of a hundred of each. If that ratio collapses, something has moved
    // the call back onto the per-chunk path.
    expect(
      r.frameCoalescedMs,
      `${r.frameOnlyMs}ms per chunk, ${r.frameCoalescedMs}ms at ${r.perFlush} chunks a flush`
    ).toBeLessThan(r.frameOnlyMs / 3)
  })

  it('does the disk work behind that path rather than on it', () => {
    // Frames accumulate and a timer writes them. What a terminal waits for is
    // the framing above; this is what happens afterwards, and it must stay the
    // smaller of the two or the coalescing is not earning anything.
    expect(
      r.appendToDiskMs,
      `${r.appendToDiskMs}ms of writing behind ${r.withHistoryMs}ms of path`
    ).toBeLessThan(r.withHistoryMs)
  })

  it('checkpoints in time that grows with session count and not faster', () => {
    // What this catches is a shape change: a queue that became global, or a scan
    // that became quadratic, either of which turns a machine with fifty
    // terminals into one that cannot shut down. Five times the sessions were
    // measured at 4.7 times the cost -- 102ms for ten, 480ms for fifty.
    const ten = r.checkpointMs['10'] ?? 0
    const fifty = r.checkpointMs[String(r.sessions)] ?? 0
    expect(fifty, `${ten}ms for ten sessions, ${fifty}ms for ${r.sessions}`).toBeLessThan(ten * 10)
  })

  it('reads every terminal back for less than it cost to write them', () => {
    // Recovery runs before the port file and the credential are published, so
    // its cost is not a window where history is missing -- it is a delay before
    // anything can find the server at all. Measured at 62ms for fifty sessions,
    // against 480ms to write them.
    expect(
      r.recoverMs,
      `${r.recoverMs}ms to restore ${r.sessions}, ${r.checkpointMs[String(r.sessions)]}ms to write them`
    ).toBeLessThan(r.checkpointMs[String(r.sessions)] ?? 0)
  })
}, 300_000)
