/**
 * Hotspot 7: the flush fan-out.
 *
 * One `flushBuffer` does all of: number the flush, frame it for every
 * desktop-style client (`Buffer.from` plus the binary header), append it to the
 * scrollback ring, queue it into the headless xterm, build a history frame
 * (UTF-8 encode and CRC-32) and scan it for a bell. This times that whole call
 * per megabyte -- the part that runs on the loop inside the flush -- with
 * history recording to a scratch directory and the client count varied. The
 * xterm parse the flush queues runs in later turns and is timed by
 * `screen-model`; the two add up to what a megabyte costs the server.
 *
 * The per-flush grouping here is the 64 KB cap a flush carries at most
 * (`MAX_FLUSH_UNITS`), so each call is one real flush.
 */
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import {
  configureHistory,
  resetHistory,
  settleHistory
} from '../../packages/server/src/history/writer'
import { resetScreens, serializeScreen } from '../../packages/server/src/terminal-screen'
import { resetScrollback } from '../../packages/server/src/terminal-scrollback'
import { addSession, connectClients, pm, removeSession } from '../lib/server-harness'
import { MB, repeatAsync, round, time } from '../lib/stats'
import { emit, metric, QUICK, type Metric } from '../lib/suite'
import '../lib/core'
import { asFlushes, transcripts } from '../lib/transcripts'

const REPS = QUICK ? 2 : 9
const CPU = { estimator: 'min', warmup: 5, minSampleMs: 200 } as const
const CLIENTS = [1, 8]

async function main(): Promise<void> {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-bench-flush-'))
  configureHistory(dir)
  const metrics: Record<string, Metric> = {}
  let run = 0

  for (const clients of CLIENTS) {
    for (const t of transcripts()) {
      const flushes = asFlushes(t)
      const mb = t.bytes / MB
      const elapsed = await repeatAsync(
        REPS,
        async () => {
          const id = `flush-${run++}`
          const wire = connectClients(clients)
          addSession(id)
          const ms = time(() => {
            for (const f of flushes) {
              pm.dataBuffers.set(id, { chunks: [f], units: f.length })
              pm.flushBuffer(id)
            }
          })
          // The parse this queued runs after; drained outside the clock so it
          // does not leak into the next sample. `screen-model` times it.
          await serializeScreen(id)
          wire.disconnect()
          removeSession(id)
          resetScreens()
          resetScrollback()
          return ms
        },
        CPU
      )
      metrics[`flush.${t.name}.${clients}client`] = metric(
        round(elapsed / mb),
        'ms/MB',
        `flushBuffer on the loop (frame, scrollback, screen queue, history frame, bell), ${t.name}, ${clients} client(s)`
      )
    }
  }

  resetHistory()
  await settleHistory().catch(() => undefined)
  fs.rmSync(dir, { recursive: true, force: true })
  emit({ suite: 'flush', metrics })
  process.exit(0)
}

void main()
