/**
 * The number the hotspots add up to: how late the server's event loop runs while
 * terminals are streaming.
 *
 * Fake ptys push output at a fixed rate through `PtyManager`'s real handlers --
 * `appendOutput` per read, the 8 ms coalescing timer, `flushBuffer` to a client,
 * the scrollback, the headless xterm and history on disk -- while
 * a 1 ms probe timer records how late the loop gets to it. A late
 * loop is a late keystroke echo and a late RPC answer for every session at once.
 *
 * The generator runs on the same loop as everything it measures, as node-pty's
 * reads do. When the loop cannot keep up it does not queue unbounded catch-up;
 * it carries at most 20 ms of owed output into the next turn and drops the rest,
 * so "delivered" below is what the server actually sustained.
 *
 * WP4 is accepted on `burst.50MBps.8s.p99` going under 2 ms, WP5 on the
 * `agents+git` row matching the `agents` row with `VORN_GIT=native`.
 */
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { createHistogram, type RecordableHistogram } from 'node:perf_hooks'
import { getGitDiffFull } from '../../packages/server/src/git-utils'
import { gitRunner } from '../../packages/server/src/git-runner'
import { configureHistory, resetHistory } from '../../packages/server/src/history/writer'
import { resetScreens } from '../../packages/server/src/terminal-screen'
import { resetScrollback } from '../../packages/server/src/terminal-scrollback'
import { makeRepo } from '../lib/git-fixture'
import { addSession, connectClients, removeSession, type FakePty } from '../lib/server-harness'
import { MB, nowMs, round } from '../lib/stats'
import { emit, metric, QUICK, type Metric } from '../lib/suite'
import '../lib/core'
import { transcript, type Transcript } from '../lib/transcripts'

const DURATION_MS = QUICK ? 1000 : 2000
/** Each scenario runs this many times in this process and reports the median, for stability. */
const ROUNDS = QUICK ? 1 : 3

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
  deliveredBytes: number
  elapsedMs: number
}

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms))

/**
 * How late a 1 ms timer fires, recorded until the returned stop is called.
 *
 * What `monitorEventLoopDelay` measures, done by hand because its histogram
 * cannot be merged into another, and rounds have to be pooled (see below).
 */
function probeLag(into: RecordableHistogram): () => void {
  let due = nowMs() + 1
  let timer: NodeJS.Timeout
  const tick = (): void => {
    const now = nowMs()
    into.record(Math.max(1, Math.round((now - due) * 1e6)))
    due = now + 1
    timer = setTimeout(tick, 1)
  }
  timer = setTimeout(tick, 1)
  return () => clearTimeout(timer)
}

async function runScenario(
  s: Scenario,
  repoDir: string,
  round_: number,
  lag: RecordableHistogram
): Promise<Outcome> {
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
  // The budget is in UTF-8 bytes, so each read is charged its byte length too.
  const sizes = s.source.chunks.map((c) => Buffer.byteLength(c))
  const maxOwed = (s.bytesPerSecond * 20) / 1000

  // Let the spawn-time work settle before the clock starts.
  await sleep(50)

  const started = nowMs()
  let sent = 0
  let owedCarry = 0
  let last = started
  let session = 0
  let gitTimer: NodeJS.Timeout | undefined
  let gitBusy = false
  if (s.gitEveryMs) {
    // One at a time, as the diff panel asks: a slow answer delays the next
    // request rather than stacking them up.
    gitTimer = setInterval(() => {
      if (gitBusy) return
      gitBusy = true
      getGitDiffFull(repoDir)
        .catch(() => null)
        .finally(() => (gitBusy = false))
    }, s.gitEveryMs)
  }

  const stopProbe = probeLag(lag)
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
        const size = sizes[cursors[i]]
        cursors[i] = (cursors[i] + 1) % s.source.chunks.length
        ptys[i].emit(chunk)
        owedCarry -= size
        sent += size
      }
      setTimeout(tick, 1)
    }
    tick()
  })
  stopProbe()
  if (gitTimer) clearInterval(gitTimer)
  const elapsed = nowMs() - started
  void sent

  // Let the last flushes land before counting what reached the client.
  await sleep(50)
  const delivered = wire.outputBytes()
  wire.disconnect()
  for (const id of ids) removeSession(id)
  resetScreens()
  resetScrollback()

  return { deliveredBytes: delivered, elapsedMs: elapsed }
}

async function main(): Promise<void> {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-bench-loop-'))
  configureHistory(dir)
  const repo = makeRepo()
  const bulk = transcript('bulk')
  const spinner = transcript('spinner')

  const scenarios: Scenario[] = [
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

  // The floor: what an idle loop reports on this machine. Context, not a metric;
  // a percentage of a millisecond and a half is noise whatever it does.
  const idle = createHistogram()
  const stopIdle = probeLag(idle)
  await sleep(DURATION_MS)
  stopIdle()
  const idleP99 = round(idle.percentile(99) / 1e6)

  for (const s of scenarios) {
    // Rounds are pooled into one histogram rather than reduced to a median of
    // per-round percentiles. A saturated loop takes few samples a round, and
    // a percentile of a few dozen samples jumps; of a few hundred it does not.
    const pooled = createHistogram()
    let bytes = 0
    let elapsed = 0
    for (let r = 0; r < ROUNDS; r++) {
      const o = await runScenario(s, repo.dir, r, pooled)
      bytes += o.deliveredBytes
      elapsed += o.elapsedMs
    }
    metrics[`${s.name}.p99`] = metric(
      round(pooled.percentile(99) / 1e6),
      'ms',
      `event-loop delay p99, ${s.label}`
    )
    // Max is kept out of the metrics: one stray turn decides it, so it cannot
    // meet a 10% reproducibility bar. It is in `info` for reading.
    maxima[s.name] = round(pooled.max / 1e6)
    metrics[`${s.name}.delivered`] = metric(
      round(bytes / MB / (elapsed / 1000)),
      'MB/s',
      `output that reached the client, ${s.label}`,
      'higher'
    )
  }

  resetHistory()
  repo.cleanup()
  fs.rmSync(dir, { recursive: true, force: true })
  emit({
    suite: 'event-loop',
    metrics,
    info: {
      git: gitRunner().mode,
      durationMs: DURATION_MS,
      rounds: ROUNDS,
      idleP99Ms: idleP99,
      maxDelayMs: maxima
    }
  })
  process.exit(0)
}

void main()
