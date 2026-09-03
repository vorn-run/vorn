// @vitest-environment jsdom
import { describe, it, expect, beforeEach } from 'vitest'
import type { CommandBlock } from '../src/renderer/lib/command-blocks'
import {
  chooseAnchor,
  pruneScrollAnchors,
  readScrollAnchor,
  resolveAnchor,
  writeScrollAnchor
} from '../src/renderer/lib/scroll-anchor'

const KEY = 'vorn:scrollAnchors'

beforeEach(() => {
  localStorage.clear()
})

function block(command: string, line: number, isDisposed = false): CommandBlock {
  return {
    command,
    exitCode: 0,
    durationMs: 10,
    cwd: null,
    outputLines: 4,
    marker: { line, isDisposed, dispose: () => {}, onDispose: () => {} }
  }
}

/** yarn build at row 100, yarn test at 200, git status at 300. */
const blocks = [block('yarn build', 100), block('yarn test', 200), block('git status', 300)]

describe('choosing what a scrolled-up pane was reading', () => {
  it('names the command the top of the viewport sits in', () => {
    expect(chooseAnchor(blocks, { viewportY: 250, baseY: 400, isAlternate: false })).toEqual({
      fromEnd: 1,
      command: 'yarn test'
    })
  })

  it('counts from the newest, so a trimmed replay still resolves it', () => {
    expect(chooseAnchor(blocks, { viewportY: 320, baseY: 400, isAlternate: false })?.fromEnd).toBe(
      0
    )
  })

  it('anchors nothing when the pane is following the output', () => {
    expect(chooseAnchor(blocks, { viewportY: 400, baseY: 400, isAlternate: false })).toBeNull()
  })

  it('anchors nothing above the first command', () => {
    expect(chooseAnchor(blocks, { viewportY: 40, baseY: 400, isAlternate: false })).toBeNull()
  })

  it('anchors nothing in the alternate buffer, which has no commands to name', () => {
    expect(chooseAnchor(blocks, { viewportY: 10, baseY: 400, isAlternate: true })).toBeNull()
  })

  it('anchors nothing for a shell with no integration at all', () => {
    expect(chooseAnchor([], { viewportY: 250, baseY: 400, isAlternate: false })).toBeNull()
  })

  it('skips a marker the terminal has dropped', () => {
    const dropped = [block('yarn build', 100), block('yarn test', 200, true)]
    expect(chooseAnchor(dropped, { viewportY: 250, baseY: 400, isAlternate: false })).toEqual({
      fromEnd: 1,
      command: 'yarn build'
    })
  })
})

describe('resolving an anchor against the screen that came back', () => {
  it('finds the row even when the replay was trimmed at the top', () => {
    const trimmed = [block('yarn test', 12), block('git status', 90)]
    expect(resolveAnchor(trimmed, { fromEnd: 1, command: 'yarn test' })).toBe(12)
  })

  it('gives up rather than guessing when the count names another command', () => {
    expect(resolveAnchor(blocks, { fromEnd: 1, command: 'yarn lint' })).toBeNull()
  })

  it('gives up when the replay is shorter than the count', () => {
    expect(
      resolveAnchor([block('git status', 20)], { fromEnd: 4, command: 'yarn build' })
    ).toBeNull()
  })

  it('gives up on a session that came back with no commands', () => {
    expect(resolveAnchor([], { fromEnd: 0, command: 'yarn test' })).toBeNull()
  })

  it('does nothing without an anchor', () => {
    expect(resolveAnchor(blocks, null)).toBeNull()
  })
})

describe('keeping the anchors', () => {
  it('remembers one per session', () => {
    writeScrollAnchor('term-1', { fromEnd: 2, command: 'yarn dev' })
    writeScrollAnchor('term-2', { fromEnd: 0, command: 'git log' })
    expect(readScrollAnchor('term-1')).toEqual({ fromEnd: 2, command: 'yarn dev' })
    expect(readScrollAnchor('term-2')?.fromEnd).toBe(0)
  })

  it('lets go when the pane scrolls back to the bottom', () => {
    writeScrollAnchor('term-1', { fromEnd: 2, command: 'yarn dev' })
    writeScrollAnchor('term-1', null)
    expect(readScrollAnchor('term-1')).toBeNull()
  })

  it('drops the ones whose session never came back', () => {
    writeScrollAnchor('term-1', { fromEnd: 1, command: 'yarn dev' })
    writeScrollAnchor('term-2', { fromEnd: 1, command: 'git log' })
    pruneScrollAnchors(new Set(['term-1']))
    expect(readScrollAnchor('term-1')).not.toBeNull()
    expect(readScrollAnchor('term-2')).toBeNull()
  })

  it('survives a stored value that is not the shape it expects', () => {
    for (const raw of ['{', '[]', 'null', '"term-1"', '{"term-1":{"fromEnd":"2"}}']) {
      localStorage.setItem(KEY, raw)
      expect(() => readScrollAnchor('term-1')).not.toThrow()
      expect(readScrollAnchor('term-1')).toBeNull()
    }
  })

  it('rejects a count that could not have come from here', () => {
    localStorage.setItem(KEY, JSON.stringify({ 'term-1': { fromEnd: -3, command: 'x' } }))
    expect(readScrollAnchor('term-1')).toBeNull()
  })
})
