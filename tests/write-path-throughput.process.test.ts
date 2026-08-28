import { describe, it, expect } from 'vitest'
import { runMeasurement } from './helpers/run-measurement'
import { spawnsRealServers } from './helpers/one-at-a-time'

/**
 * What the server spends on each chunk of terminal output.
 *
 * The hottest path in the process: every byte an agent prints arrives here, and
 * anything added to it is paid on every keystroke of every session at once. Two
 * things changed on it — the byte buffer stopped rebuilding itself per chunk,
 * and a screen model was added beside it. Whether that is a net win is not
 * answerable by reasoning about which sounds heavier, so all three shapes are
 * measured against the same bytes.
 *
 * In a child process for the same reason as the memory measurement: this
 * worker's own scheduling noise is larger than the differences that matter.
 *
 * Every assertion is a comparison between numbers from the same run, never a
 * ceiling on one of them. Wall-clock on a loaded CI box is not a thing to assert
 * against a constant, and a flaky performance test gets deleted rather than
 * investigated — which leaves nothing at all. What these catch is a change of
 * *shape*: the buffer going quadratic again, or the model becoming the thing
 * that dominates this path.
 *
 * The absolute figures belong in a commit message, where they are a record
 * rather than a trap.
 */

// Takes the same lock the server suites take. This one measures wall-clock and
// heap, so anything spawning beside it is measuring something else.
spawnsRealServers()

interface Measurement {
  chunks: number
  bytes: number
  /** The buffer as it was: concatenate the whole thing per chunk, then re-slice. */
  oldBufferMs: number
  /** The buffer as it is: push, join on read. */
  newBufferMs: number
  /** The buffer as it is, with the screen model fed alongside. */
  newBufferAndScreenMs: number
  /** What the model alone adds. */
  screenOnlyMs: number
}

describe('the path every byte of output takes', () => {
  const r = runMeasurement<Measurement>('measure-write-path.ts', { timeoutMs: 300_000 })

  it('is faster than it was before a screen model was added to it', () => {
    // The headline, and the reason the buffer fix belongs in the same branch as
    // the emulator. Adding a full VT parse of every byte sounds like it must
    // cost something, and it does — but the path it was added to was spending
    // twenty-five times more than that rebuilding a quarter-megabyte string on
    // every chunk. Measured at roughly 1040 ms before and 42 ms after, for
    // twenty thousand chunks.
    expect(
      r.newBufferAndScreenMs,
      `before ${r.oldBufferMs}ms, after ${r.newBufferAndScreenMs}ms for ${r.chunks} chunks`
    ).toBeLessThan(r.oldBufferMs)
  })

  it('spends most of what is left on the model rather than the buffer', () => {
    // A ratio, not a ceiling. Absolute wall-clock in a permanent suite is a
    // flaky test on a loaded CI box, and a flaky performance test gets deleted
    // rather than investigated -- leaving nothing. Both numbers come from the
    // same process and the same run, so their relationship is stable even when
    // the machine is not.
    //
    // What this catches is the buffer going quadratic again: it would stop being
    // the small half. Measured at roughly 4ms against 38ms when written.
    expect(r.newBufferMs, `buffer ${r.newBufferMs}ms, model ${r.screenOnlyMs}ms`).toBeLessThan(
      r.screenOnlyMs
    )
  })
}, 300_000)
