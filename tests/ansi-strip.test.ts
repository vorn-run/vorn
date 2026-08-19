import { describe, it, expect } from 'vitest'
import { stripAnsi } from '../packages/server/src/ansi-strip'

const ESC = '\x1b'

/**
 * A PTY emits drawing instructions, not text. Everything that reads the stored
 * line buffer — the status parser, agent history, a client asking for a session's
 * tail — reads whatever survives this function.
 */
describe('stripping escape sequences', () => {
  it('removes colour', () => {
    expect(stripAnsi(`${ESC}[31mred${ESC}[0m`)).toBe('red')
  })

  it('removes the cursor hide and show a TUI wraps every repaint in', () => {
    // The pattern used to accept only digits and semicolons as parameters, so
    // `?25l` never matched and a literal `[?25l` was stored. The ESC renders as
    // nothing, so it surfaced as bracket noise through otherwise fine text.
    expect(stripAnsi(`${ESC}[?25lWorking${ESC}[?25h`)).toBe('Working')
  })

  it.each([
    ['bracketed paste on', `${ESC}[?2004h`],
    ['bracketed paste off', `${ESC}[?2004l`],
    ['alternate screen', `${ESC}[?1049h`],
    ['mouse tracking', `${ESC}[?1000h`],
    ['cursor position report', `${ESC}[6n`],
    ['erase to end of line', `${ESC}[K`],
    ['move cursor', `${ESC}[12;40H`],
    ['scroll region', `${ESC}[1;24r`],
    ['256 colour', `${ESC}[38;5;213m`],
    ['soft reset', `${ESC}[!p`]
  ])('removes %s', (_label, sequence) => {
    expect(stripAnsi(`a${sequence}b`)).toBe('ab')
  })

  it('removes a window title, which carries text that would otherwise be kept', () => {
    expect(stripAnsi(`${ESC}]0;some title\x07done`)).toBe('done')
  })

  it('removes a hyperlink wrapper but keeps the label', () => {
    expect(stripAnsi(`${ESC}]8;;https://example.com\x07label${ESC}]8;;\x07`)).toBe('label')
  })

  it('removes charset selection and single-character escapes', () => {
    expect(stripAnsi(`${ESC}(Bplain${ESC}=x${ESC}>y`)).toBe('plainxy')
  })

  it('leaves ordinary text alone, brackets included', () => {
    expect(stripAnsi('a [not] an escape [?25l-ish')).toBe('a [not] an escape [?25l-ish')
  })
})

/**
 * The half that made stored output unreadable even where escapes were handled.
 */
describe('carriage returns', () => {
  it('lets a redraw overwrite rather than concatenate', () => {
    // Deleting the \r instead turned a spinner redrawing one line into
    // `WoWorkWorking`, which is what a session tail actually looked like.
    expect(stripAnsi('Wo\rWork\rWorking')).toBe('Working')
  })

  it('keeps a CRLF line ending intact', () => {
    // Read as an overwrite, `abc\r\n` would delete the line it terminates.
    expect(stripAnsi('abc\r\ndef\r\n')).toBe('abc\ndef\n')
  })

  it('applies the rule per line, not across the chunk', () => {
    expect(stripAnsi('aa\rbb\ncc\rdd')).toBe('bb\ndd')
  })

  it('keeps a line with no carriage return in it', () => {
    expect(stripAnsi('plain line')).toBe('plain line')
  })

  it('handles a progress bar redrawn many times', () => {
    const frames = ['10%', '40%', '80%', '100%'].join('\r')
    expect(stripAnsi(frames)).toBe('100%')
  })

  it('drops a line that was erased and not redrawn', () => {
    // What the reader would see on a real terminal: nothing.
    expect(stripAnsi('transient\r')).toBe('')
  })
})

describe('the two together', () => {
  it('reduces a real repaint to what a person would have read', () => {
    const raw =
      `${ESC}[?25l${ESC}[2K\rWo${ESC}[?25h` +
      `${ESC}[?25l${ESC}[2K\rWork${ESC}[?25h` +
      `${ESC}[?25l${ESC}[2K\rWorking${ESC}[?25h`
    expect(stripAnsi(raw)).toBe('Working')
  })
})
