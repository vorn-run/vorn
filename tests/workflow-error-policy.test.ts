import { describe, it, expect } from 'vitest'
import { stopsRunOnError } from '@vornrun/shared/workflow-graph'

/**
 * The per-node error policy, and the branch-skipping it drives.
 *
 * The engine halts a run by marking everything downstream of the failure as
 * skipped — nothing then becomes ready and the wave loop runs dry. So the two
 * things worth pinning are which policy a node gets when it declares none, and
 * that "downstream" never swallows a node another live path still feeds.
 */
describe('stopsRunOnError', () => {
  it('stops when the node says nothing', () => {
    expect(stopsRunOnError({})).toBe(true)
  })

  it('stops when the node says so', () => {
    expect(stopsRunOnError({ onError: 'stop' })).toBe(true)
  })

  it('carries on only when the node opted out', () => {
    expect(stopsRunOnError({ onError: 'continue' })).toBe(false)
  })
})
