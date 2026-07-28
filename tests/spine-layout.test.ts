import { describe, it, expect } from 'vitest'
import type { CommandBlock, MarkerLike } from '../src/renderer/lib/command-blocks'
import {
  computeSpineMarks,
  isRoutine,
  MIN_MARK_PX,
  type BufferMetrics
} from '../src/renderer/lib/spine-layout'

function marker(line: number, disposed = false): MarkerLike {
  return { line, isDisposed: disposed, dispose: () => {}, onDispose: () => {} }
}

function block(over: Partial<CommandBlock> & { line: number }): CommandBlock {
  const { line, ...rest } = over
  return {
    command: 'cmd',
    exitCode: 0,
    durationMs: 100,
    outputLines: 1,
    marker: marker(line),
    ...rest
  }
}

/**
 * A 24-row viewport at the top of the buffer. Paired with a spine height of
 * 240 below, one row is exactly 10px, so positions can be asserted directly.
 */
const metrics = (over: Partial<BufferMetrics> = {}): BufferMetrics => ({
  length: 101,
  viewportY: 0,
  baseY: 0,
  rows: 24,
  cursorLine: 23,
  isAlternate: false,
  ...over
})

const H = 240

describe('computeSpineMarks', () => {
  it('places each mark beside the rows it stands for', () => {
    // The whole point of the gutter: a mark sits next to its own block, not
    // at some fraction of the entire session.
    const marks = computeSpineMarks(
      [block({ line: 0 }), block({ line: 8 }), block({ line: 16 })],
      metrics(),
      H
    )
    expect(marks.map((m) => Math.round(m.y))).toEqual([0, 80, 160])
  })

  it('sizes a mark to the rows its block occupies', () => {
    const marks = computeSpineMarks([block({ line: 0 }), block({ line: 8 })], metrics(), H)
    // Rows 0-7 is 8 rows at 10px, less the 2px separation.
    expect(Math.round(marks[0].height)).toBe(78)
    // The last block runs to the end of what has been written (row 23).
    expect(Math.round(marks[1].height)).toBe(158)
  })

  it('gives each block the rows up to the next one', () => {
    const marks = computeSpineMarks(
      [block({ line: 0 }), block({ line: 8 }), block({ line: 16 })],
      metrics(),
      H
    )
    expect(marks.map((m) => m.endLine)).toEqual([7, 15, 23])
  })

  it('drops blocks whose output ends above the viewport', () => {
    // Blocks at 0 and 5 end at rows 4 and 9, both fully scrolled past.
    const marks = computeSpineMarks(
      [block({ line: 0 }), block({ line: 5 }), block({ line: 10 })],
      metrics({ viewportY: 28, cursorLine: 40 }),
      H
    )
    expect(marks).toHaveLength(1)
    expect(marks[0].line).toBe(10)
  })

  it('keeps a block whose output still reaches into view', () => {
    // Its command scrolled off, but the tail of its output has not.
    const marks = computeSpineMarks(
      [block({ line: 5 }), block({ line: 30 })],
      metrics({ viewportY: 28, cursorLine: 40 }),
      H
    )
    expect(marks.map((m) => m.line)).toEqual([5, 30])
  })

  it('clips a block straddling the top edge', () => {
    // Its command scrolled off, but its output is still on screen.
    const marks = computeSpineMarks(
      [block({ line: 4 })],
      metrics({ viewportY: 10, cursorLine: 20 }),
      H
    )
    expect(marks).toHaveLength(1)
    expect(Math.round(marks[0].y)).toBe(0)
  })

  it('follows the viewport as it scrolls', () => {
    const blocks = [block({ line: 12 })]
    const atTop = computeSpineMarks(blocks, metrics({ viewportY: 0, cursorLine: 30 }), H)
    const scrolled = computeSpineMarks(blocks, metrics({ viewportY: 6, cursorLine: 30 }), H)
    expect(Math.round(atTop[0].y)).toBe(120)
    expect(Math.round(scrolled[0].y)).toBe(60)
  })

  it('returns nothing on the alternate buffer', () => {
    // Markers reference the normal buffer, so they point at unrelated rows
    // while a full-screen application is up.
    expect(computeSpineMarks([block({ line: 10 })], metrics({ isAlternate: true }), H)).toEqual([])
  })

  it('returns nothing before the spine has been measured', () => {
    expect(computeSpineMarks([block({ line: 10 })], metrics(), 0)).toEqual([])
  })

  it('skips disposed markers', () => {
    const stale = block({ line: 10 })
    stale.marker = marker(10, true)
    expect(computeSpineMarks([stale], metrics(), H)).toEqual([])
  })

  it('never draws a bar shorter than the minimum', () => {
    // A one-row block in a short card would otherwise be invisible.
    const marks = computeSpineMarks(
      [block({ line: 0 }), block({ line: 1 })],
      metrics({ cursorLine: 1 }),
      24
    )
    expect(marks[0].height).toBeGreaterThanOrEqual(MIN_MARK_PX)
  })

  it('includes the running command as its own mark', () => {
    const marks = computeSpineMarks(
      [],
      metrics(),
      H,
      { command: 'yarn build', since: 0, line: 5 },
      2500
    )
    expect(marks).toHaveLength(1)
    expect(marks[0].status).toBe('running')
    expect(marks[0].command).toBe('yarn build')
    expect(marks[0].durationMs).toBe(2500)
    expect(marks[0].routine).toBe(false)
  })

  it('orders a running command after the finished ones it follows', () => {
    const marks = computeSpineMarks([block({ line: 0 })], metrics(), H, {
      command: 'yarn build',
      since: 0,
      line: 10
    })
    expect(marks.map((m) => m.status)).toEqual(['ok', 'running'])
    // The finished block stops where the running one begins.
    expect(marks[0].endLine).toBe(9)
  })
})

