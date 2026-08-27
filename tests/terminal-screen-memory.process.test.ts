import { describe, it, expect } from 'vitest'
import { runMeasurement } from './helpers/run-measurement'
import { spawnsRealServers } from './helpers/one-at-a-time'

/**
 * What fifty screen models cost, and whether they are given back.
 *
 * Every PTY now carries a headless emulator, so the question "what does this
 * cost at scale" has an answer that has to be measured rather than reasoned
 * about — the whole reason the model runs with no scrollback is a claim about
 * memory, and a claim about memory is worth what its measurement is worth.
 *
 * Measured in a child process, not here. This worker's heap holds the module
 * graph, React and whatever the rest of the suite left behind, and that noise is
 * larger than the thing being measured. The child runs with `--expose-gc`,
 * because without it there is no way to tell "released" from "not collected
 * yet", and a number that cannot tell those apart is not a measurement.
 *
 * The assertions are a ceiling and a release, never a point value. Heap numbers
 * move with the Node version and the machine, and a tight assertion here would
 * be a flaky test — which in this position gets deleted within a month, leaving
 * nothing at all.
 */

// Takes the same lock the server suites take. This one measures wall-clock and
// heap, so anything spawning beside it is measuring something else.
spawnsRealServers()

interface Measurement {
  sessions: number
  cols: number
  rows: number
  modelled: number
  remaining: number
  heldBytes: number
  residualFirst: number
  residualSecond: number
}

describe('fifty sessions', () => {
  const result = runMeasurement<Measurement>('measure-screens.ts', {
    env: { NODE_OPTIONS: '--expose-gc' },
    timeoutMs: 180_000
  })
  const mb = (bytes: number): string => `${(bytes / 1024 / 1024).toFixed(1)} MB`

  it('models every one of them', () => {
    expect(result.modelled).toBe(result.sessions)
    expect(result.remaining).toBe(0)
  })

  it('costs an amount worth paying', () => {
    // Roughly 8 MB when this was written, at 200x50 with realistic coloured
    // output. The ceiling is generous on purpose: what would matter is a change
    // that made this an order of magnitude worse -- giving the model the
    // client's two thousand lines of scrollback, say, which computes to around
    // a quarter of a gigabyte here.
    expect(result.heldBytes, `held ${mb(result.heldBytes)} for ${result.sessions}`).toBeLessThan(
      32 * 1024 * 1024
    )
  })

  it('gives it back when the terminals go', () => {
    // What this catches is a retained *reference* -- a `clearScreen` that stopped
    // removing the entry from the map would keep every session ever closed
    // resident, and this reports 3.9 MB against 51 KB when that happens.
    //
    // What it does not catch, checked rather than assumed: a missing
    // `term.dispose()`. Dropping the map entry leaves the terminal unreachable
    // and V8 collects it either way, so dispose earns its place by releasing
    // xterm's internal listeners rather than by returning heap. Worth saying,
    // because the obvious reading of this test is the wrong one.
    //
    // Measured on the second cycle rather than the first. The first leaves
    // several hundred kilobytes behind whatever happens -- module init, lazy V8
    // structures, xterm's own one-time setup -- and a leak is a thing that
    // accumulates, so a second identical cycle is what tells them apart. It came
    // back to about 50 KB against 8 MB held.
    expect(
      result.residualSecond,
      `held ${mb(result.heldBytes)}, kept ${mb(result.residualSecond)} after release`
    ).toBeLessThan(result.heldBytes / 8)
  })
}, 180_000)
