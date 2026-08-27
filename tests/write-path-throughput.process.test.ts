import { describe, it, expect } from 'vitest'
import { spawnSync } from 'node:child_process'
import path from 'node:path'

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
 * The ceilings are generous and the comparison is relative. Wall-clock on a
 * loaded CI box is not a number to assert tightly, and a flaky performance test
 * gets deleted rather than investigated. What these catch is a change of shape —
 * a per-chunk cost that goes back to being quadratic, or a model that becomes an
 * order of magnitude more expensive to feed.
 */

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

function measure(): Measurement {
  const script = path.join(__dirname, 'helpers', 'measure-write-path.ts')
  const run = spawnSync('npx', ['tsx', script], {
    cwd: path.join(__dirname, '..'),
    encoding: 'utf-8',
    timeout: 300_000
  })
  if (run.status !== 0) throw new Error(`measurement failed (${run.status}):\n${run.stderr}`)
  return JSON.parse(run.stdout.trim().split('\n').pop() ?? '') as Measurement
}

describe('the path every byte of output takes', () => {
  const r = measure()

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

  it('holds the buffer to a flat cost per chunk', () => {
    // The old shape was quadratic in disguise: every append re-formed the whole
    // buffer, so the cost per chunk grew with the buffer until it hit the cap and
    // stayed there, expensively. Roughly 0.2µs/chunk now.
    expect(r.newBufferMs, `${r.newBufferMs}ms for ${r.chunks} chunks`).toBeLessThan(200)
  })

  it('keeps the screen model affordable to feed', () => {
    // About 1.9µs per chunk when this was written — a real VT parse, and the
    // reason the model is fed from the 8ms coalescer rather than from every raw
    // PTY write. An order of magnitude worse than this would be worth knowing
    // about; a doubling would not.
    expect(r.screenOnlyMs, `${r.screenOnlyMs}ms for ${r.chunks} chunks`).toBeLessThan(400)
  })
}, 300_000)
