import { describe, it, expect, afterEach } from 'vitest'
import {
  selectCore,
  resetCoreSelection,
  setExperimentalSource,
  type NativeCore,
  type PipelineEvent
} from '../packages/server/src/native-core'
import {
  readFrames,
  readHeader,
  writeHeader,
  type LogRecord
} from '../packages/server/src/history/log'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { readCheckpoint, writeCheckpoint } from '../packages/server/src/history/checkpoint'
import {
  clearScreen,
  createScreen,
  feedScreen,
  hasScreen,
  resetScreens,
  serializeScreen,
  setBellReporter
} from '../packages/server/src/terminal-screen'
import { pipelineFor } from '../packages/server/src/core-pipeline'
import {
  appendScrollback,
  readScrollback,
  resetScrollback,
  seedScrollback
} from '../packages/server/src/terminal-scrollback'

/**
 * Settings › Experimental › terminal output on a core thread.
 *
 * The thread's own behaviour -- ordering, the ring, the frame layout -- is
 * tested in Rust (`crates/pipeline`). What only a test from this side can say
 * is that the two languages agree: frames the thread builds are frames
 * `log.ts` reads, and a checkpoint body it writes is one `checkpoint.ts`
 * accepts. Both are formats on disk, where a disagreement is found months
 * later as a session that will not restore.
 *
 * Skipped without a binary built with libghostty-vt; CI runs it after
 * building one.
 */

const core: NativeCore | null = selectCore({ env: { VORN_CORE: 'native' } }).native

const ID = 'piped'
const EPOCH = 7

afterEach(() => {
  setBellReporter(null)
  setExperimentalSource(null)
  resetCoreSelection()
  resetScreens()
  resetScrollback()
})

/** Until the thread has delivered what it was going to. */
async function settled(): Promise<void> {
  await new Promise((r) => setTimeout(r, 20))
}

describe.runIf(core?.TerminalPipeline !== undefined)('a terminal on a core thread', () => {
  function pipeline(onEvent: (e: PipelineEvent) => void = () => {}) {
    return new core!.TerminalPipeline!(80, 24, onEvent)
  }

  it('frames history the TypeScript reader reads back record for record', () => {
    const p = pipeline()
    const text = ['\x1b[31mred\x1b[0m\r\n', '▁▂▃ 日本語 🙂', 'last']
    let offset = 0
    text.forEach((t, rseq) => {
      p.feed(t, { rseq, startOffset: offset })
      offset += Buffer.byteLength(t)
    })
    p.resize(100, 30, { rseq: 3, startOffset: offset })
    p.scrollback() // a round trip, so everything above has been framed

    const log = Buffer.concat([
      writeHeader(1, { epoch: EPOCH, nextRseq: 0, nextOffset: 0 }),
      p.takeFrames()
    ])
    const { records, reason } = readFrames(log, readHeader(log)!)
    p.free()

    expect(reason).toBe('end')
    const second = Buffer.byteLength(text[0]!)
    expect(records).toEqual<LogRecord[]>([
      { kind: 'data', rseq: 0, startOffset: 0, stream: 0, data: text[0]! },
      { kind: 'data', rseq: 1, startOffset: second, stream: 0, data: text[1]! },
      { kind: 'data', rseq: 2, startOffset: offset - 4, stream: 0, data: text[2]! },
      { kind: 'resize', rseq: 3, startOffset: offset, cols: 100, rows: 30, pxWidth: 0, pxHeight: 0 }
    ])
  })

  it('writes a checkpoint body checkpoint.ts accepts, cut where it was asked for', async () => {
    const p = pipeline()
    p.restoreLabels('a title', '/srv')
    p.feed('before the cut\r\n', { rseq: 0, startOffset: 0 })
    const resume = { epoch: EPOCH, nextRseq: 1, nextOffset: 16 }
    const cutting = p.cut({ generation: 2, resume })
    // Sent after the cut was asked for, so in neither its screen nor its frames.
    p.feed('after the cut', { rseq: 1, startOffset: 16 })
    const cut = await cutting

    // Through the same write and read a restore uses.
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-pipeline-'))
    expect(await writeCheckpoint(dir, cut.body!)).toBe(true)
    const checkpoint = readCheckpoint(dir)
    fs.rmSync(dir, { recursive: true, force: true })
    expect(checkpoint).toMatchObject({
      scrollback: 'before the cut\r\n',
      cols: 80,
      rows: 24,
      title: 'a title',
      cwd: '/srv',
      generation: 2,
      resume
    })
    expect(checkpoint?.screen).toContain('before the cut')
    expect(checkpoint?.screen).not.toContain('after')
    // The frame for the output before the cut, and not the one after it.
    const header = writeHeader(2, resume)
    const before = readFrames(Buffer.concat([header, cut.frames]), readHeader(header)!)
    expect(before.records.map((r) => r.rseq)).toEqual([0])
    p.free()
  })

  it('rings for a bell and not for the BEL that ends a title', async () => {
    const events: PipelineEvent[] = []
    const p = pipeline((e) => events.push(e))
    p.feed('\x1b]0;title\x07')
    p.feed('done\x07')
    p.scrollback()
    await settled()
    p.free()

    expect(events.map((e) => e.kind)).toEqual(['bell'])
  })

  it('rejects a cut the thread will never answer, rather than leaving it pending', async () => {
    const p = pipeline()
    p.free()
    expect(() =>
      p.cut({ generation: 1, resume: { epoch: 1, nextRseq: 0, nextOffset: 0 } })
    ).toThrow()
  })
})

describe.runIf(core?.TerminalPipeline !== undefined)('the server, with the switch on', () => {
  function on(): void {
    resetCoreSelection()
    setExperimentalSource(() => ({ nativePipeline: true }))
  }

  it('gives a new terminal a pipeline, and carries on its scrollback', () => {
    on()
    appendScrollback(ID, 'printed before it had one\r\n')
    createScreen(ID, 80, 24)

    expect(pipelineFor(ID)).toBeDefined()
    appendScrollback(ID, 'and after')
    expect(readScrollback(ID)).toBe('printed before it had one\r\nand after')
  })

  it('keeps the scrollback when the pipeline goes, for a session resumed under the id', () => {
    on()
    createScreen(ID, 80, 24)
    seedScrollback(ID, 'what it showed')
    clearScreen(ID)

    expect(hasScreen(ID)).toBe(false)
    expect(readScrollback(ID)).toBe('what it showed')
  })

  it('reports a bell through the reporter, after the flush', async () => {
    on()
    const rang: string[] = []
    setBellReporter((id) => rang.push(id))
    createScreen(ID, 80, 24)

    expect(feedScreen(ID, 'ding\x07')).toBe(false)
    await serializeScreen(ID)
    await settled()

    expect(rang).toEqual([ID])
  })

  it('parses a replay into the screen without keeping it as scrollback', async () => {
    on()
    createScreen(ID, 80, 24)
    feedScreen(ID, 'replayed screen')

    expect((await serializeScreen(ID))?.screen).toContain('replayed screen')
    expect(readScrollback(ID)).toBe('')
  })
})
