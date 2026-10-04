/**
 * The terminal output every suite is timed against.
 *
 * Deterministic: a seeded generator, so a run before and after a change feed the
 * same bytes in the same chunks and the comparison means something. Chunk sizes
 * matter as much as content -- `appendOutput` runs once per raw chunk, so the
 * same megabyte costs very different amounts as sixty-byte keystroke echoes or
 * as four-kilobyte reads.
 */
import { chunks as processTestChunks } from '../../tests/helpers/measure-output'

export interface Transcript {
  name: string
  description: string
  chunks: string[]
  bytes: number
}

/** mulberry32: small, fast, and the same everywhere. */
function rng(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

function byteLength(chunks: string[]): number {
  let n = 0
  for (const c of chunks) n += Buffer.byteLength(c, 'utf-8')
  return n
}

function make(name: string, description: string, chunks: string[]): Transcript {
  return { name, description, chunks, bytes: byteLength(chunks) }
}

const SPINNER = '⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏'
const FILES = [
  'packages/server/src/pty-manager.ts',
  'packages/server/src/terminal-screen.ts',
  'src/renderer/lib/terminal-registry.ts',
  'packages/shared/src/protocol.ts',
  'tests/pty-manager-screen.test.ts'
]

/**
 * What a coding agent's TUI writes while it works: a status line redrawn in
 * place many times a second, with tool calls and their results in between.
 *
 * This is the shape `stripAnsi`'s carriage-return pass and the status regexes
 * were not written for -- most bytes are repaints of one line.
 */
function spinnerFrames(targetBytes: number, seed: number): string[] {
  const rand = rng(seed)
  const frames: string[] = []
  let bytes = 0
  let tick = 0
  const push = (s: string): void => {
    frames.push(s)
    bytes += Buffer.byteLength(s, 'utf-8')
  }
  push('\x1b]0;✳ claude\x07\x1b[?2004h\x1b[?25l')
  while (bytes < targetBytes) {
    tick++
    const r = rand()
    if (r < 0.04) {
      const file = FILES[Math.floor(rand() * FILES.length)]
      push(
        `\r\x1b[2K\r\n\x1b[1m\x1b[38;5;114m⏺\x1b[39m Read\x1b[22m(${file})\r\n` +
          `  \x1b[2m⎿  Read ${50 + Math.floor(rand() * 900)} lines\x1b[22m\r\n\r\n`
      )
    } else if (r < 0.06) {
      const words = Math.floor(8 + rand() * 40)
      let text = ''
      for (let i = 0; i < words; i++)
        text += ['the', 'flush', 'buffer', 'screen', 'model', 'per', 'chunk'][i % 7] + ' '
      push(`\r\x1b[2K\x1b[38;5;231m⏺\x1b[39m ${text.trimEnd()}\r\n`)
    } else if (r < 0.061) {
      // A prompt opening and closing, the bracketed-paste signal `appendOutput` keys status on.
      push('\x1b[?2004l\x1b[?25h\r\n> \x1b[?2004h\x1b[?25l')
    } else {
      const s = Math.floor(tick / 10)
      const tokens = (tick * 13) % 9000
      push(
        `\r\x1b[2K\x1b[38;5;174m${SPINNER[tick % SPINNER.length]}\x1b[39m ` +
          `\x1b[38;5;174mThinking…\x1b[39m \x1b[2m(${s}s · ↑ ${(tokens / 1000).toFixed(1)}k tokens · esc to interrupt)\x1b[22m`
      )
    }
  }
  return frames
}

/**
 * Group frames into reads the way a PTY delivers them: usually one write per
 * read, sometimes a handful, now and then a large backlog when the reader fell
 * behind.
 */
function groupIntoReads(frames: string[], seed: number): string[] {
  const rand = rng(seed)
  const reads: string[] = []
  let i = 0
  while (i < frames.length) {
    const r = rand()
    const n = r < 0.7 ? 1 : r < 0.95 ? 2 + Math.floor(rand() * 9) : 20 + Math.floor(rand() * 41)
    reads.push(frames.slice(i, i + n).join(''))
    i += n
  }
  return reads
}

/** A build log scrolling by: plain lines, some colour, read in full 4 KB pages. */
function bulkLog(targetBytes: number, seed: number): string[] {
  const rand = rng(seed)
  let text = ''
  let line = 0
  while (text.length < targetBytes) {
    line++
    const file = FILES[line % FILES.length]
    const kind = rand()
    if (kind < 0.1) text += `\x1b[33mwarning\x1b[0m: unused variable in ${file}:${line}\r\n`
    else if (kind < 0.12) text += `\x1b[31merror\x1b[0m TS2345: ${file}(${line},7)\r\n`
    else text += `  compiling ${file} [${line}] ${'.'.repeat(Math.floor(rand() * 60))}\r\n`
  }
  const reads: string[] = []
  for (let i = 0; i < text.length; i += 4096) reads.push(text.slice(i, i + 4096))
  return reads
}

let cache: Transcript[] | null = null

export function transcripts(): Transcript[] {
  cache ??= [
    make(
      'agent',
      'the process tests’ generator: coloured, cursor-addressed lines, one per read (~62 B)',
      processTestChunks()
    ),
    make(
      'spinner',
      '1 MB of an agent TUI redrawing its status line, with tool calls between; reads of 1-60 frames',
      groupIntoReads(spinnerFrames(1024 * 1024, 7), 11)
    ),
    make('bulk', '2 MB of build log in 4 KB reads', bulkLog(2 * 1024 * 1024, 3))
  ]
  return cache
}

export function transcript(name: string): Transcript {
  const found = transcripts().find((t) => t.name === name)
  if (!found) throw new Error(`no transcript named ${name}`)
  return found
}

/** What one flush carries at most in the bench: this many reads, or this many bytes. */
export const FLUSH_READS = 100
export const FLUSH_BYTES = 64 * 1024

/**
 * The same bytes as the flushes `PtyManager` would make of them.
 *
 * The screen model, the scrollback and history are fed per flush, not per read.
 * How many reads a flush gathers depends on timing; this takes the assumption
 * `tests/helpers/measure-history.ts` already makes -- a hundred reads -- and caps
 * a flush at 64 KB, the server's flush cap.
 */
export function asFlushes(t: Transcript, perFlush = FLUSH_READS, maxBytes = FLUSH_BYTES): string[] {
  const out: string[] = []
  let cur = ''
  let curBytes = 0
  let n = 0
  for (const c of t.chunks) {
    cur += c
    curBytes += Buffer.byteLength(c)
    n++
    if (n >= perFlush || curBytes >= maxBytes) {
      out.push(cur)
      cur = ''
      curBytes = 0
      n = 0
    }
  }
  if (cur) out.push(cur)
  return out
}