describe('clustering', () => {
  // Rows only collide when the card is very short: at 24px for 24 rows, one
  // row is 1px.
  const tiny = 24

  it('merges marks that would overlap', () => {
    const marks = computeSpineMarks(
      [block({ line: 0 }), block({ line: 1 }), block({ line: 2 })],
      metrics(),
      tiny
    )
    expect(marks).toHaveLength(1)
    expect(marks[0].count).toBe(3)
  })

  it('lets a failure win its cluster', () => {
    const marks = computeSpineMarks(
      [
        block({ line: 0, command: 'ok-one' }),
        block({ line: 1, command: 'boom', exitCode: 1 }),
        block({ line: 2, command: 'ok-two' })
      ],
      metrics(),
      tiny
    )
    expect(marks).toHaveLength(1)
    expect(marks[0].status).toBe('fail')
    expect(marks[0].command).toBe('boom')
    expect(marks[0].exitCode).toBe(1)
  })

  it('covers every merged block, so hovering highlights all of them', () => {
    const marks = computeSpineMarks(
      [block({ line: 0 }), block({ line: 1 }), block({ line: 2 })],
      metrics(),
      tiny
    )
    expect(marks[0].endLine).toBe(23)
  })

  it('only calls a cluster routine when every member is', () => {
    const marks = computeSpineMarks(
      [block({ line: 0 }), block({ line: 1, durationMs: 9000, outputLines: 400 })],
      metrics(),
      tiny
    )
    expect(marks[0].routine).toBe(false)
  })

  it('keeps marks separate when there is room for both', () => {
    const marks = computeSpineMarks([block({ line: 0 }), block({ line: 8 })], metrics(), H)
    expect(marks).toHaveLength(2)
  })
})

describe('isRoutine', () => {
  it('accepts a fast, quiet success', () => {
    expect(isRoutine({ exitCode: 0, durationMs: 200, outputLines: 1 })).toBe(true)
  })

  it.each([
    ['a failure', { exitCode: 1, durationMs: 200, outputLines: 1 }],
    ['a slow command', { exitCode: 0, durationMs: 2001, outputLines: 1 }],
    ['a chatty command', { exitCode: 0, durationMs: 200, outputLines: 6 }]
  ])('rejects %s', (_label, input) => {
    expect(isRoutine(input)).toBe(false)
  })

  it('accepts the boundary values', () => {
    expect(isRoutine({ exitCode: 0, durationMs: 2000, outputLines: 5 })).toBe(true)
  })
})
