import fs from 'node:fs'
import path from 'node:path'
import { describe, it, expect } from 'vitest'
import { runMeasurement } from './helpers/run-measurement'
import { spawnsRealServers } from './helpers/one-at-a-time'

/**
 * What fifty screen models cost, and whether they are given back.
 *
 * Every PTY carries a screen model, so the question "what does this
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
  heldRss: number
  rssAfterFirst: number
  rssAfterSecond: number
}

/**
 * The model lives in libghostty-vt, outside V8's heap, so the budget is checked
 * against RSS, and a leak shows as RSS that keeps growing on the second cycle
 * instead of being reused from the first. Runs where `yarn build:core` has
 * produced the binary.
 */
const builtCore = fs.existsSync(path.resolve(__dirname, '../packages/core/vorn_core.node'))

describe.runIf(builtCore)(
  'fifty sessions',
  () => {
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
      // What would matter is a change that made this an order of magnitude
      // worse, such as giving the model the client's two thousand lines of
      // scrollback.
      expect(result.heldRss, `held ${mb(result.heldRss)} RSS for ${result.sessions}`).toBeLessThan(
        32 * 1024 * 1024
      )
    })

    it('reuses what it gave back', () => {
      // An allocator keeps freed pages, so RSS after release says little. What a
      // leak does is make the second identical cycle need fresh pages again.
      const growth = result.rssAfterSecond - result.rssAfterFirst
      expect(growth, `grew ${mb(growth)} on the second cycle`).toBeLessThan(result.heldRss / 8)
    })
  },
  180_000
)
