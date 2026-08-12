import { describe, it, expect } from 'vitest'
import {
  stopsRunOnError,
  buildGraph,
  collectSkippedBranch
} from '../src/renderer/lib/workflow-execution'

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

describe('collectSkippedBranch', () => {
  const skipFrom = (
    edges: Array<{ source: string; target: string }>,
    start: string,
    terminal: string[] = []
  ) => {
    const { successors, predecessors } = buildGraph(edges)
    return collectSkippedBranch(start, successors, predecessors, (id) => terminal.includes(id))
  }

  it('takes the whole chain below a failure', () => {
    const skipped = skipFrom(
      [
        { source: 'a', target: 'b' },
        { source: 'b', target: 'c' },
        { source: 'c', target: 'd' }
      ],
      'b'
    )
    expect([...skipped].sort()).toEqual(['b', 'c', 'd'])
  })

  it('leaves a join alone while another predecessor is still live', () => {
    // a → b → join, and a → c → join. `b` failed; `c` has not run yet, so the
    // join is still reachable and must not be skipped out from under it.
    const skipped = skipFrom(
      [
        { source: 'a', target: 'b' },
        { source: 'a', target: 'c' },
        { source: 'b', target: 'join' },
        { source: 'c', target: 'join' }
      ],
      'b'
    )
    expect([...skipped]).toEqual(['b'])
    expect(skipped.has('join')).toBe(false)
  })

  it('takes the join once every other predecessor has settled', () => {
    const skipped = skipFrom(
      [
        { source: 'a', target: 'b' },
        { source: 'a', target: 'c' },
        { source: 'b', target: 'join' },
        { source: 'c', target: 'join' }
      ],
      'b',
      ['c']
    )
    expect([...skipped].sort()).toEqual(['b', 'join'])
  })

  it('does not revisit a node on a cycle', () => {
    const skipped = skipFrom(
      [
        { source: 'a', target: 'b' },
        { source: 'b', target: 'a' }
      ],
      'a'
    )
    expect([...skipped].sort()).toEqual(['a', 'b'])
  })
})
