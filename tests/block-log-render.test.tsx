// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { render, screen, fireEvent, cleanup, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { BlockLog } from '../src/renderer/components/BlockLog'
import { captureBlock, clearBlockLog } from '../src/renderer/lib/block-log'
import type { BlockRow } from '../src/renderer/lib/block-render'

/**
 * Finished commands drawn as elements. This is what makes a block a real
 * container — its padding, boundary and per-block copy are CSS rather than
 * something approximated inside the character grid.
 */

function row(text: string, style: Partial<BlockRow['runs'][0]> = {}): BlockRow {
  return {
    runs: [
      {
        text,
        fg: { kind: 'default' },
        bg: { kind: 'default' },
        bold: false,
        dim: false,
        italic: false,
        underline: false,
        strikethrough: false,
        inverse: false,
        ...style
      }
    ]
  }
}

/** Drives capture through a buffer stub rather than a real terminal. */
function seed(opts: {
  command: string | null
  rows: BlockRow[]
  exitCode?: number
  durationMs?: number
  cwd?: string | null
}): void {
  const lines = opts.rows
  captureBlock({
    terminalId: 't',
    buffer: {
      getLine: (y: number) =>
        lines[y]
          ? {
              length: 1,
              getCell: (x: number) =>
                x === 0
                  ? {
                      getChars: () => lines[y].runs[0].text,
                      getWidth: () => 1,
                      isBold: () => (lines[y].runs[0].bold ? 1 : 0),
                      isDim: () => 0,
                      isItalic: () => 0,
                      isUnderline: () => 0,
                      isStrikethrough: () => 0,
                      isInverse: () => 0,
                      isFgDefault: () => true,
                      isBgDefault: () => true,
                      isFgPalette: () => false,
                      isBgPalette: () => false,
                      isFgRGB: () => false,
                      isBgRGB: () => false,
                      getFgColor: () => 0,
                      getBgColor: () => 0
                    }
                  : undefined
            }
          : undefined
    } as never,
    startLine: 0,
    endLine: lines.length - 1,
    command: opts.command,
    exitCode: opts.exitCode ?? 0,
    durationMs: opts.durationMs ?? 120,
    cwd: opts.cwd ?? null
  })
}

beforeEach(() => {
  clearBlockLog('t')
  // jsdom has no layout, so the tail-following scroll is a no-op here.
  Element.prototype.scrollIntoView = vi.fn()
  Object.defineProperty(navigator, 'clipboard', {
    value: { writeText: vi.fn(() => Promise.resolve()) },
    configurable: true
  })
})
afterEach(() => cleanup())

describe('BlockLog', () => {
  it('draws nothing at all before a command has finished', () => {
    // An empty log must not reserve space, or a fresh session opens on a band
    // of nothing above the prompt.
    const { container } = render(<BlockLog terminalId="t" />)
    expect(container).toBeEmptyDOMElement()
  })

  it('titles a block with the command and shows its output below', () => {
    seed({ command: 'ls', rows: [row('ls'), row('LICENSE'), row('README.md')] })
    render(<BlockLog terminalId="t" />)
    expect(screen.getByText('ls')).toBeInTheDocument()
    expect(screen.getByText('LICENSE')).toBeInTheDocument()
  })

  it('shows the exit code only when the command failed', () => {
    seed({ command: 'false', rows: [row('false')], exitCode: 1 })
    render(<BlockLog terminalId="t" />)
    expect(screen.getByText('exit 1')).toBeInTheDocument()
  })

  it('leaves a successful block unlabelled, so failure is what stands out', () => {
    seed({ command: 'true', rows: [row('true')], exitCode: 0 })
    render(<BlockLog terminalId="t" />)
    expect(screen.queryByText(/^exit /)).toBeNull()
  })

  it('titles a block by its first row when the shell cannot report the command', () => {
    // cmd.exe cannot name the command, so it stays where the user typed it.
    // A "(command)" placeholder would print above the real thing.
    seed({ command: null, rows: [row('C:\\> dir'), row('Volume in drive C')] })
    render(<BlockLog terminalId="t" />)
    expect(screen.getByText('C:\\> dir')).toBeInTheDocument()
    expect(screen.queryByText('(command)')).toBeNull()
  })

  it('copies the block output rather than the heading', () => {
    seed({ command: 'ls', rows: [row('ls'), row('LICENSE')] })
    render(<BlockLog terminalId="t" />)
    fireEvent.click(screen.getByLabelText('Copy output'))
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(expect.stringContaining('LICENSE'))
  })

  it('appends a new block as commands finish', () => {
    seed({ command: 'first', rows: [row('first')] })
    render(<BlockLog terminalId="t" />)
    act(() => seed({ command: 'second', rows: [row('second')] }))
    expect(screen.getByText('first')).toBeInTheDocument()
    expect(screen.getByText('second')).toBeInTheDocument()
  })

  it('shows the directory a command ran in', () => {
    seed({ command: 'ls', rows: [row('ls')], cwd: '/Users/j/dev/vorn' })
    render(<BlockLog terminalId="t" />)
    expect(screen.getByText(/vorn/)).toBeInTheDocument()
  })
})
