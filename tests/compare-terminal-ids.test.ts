import { describe, it, expect } from 'vitest'
import { compareTerminalIds } from '../packages/web/src/renderer/hooks/useVisibleTerminals'
import type { TerminalState } from '../packages/web/src/renderer/stores/types'

function termState(createdAt: number, lastOutput: number): TerminalState {
  return {
    session: {
      id: 'irrelevant',
      projectName: 'p',
      createdAt
    },
    status: 'idle',
    lastOutputTimestamp: lastOutput
  } as unknown as TerminalState
}

const TERMS = new Map<string, TerminalState>([
  ['a', termState(1000, 5000)],
  ['b', termState(2000, 4000)],
  ['c', termState(3000, 3000)]
])

describe('compareTerminalIds', () => {
  it('manual mode orders ids by their position in terminalOrder', () => {
    const order = ['b', 'a', 'c']
    const ids = ['c', 'a', 'b']
      .slice()
      .sort((x, y) => compareTerminalIds(x, y, TERMS, 'manual', order))
    expect(ids).toEqual(['b', 'a', 'c'])
  })

  it('manual mode pushes ids missing from terminalOrder to the end (no NaN)', () => {
    const order = ['a']
    const ids = ['c', 'b', 'a']
      .slice()
      .sort((x, y) => compareTerminalIds(x, y, TERMS, 'manual', order))
    // 'a' is in the order list, 'b' and 'c' are not — they keep relative order
    // (Array#sort in V8 is stable since ES2019).
    expect(ids[0]).toBe('a')
    expect(ids.slice(1).sort()).toEqual(['b', 'c'])
  })

  it('manual mode treats two missing ids as equal (no Infinity - Infinity)', () => {
    const order: string[] = []
    expect(compareTerminalIds('a', 'b', TERMS, 'manual', order)).toBe(0)
    expect(compareTerminalIds('b', 'a', TERMS, 'manual', order)).toBe(0)
  })

  it('created mode orders newest first by session.createdAt', () => {
    const ids = ['a', 'b', 'c']
      .slice()
      .sort((x, y) => compareTerminalIds(x, y, TERMS, 'created', []))
    expect(ids).toEqual(['c', 'b', 'a'])
  })

  it('recent mode orders most-recently-active first by lastOutputTimestamp', () => {
    const ids = ['a', 'b', 'c']
      .slice()
      .sort((x, y) => compareTerminalIds(x, y, TERMS, 'recent', []))
    expect(ids).toEqual(['a', 'b', 'c'])
  })

  it('treats unknown terminal ids as last while keeping known ids ordered', () => {
    const ids = ['a', 'ghost', 'c']
      .slice()
      .sort((x, y) => compareTerminalIds(x, y, TERMS, 'created', []))
    expect(ids[ids.length - 1]).toBe('ghost')
    expect(ids.slice(0, 2)).toEqual(['c', 'a'])
  })

  it('merging visible + minimized ids produces a sortMode-correct list (TabView allTabIds)', () => {
    // Simulates TabView merging orderedIds + minimizedIds and re-sorting via
    // the shared comparator under the active sortMode.
    const visible = ['a', 'c']
    const minimized = ['b']
    const merged = [...visible, ...minimized].sort((x, y) =>
      compareTerminalIds(x, y, TERMS, 'created', [])
    )
    expect(merged).toEqual(['c', 'b', 'a'])
  })
})
