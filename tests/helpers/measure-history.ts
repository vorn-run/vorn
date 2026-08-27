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

const CHUNKS = 20_000
const COLS = 200
const ROWS = 50
const SESSIONS = 50

/** Output shaped like a working agent's: colour, cursor movement, varied text. */
function chunks(): string[] {
  const out: string[] = []
  for (let i = 0; i < CHUNKS; i++) {
    const fg = 30 + (i % 8)
    out.push(
      `\x1b[${(i % ROWS) + 1};1H\x1b[${fg}m` +
        `⏺ packages/server/src/file-${i % 40}.ts:${i} ` +
        `${'▁▂▃▄▅▆▇█'[i % 8]} done\x1b[0m\r\n`
    )
  }
  return out
}

function ms(run: () => void): number {
  const started = process.hrtime.bigint()
  run()
  return Number(process.hrtime.bigint() - started) / 1e6
}

async function msAsync(run: () => Promise<void>): Promise<number> {
  const started = process.hrtime.bigint()
  await run()
  return Number(process.hrtime.bigint() - started) / 1e6
}

function scratch(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-measure-history-'))
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
    await recoverHistory(warm, [{ id: 'warm', cols: COLS, rows: ROWS }])
    resetHistory()
    resetScreens()
    resetScrollback()
    fs.rmSync(warm, { recursive: true, force: true })
  }

  // ---- What an append adds to the path every byte already takes. -------------
  //
  // Per chunk, which is the pessimistic reading: production calls `recordOutput`
  // once per flush, and a flush coalesces every byte that arrived in eight
  // milliseconds. So the real figure is smaller than this by however many chunks
  // a flush covers.
  let dir = scratch()
  configureHistory(dir)
  resetScreens()
  resetScrollback()
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
  resetHistory()
  resetScreens()
  resetScrollback()
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
    resetHistory()
    resetScreens()
    resetScrollback()
    configureHistory(dir)
    const ids = fill(count, screenful)
    // Everything at once, which is what `shutdown()` does and the worst this
    // ever has to do in one go.
    checkpointMs[String(count)] = +(await msAsync(() => flushHistory())).toFixed(1)

    if (count === SESSIONS) {
      bytesPerSession = Math.round(
        ids.reduce(
          (total, id) =>
            total +
            fs
              .readdirSync(historyDir(dir, id))
              .reduce((n, f) => n + fs.statSync(path.join(historyDir(dir, id), f)).size, 0),
          0
        ) / ids.length
      )
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
    resetHistory()
    resetScreens()
    resetScrollback()
    fs.rmSync(dir, { recursive: true, force: true })
  }

  process.stdout.write(
    JSON.stringify({
      chunks: CHUNKS,
      sessions: SESSIONS,
      withoutHistoryMs: +withoutHistory.toFixed(1),
      withHistoryMs: +withHistory.toFixed(1),
      appendOnlyMs: +(withHistory - withoutHistory).toFixed(1),
      appendToDiskMs: +appendToDiskMs.toFixed(1),
      checkpointMs,
      recoverMs,
      bytesPerSession
    }) + '\n'
  )
}

void main()
