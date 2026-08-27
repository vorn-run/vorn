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
  appendOnlyMs: number
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
    // The question that decides whether this belongs on the flush at all. A
    // frame is a length, a checksum over the payload, and a push -- against a
    // full VT parse of the same bytes, which was already there. Measured at
    // roughly 38.6ms without and 39.7ms with, for twenty thousand chunks: about
    // three per cent, and that is the pessimistic reading, because production
    // records once per flush rather than once per chunk.
    expect(
      r.appendOnlyMs,
      `${r.appendOnlyMs}ms added to ${r.withoutHistoryMs}ms for ${r.chunks} chunks`
    ).toBeLessThan(r.withoutHistoryMs / 4)
  })

  it('does the disk work behind that path rather than on it', () => {
    // Frames accumulate and a timer writes them. What a terminal waits for is
    // the append above; this is what happens afterwards, and it must stay the
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
    // measured at 5.2 times the cost -- 121ms for ten, 627ms for fifty.
    const ten = r.checkpointMs['10'] ?? 0
    const fifty = r.checkpointMs[String(r.sessions)] ?? 0
    expect(fifty, `${ten}ms for ten sessions, ${fifty}ms for ${r.sessions}`).toBeLessThan(ten * 10)
  })

  it('reads every terminal back for less than it cost to write them', () => {
    // Recovery sits between the listen and the first client, so its cost is a
    // window where history is not loaded yet. Measured at 77ms for fifty
    // sessions, against 627ms to write them.
    expect(
      r.recoverMs,
      `${r.recoverMs}ms to restore ${r.sessions}, ${r.checkpointMs[String(r.sessions)]}ms to write them`
    ).toBeLessThan(r.checkpointMs[String(r.sessions)] ?? 0)
  })
}, 300_000)
