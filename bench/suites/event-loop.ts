/**
 * The number the hotspots add up to: how late the server's event loop runs while
 * terminals are streaming.
 *
 * Fake ptys push output at a fixed rate through `PtyManager`'s real handlers --
 * `appendOutput` per read, the 8 ms coalescing timer, `flushBuffer` to a client,
 * the scrollback, the headless xterm and history on disk -- while
 * `monitorEventLoopDelay` records how late the loop gets to its timers. A late
 * loop is a late keystroke echo and a late RPC answer for every session at once.
 *
 * The generator runs on the same loop as everything it measures, as node-pty's
 * reads do. When the loop cannot keep up it does not queue unbounded catch-up;
 * it carries at most 20 ms of owed output into the next turn and drops the rest,
 * so "delivered" below is what the server actually sustained.
 *
 * WP4 is accepted on `burst.50MBps.8s.p99` going under 2 ms, WP5 on the
 * `agents+git` row matching the `agents` row.
 */
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { monitorEventLoopDelay } from 'node:perf_hooks'
import { getGitDiffFull } from '../../packages/server/src/git-utils'
import { configureHistory, resetHistory } from '../../packages/server/src/history/writer'
import { resetScreens } from '../../packages/server/src/terminal-screen'
import { resetScrollback } from '../../packages/server/src/terminal-scrollback'
import { makeRepo } from '../lib/git-fixture'
import { addSession, connectClients, removeSession, type FakePty } from '../lib/server-harness'
import { MB, median, nowMs, round } from '../lib/stats'
import { emit, metric, QUICK, type Metric } from '../lib/suite'
import { transcript, type Transcript } from '../lib/transcripts'

const DURATION_MS = QUICK ? 1000 : 2000
/** Each scenario runs this many times in this process and reports the median, for stability. */
const ROUNDS = QUICK ? 1 : 5

interface Scenario {
  name: string
  label: string
  sessions: number
  /** Total across all sessions. */
  bytesPerSecond: number
  source: Transcript
  /** A git call on the loop at this interval, as the diff panel and worktree refresh do. */
  gitEveryMs?: number
}

interface Outcome {
  p50: number
  p99: number
  max: number
  deliveredMBps: number
}

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms))

async function runScenario(s: Scenario, repoDir: string, round_: number): Promise<Outcome> {
  const ids: string[] = []
  const ptys: FakePty[] = []
  const wire = connectClients(1)
  for (let i = 0; i < s.sessions; i++) {
    const id = `${s.name}-${round_}-${i}`
    ids.push(id)
    ptys.push(addSession(id))
  }
  const cursors = new Array<number>(s.sessions).fill(0)
  // Staggered so sessions are not all on the same read.
  for (let i = 0; i < s.sessions; i++)
    cursors[i] = Math.floor((i * s.source.chunks.length) / s.sessions)
  const readBytes = s.source.bytes / s.source.chunks.length
  const maxOwed = (s.bytesPerSecond * 20) / 1000

  // Let the spawn-time work settle before the clock starts.
  await sleep(50)

  const histogram = monitorEventLoopDelay({ resolution: 1 })
  const started = nowMs()
  let sent = 0
  let owedCarry = 0
  let last = started
  let session = 0
  let gitTimer: NodeJS.Timeout | undefined
  if (s.gitEveryMs) gitTimer = setInterval(() => void getGitDiffFull(repoDir), s.gitEveryMs)

  histogram.enable()
  await new Promise<void>((resolve) => {
    const tick = (): void => {
      const now = nowMs()
      if (now - started >= DURATION_MS) {
        resolve()
        return
      }
      owedCarry = Math.min(maxOwed, owedCarry + ((now - last) * s.bytesPerSecond) / 1000)
      last = now
      while (owedCarry >= readBytes) {
        const i = session
        session = (session + 1) % s.sessions
        const chunk = s.source.chunks[cursors[i]]
        cursors[i] = (cursors[i] + 1) % s.source.chunks.length
        ptys[i].emit(chunk)
        owedCarry -= chunk.length
        sent += chunk.length
      }
      setTimeout(tick, 1)
    }
    tick()
  })
  histogram.disable()
  if (gitTimer) clearInterval(gitTimer)
  const elapsed = nowMs() - started
  void sent

  // Let the last flushes land before counting what reached the client.
  await sleep(50)
  const delivered = wire.sockets[0].bytes
  wire.disconnect()
  for (const id of ids) removeSession(id)
  resetScreens()
  resetScrollback()

  return {
    p50: histogram.percentile(50) / 1e6,
    p99: histogram.percentile(99) / 1e6,
    max: histogram.max / 1e6,
    deliveredMBps: delivered / MB / (elapsed / 1000)
  }
}

async function main(): Promise<void> {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-bench-loop-'))
  configureHistory(dir)
  const repo = makeRepo()
  const bulk = transcript('bulk')
  const spinner = transcript('spinner')

  const scenarios: Scenario[] = [
    { name: 'idle', label: 'no output (floor)', sessions: 1, bytesPerSecond: 0, source: bulk },
    ...[1, 8, 32].map((n) => ({
      name: `burst.50MBps.${n}s`,
      label: `50 MB/s of build log across ${n} session(s)`,
      sessions: n,
      bytesPerSecond: 50 * MB,
      source: bulk
    })),
    {
      name: 'agents',
      label: '8 agent TUIs at 1 MB/s each',
      sessions: 8,
      bytesPerSecond: 8 * MB,
      source: spinner
    },
    {
      name: 'agents+git',
      label: '8 agent TUIs at 1 MB/s each, getGitDiffFull (460 KB diff) every 250 ms',
      sessions: 8,
      bytesPerSecond: 8 * MB,
      source: spinner,
      gitEveryMs: 250
    }
  ]

  const metrics: Record<string, Metric> = {}
  const maxima: Record<string, number> = {}
  for (const s of scenarios) {
    const outcomes: Outcome[] = []
    for (let r = 0; r < ROUNDS; r++) {
      if (s.bytesPerSecond === 0) {
        const h = monitorEventLoopDelay({ resolution: 1 })
        h.enable()
        await sleep(DURATION_MS)
        h.disable()
        outcomes.push({
          p50: h.percentile(50) / 1e6,
          p99: h.percentile(99) / 1e6,
          max: h.max / 1e6,
          deliveredMBps: 0
        })
      } else {
        outcomes.push(await runScenario(s, repo.dir, r))
      }
    }
    const pick = (k: keyof Outcome): number => median(outcomes.map((o) => o[k]))
    metrics[`${s.name}.p99`] = metric(round(pick('p99')), 'ms', `event-loop delay p99, ${s.label}`)
    // Max is kept out of the metrics: one stray turn decides it, so it cannot
    // meet a 10% reproducibility bar. It is in `info` for reading.
    maxima[s.name] = round(pick('max'))
    if (s.bytesPerSecond > 0) {
      metrics[`${s.name}.delivered`] = metric(
        round(pick('deliveredMBps')),
        'MB/s',
        `output that reached the client, ${s.label}`,
        'higher'
      )
    }
  }

  resetHistory()
  repo.cleanup()
  fs.rmSync(dir, { recursive: true, force: true })
  emit({
    suite: 'event-loop',
    metrics,
    info: { durationMs: DURATION_MS, rounds: ROUNDS, maxDelayMs: maxima }
  })
  process.exit(0)
}

void main()
