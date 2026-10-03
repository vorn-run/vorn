import fs from 'node:fs'
import path from 'node:path'
import { describe, it, expect } from 'vitest'
import * as headless from '@xterm/headless'
import * as serializeAddon from '@xterm/addon-serialize'
import { loadNativeCore, type NativeScreen } from '../packages/server/src/native-core'
import { BLANKS_AS_SPACES, PALETTE_AS_256 } from './helpers/screen-parity'

/**
 * The native screen model against the headless xterm it replaces, per feature.
 *
 * Each case feeds the same bytes to both, then replays each one's serialized
 * screen into a fresh xterm and compares what that xterm shows: the visible
 * rows, the cursor, and every cell's style. That is what a restore does with a
 * checkpoint, so it is the parity that matters, and it does not depend on two
 * formatters choosing the same escape sequences. Differences that are accepted
 * are named in `helpers/screen-parity.ts` and normalized here by that name.
 *
 * Runs only where `yarn build:core` has produced the binary with libghostty-vt.
 */

function interop<T>(mod: unknown): T {
  return (mod as { default?: T })?.default ?? (mod as T)
}
const { Terminal } = interop<typeof import('@xterm/headless')>(headless)
const { SerializeAddon } = interop<typeof import('@xterm/addon-serialize')>(serializeAddon)
type Term = InstanceType<typeof Terminal>

const builtCore = path.resolve(__dirname, '../packages/core/vorn_core.node')
const core = fs.existsSync(builtCore) ? loadNativeCore([builtCore]) : null

function write(term: Term, data: string): Promise<void> {
  return new Promise((resolve) => term.write(data, resolve))
}

function xterm(cols: number, rows: number): Term {
  return new Terminal({ cols, rows, scrollback: 0, allowProposedApi: true })
}

interface View {
  rows: string[]
  cursor: [number, number]
  /** One entry per cell: fg/bg colour (mode-normalized) and attributes. */
  styles: string[]
  alternate: boolean
}

/** What a terminal shows, in terms a person would notice. */
function view(term: Term): View {
  const buf = term.buffer.active
  const rows: string[] = []
  const styles: string[] = []
  for (let y = 0; y < term.rows; y++) {
    const line = buf.getLine(y)
    rows.push(line?.translateToString(true) ?? '')
    for (let x = 0; x < term.cols; x++) {
      const cell = line?.getCell(x)
      if (!cell || cell.getChars() === '') continue
      const style = [
        `${x},${y}`,
        colour(cell.isFgDefault(), cell.isFgRGB(), cell.getFgColor()),
        colour(cell.isBgDefault(), cell.isBgRGB(), cell.getBgColor()),
        cell.isBold(),
        cell.isItalic(),
        cell.isUnderline(),
        cell.isInverse(),
        cell.isDim(),
        cell.getWidth()
      ].join(' ')
      // BLANKS_AS_SPACES: an unstyled space is the same as an empty cell.
      if (cell.getChars() === ' ' && style === `${x},${y} default default 0 0 0 0 0 1`) continue
      styles.push(style)
    }
  }
  return {
    rows,
    cursor: [buf.cursorX, buf.cursorY],
    styles,
    alternate: buf.type === 'alternate'
  }
}

/**
 * `PALETTE_AS_256`: a palette colour is compared by its index, whether it was
 * set as a 16-colour or a 256-colour code, so `32` and `38;5;2` are equal.
 */
function colour(isDefault: boolean, isRgb: boolean, value: number): string {
  if (isDefault) return 'default'
  if (isRgb) return `rgb:${value}`
  return `palette:${value}`
}

async function replay(serialized: string, cols: number, rows: number): Promise<View> {
  const term = xterm(cols, rows)
  await write(term, serialized)
  const v = view(term)
  term.dispose()
  return v
}

async function both(
  input: string,
  cols = 40,
  rows = 8
): Promise<{ js: View; native: View; jsTitle: string; nativeTitle: string }> {
  const term = xterm(cols, rows)
  let jsTitle = ''
  term.onTitleChange((t) => (jsTitle = t))
  const serializer = new SerializeAddon()
  term.loadAddon(serializer)
  await write(term, input)
  const js = await replay(serializer.serialize(), cols, rows)
  term.dispose()

  const screen = new core!.Screen!(cols, rows) as NativeScreen
  screen.feed(input)
  const snap = screen.serialize()
  const native = await replay(snap.screen, cols, rows)
  screen.free()
  return { js, native, jsTitle, nativeTitle: snap.title }
}

const CASES: Array<{ name: string; input: string; cols?: number; rows?: number }> = [
  {
    name: 'coloured text, 16, 256 and true colour',
    input:
      '\x1b[32m✓\x1b[0m passed\r\n\x1b[1;31merror\x1b[0m: \x1b[38;5;208mwarn\x1b[0m ' +
      '\x1b[38;2;10;20;30mrgb\x1b[48;5;17m bg\x1b[0m\r\n\x1b[3;4;7mstyled\x1b[0m'
  },
  { name: 'wide characters', input: '漢字 and 😀 emoji\r\n全角テキスト' },
  {
    name: 'cursor movement and erase',
    input: 'first line\r\nsecond\x1b[1;5Hxx\x1b[2;1H\x1b[K\x1b[3;10Hhere'
  },
  {
    name: 'a status line redrawn in place',
    input: '⠋ Thinking (1s)\r\x1b[2K⠙ Thinking (2s)\r\x1b[2K⠹ Done'
  },
  {
    name: 'lines that wrap and scroll off',
    input: Array.from({ length: 12 }, (_, i) => `line ${i} `.repeat(6)).join('\r\n'),
    cols: 30,
    rows: 6
  },
  {
    name: 'the alternate screen',
    input: 'shell prompt $ \x1b[?1049h\x1b[2J\x1b[H\x1b[7m TUI header \x1b[0m\r\nbody'
  },
  {
    name: 'a scroll region',
    input: '\x1b[2;5r\x1b[2;1Ha\r\nb\r\nc\r\nd\r\ne\r\nf\x1b[r\x1b[8;1Hbottom'
  }
]

describe.runIf(core?.Screen)('native screen parity with xterm', () => {
  for (const c of CASES) {
    it(c.name, async () => {
      const { js, native } = await both(c.input, c.cols, c.rows)
      expect(native.rows).toEqual(js.rows)
      expect(native.cursor).toEqual(js.cursor)
      expect(native.alternate).toBe(js.alternate)
      expect(native.styles, `${PALETTE_AS_256}, ${BLANKS_AS_SPACES}`).toEqual(js.styles)
    })
  }

  it('the title', async () => {
    const { jsTitle, nativeTitle } = await both('\x1b]0;first\x07\x1b]2;✳ claude\x07')
    expect(nativeTitle).toBe(jsTitle)
  })
})
