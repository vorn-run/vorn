import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { appendScrollback, resetScrollback } from '../../packages/server/src/terminal-scrollback'
import {
  createScreen,
  feedScreen,
  serializeScreen,
  resetScreens
} from '../../packages/server/src/terminal-screen'
import {
  configureHistory,
  startHistory,
  recordOutput,
  flushHistory,
  settleHistory,
  resetHistory
} from '../../packages/server/src/history/writer'
import { recoverHistory } from '../../packages/server/src/history/recovery'
import { historyDir } from '../../packages/server/src/history/checkpoint'
import { frameOutput } from '../../packages/server/src/history/log'
import { chunks, ms, msAsync, CHUNKS, COLS, ROWS } from './measure-output'

/**
 * What writing history costs, in the three places it is paid.
 *
 * The interval this runs at is a trade -- how much log a crash leaves to replay,
 * against how much I/O an idle machine does -- and the only honest way to pick a
 * number is to measure both sides. So: what an append adds to the hottest path
 * in the server, what a checkpoint costs against session count, and what a
 * restart spends reading it all back.
 *
 * Prints one line of JSON; the test reads it and decides.
 */

const SESSIONS = 50

/**
 * How many chunks one flush gathers.
 *
 * `PtyManager.BUFFER_FLUSH_MS` is 8, and node-pty delivers small chunks quickly
 * under load, so a busy terminal's flush covers a great many of them. A hundred
 * is a conservative reading of the twenty thousand chunks below arriving at a
 * rate a terminal can actually produce.
 */
const PER_FLUSH = 100

function scratch(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-measure-history-'))
}

/** Everything this module can be holding, in a fixed order. */
function clean(): void {
  resetHistory()
  resetScreens()
  resetScrollback()
}

/** Fill `count` sessions with a screenful each, and their logs with the rest. */
function fill(count: number, data: string[]): string[] {
  const ids: string[] = []
  for (let i = 0; i < count; i++) {
    const id = `session-${i}`
    ids.push(id)
    createScreen(id, COLS, ROWS)
    startHistory(id)
    for (const chunk of data) {
      appendScrollback(id, chunk)
      feedScreen(id, chunk)
      recordOutput(id, chunk)
    }
  }
  return ids
}

async function main(): Promise<void> {
  const data = chunks()

  // Warm the JIT so the first shape measured is not paying for compilation the
  // others avoid.
  {
    const warm = scratch()
    configureHistory(warm)
    createScreen('warm', COLS, ROWS)
    startHistory('warm')
    for (const c of data.slice(0, 500)) {
      appendScrollback('warm', c)
      feedScreen('warm', c)
      recordOutput('warm', c)
    }
    await flushHistory()
    await recoverHistory(warm, [{ id: 'warm' }])
    clean()
    fs.rmSync(warm, { recursive: true, force: true })
  }

  // ---- What an append adds to the path every byte already takes. -------------
  //
  // Per chunk, which is the pessimistic reading: production calls `recordOutput`
  // once per flush, and a flush coalesces every byte that arrived in eight
  // milliseconds. So the real figure is smaller than this by however many chunks
  // a flush covers.
  //
  // Measured directly rather than as a difference. An earlier version of this
  // subtracted two whole-path timings and called the remainder the cost of an
  // append; both samples are around forty milliseconds and the machine's noise
  // is larger than the gap, so that number came out anywhere from three per cent
  // to negative. What it is is the frame construction, and it can be timed on
  // its own.
  //
  // Twice, because the two answer different questions. Per chunk is what the
  // path would pay if history were recorded where the output arrives. Coalesced
  // is what the server actually pays: `recordOutput` is called from the flush,
  // which gathers eight milliseconds of output into one write, so the real unit
  // is a batch of chunks rather than a chunk -- one encode and one checksum pass
  // over the same bytes instead of a hundred of each.
  const frameOnly = ms(() => {
    for (const c of data) frameOutput(c)
  })

  const flushes: string[] = []
  for (let at = 0; at < data.length; at += PER_FLUSH) {
    flushes.push(data.slice(at, at + PER_FLUSH).join(''))
  }
  const frameCoalesced = ms(() => {
    for (const f of flushes) frameOutput(f)
  })

  let dir = scratch()
  clean()
  configureHistory(dir)
  createScreen('s', COLS, ROWS)
  let withoutHistory = ms(() => {
    for (const c of data) {
      appendScrollback('s', c)
      feedScreen('s', c)
    }
  })
  // The parse is queued, so the loop finishing is not the work finishing.
  // Draining inside the clock is the difference between measuring the work and
  // measuring the scheduling of it.
  withoutHistory += await msAsync(async () => {
    await serializeScreen('s')
  })

  resetScreens()
  resetScrollback()
  resetHistory()
  configureHistory(dir)
  createScreen('t', COLS, ROWS)
  startHistory('t')
  let withHistory = ms(() => {
    for (const c of data) {
      appendScrollback('t', c)
      feedScreen('t', c)
      recordOutput('t', c)
    }
  })
  withHistory += await msAsync(async () => {
    await serializeScreen('t')
  })
  // Separately, because it is not on the hot path: the frames are handed to a
  // timer and written out behind it. What the terminal waits for is above.
  const appendToDiskMs = await msAsync(settleHistory)
  clean()
  fs.rmSync(dir, { recursive: true, force: true })

  // ---- What a checkpoint costs, against session count. -----------------------
  // Enough to fill the scrollback cap, so a checkpoint is the size a real
  // session's is rather than the size an empty one's is. That buffer is the
  // dominant part of what lands on disk.
  const screenful = data.slice(0, 5_000)
  const checkpointMs: Record<string, number> = {}
  let recoverMs = 0
  let bytesPerSession = 0

  for (const count of [1, 10, SESSIONS]) {
    dir = scratch()
    clean()
    configureHistory(dir)
    const ids = fill(count, screenful)
    // Everything at once, which is what `shutdown()` does and the worst this
    // ever has to do in one go.
    checkpointMs[String(count)] = +(await msAsync(() => flushHistory())).toFixed(1)

    if (count === SESSIONS) {
      let bytes = 0
      for (const id of ids) {
        const at = historyDir(dir, id)
        for (const file of fs.readdirSync(at)) bytes += fs.statSync(path.join(at, file)).size
      }
      bytesPerSession = Math.round(bytes / ids.length)
      resetScreens()
      resetScrollback()
      recoverMs = +(
        await msAsync(async () => {
          await recoverHistory(
            dir,
            ids.map((id) => ({ id, cols: COLS, rows: ROWS }))
          )
        })
      ).toFixed(1)
    }
    clean()
    fs.rmSync(dir, { recursive: true, force: true })
  }

  process.stdout.write(
    JSON.stringify({
      chunks: CHUNKS,
      sessions: SESSIONS,
      withoutHistoryMs: +withoutHistory.toFixed(1),
      withHistoryMs: +withHistory.toFixed(1),
      frameOnlyMs: +frameOnly.toFixed(1),
      frameCoalescedMs: +frameCoalesced.toFixed(1),
      perFlush: PER_FLUSH,
      appendToDiskMs: +appendToDiskMs.toFixed(1),
      checkpointMs,
      recoverMs,
      bytesPerSession
    }) + '\n'
  )
}

void main()
