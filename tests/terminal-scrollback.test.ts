import { describe, it, expect, beforeEach } from 'vitest'
import {
  appendScrollback,
  clearScrollback,
  readScrollback,
  resetScrollback
} from '../packages/server/src/terminal-scrollback'

const ESC = '\x1b'

beforeEach(resetScrollback)

/**
 * A client attaching a terminal emulator to a live session needs the bytes as
 * they were emitted. The stripped line buffer cannot serve that: it answers what
 * an agent said, and the escape sequences it removes are exactly what a terminal
 * needs in order to draw.
 */
describe('what a terminal needs on attach', () => {
  it('keeps escape sequences, which the stripped buffer removes', () => {
    appendScrollback('a', `${ESC}[31mred${ESC}[0m`)

    expect(readScrollback('a')).toBe(`${ESC}[31mred${ESC}[0m`)
  })

  it('keeps carriage returns, so a repaint still repaints', () => {
    appendScrollback('a', 'Wo\rWork\rWorking')

    expect(readScrollback('a')).toBe('Wo\rWork\rWorking')
  })

  it('joins chunks in the order they arrived', () => {
    appendScrollback('a', 'one ')
    appendScrollback('a', 'two')

    expect(readScrollback('a')).toBe('one two')
  })

  it('keeps terminals apart', () => {
    appendScrollback('a', 'first')
    appendScrollback('b', 'second')

    expect(readScrollback('a')).toBe('first')
    expect(readScrollback('b')).toBe('second')
  })

  it('reads a terminal that has produced nothing as empty', () => {
    expect(readScrollback('never-seen')).toBe('')
  })

  it('forgets a terminal that exited', () => {
    // Or a long-lived server accumulates a buffer per session it ever ran.
    appendScrollback('a', 'output')

    clearScrollback('a')

    expect(readScrollback('a')).toBe('')
  })
})

describe('staying bounded', () => {
  it('drops the oldest output rather than growing', () => {
    appendScrollback('a', 'x'.repeat(400_000))

    expect(readScrollback('a').length).toBeLessThanOrEqual(256 * 1024)
  })

  it('keeps the most recent output, which is the part being drawn', () => {
    appendScrollback('a', `${'x'.repeat(400_000)}\nthe latest line`)

    expect(readScrollback('a').endsWith('the latest line')).toBe(true)
  })

  it('cuts at a line boundary so no escape sequence is severed', () => {
    // Half a sequence makes an emulator swallow the text after it or print the
    // sequence as literal characters, which is worse than losing the text.
    appendScrollback('a', `${'x'.repeat(300_000)}\n${ESC}[31mred${ESC}[0m\n`)
    const kept = readScrollback('a')

    expect(kept.startsWith(ESC)).toBe(true)
    expect(kept).toContain(`${ESC}[31mred${ESC}[0m`)
  })

  it('still bounds a single line with no boundary to cut at', () => {
    // A progress bar redrawing with carriage returns and never a newline.
    appendScrollback('a', 'y'.repeat(400_000))

    expect(readScrollback('a').length).toBe(256 * 1024)
  })

  it('bounds across many small appends, not just one large one', () => {
    for (let i = 0; i < 1000; i++) appendScrollback('a', 'z'.repeat(1000))

    expect(readScrollback('a').length).toBeLessThanOrEqual(256 * 1024)
  })
})
