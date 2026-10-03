/**
 * What the per-session state costs to hold: the screen model plus the analysis
 * state, after each of 32 sessions at 200x50 has taken the 1 MB agent
 * transcript.
 *
 * Measured as growth over a baseline taken after start-up, with the GC forced on
 * both sides. RSS is the number that matters for the native core, whose memory
 * is off the V8 heap; the heap is reported beside it so the JS path's share is
 * visible.
 */
import {
  createScreen,
  feedScreen,
  resetScreens,
  serializeScreen
} from '../../packages/server/src/terminal-screen'
import { addAnalysisSession, pm, removeSession } from '../lib/server-harness'
import { MB, round } from '../lib/stats'
import { emit, metric, QUICK, type Metric } from '../lib/suite'
import '../lib/core'
import { asFlushes, transcript } from '../lib/transcripts'

const SESSIONS = QUICK ? 8 : 32

function settle(): { rss: number; heap: number } {
  for (let i = 0; i < 3; i++) global.gc?.()
  const m = process.memoryUsage()
  return { rss: m.rss, heap: m.heapUsed + m.external }
}

async function main(): Promise<void> {
  const t = transcript('spinner')
  const flushes = asFlushes(t)
  // Warm the code paths so their one-off allocations land in the baseline.
  addAnalysisSession('warm')
  createScreen('warm', 200, 50)
  for (const c of t.chunks.slice(0, 200)) pm.appendOutput('warm', c)
  feedScreen('warm', flushes[0])
  await serializeScreen('warm')
  removeSession('warm')
  resetScreens()

  const before = settle()
  const ids: string[] = []
  for (let i = 0; i < SESSIONS; i++) {
    const id = `mem-${i}`
    ids.push(id)
    addAnalysisSession(id)
    createScreen(id, 200, 50)
    for (const c of t.chunks) pm.appendOutput(id, c)
    for (const f of flushes) feedScreen(id, f)
  }
  for (const id of ids) await serializeScreen(id)
  const after = settle()

  const metrics: Record<string, Metric> = {
    'rss.perSession': metric(
      round((after.rss - before.rss) / MB / SESSIONS, 3),
      'MB',
      `RSS growth per session (screen model + analysis), ${SESSIONS} sessions at 200x50`
    ),
    'heap.perSession': metric(
      round((after.heap - before.heap) / MB / SESSIONS, 3),
      'MB',
      `V8 heap + external growth per session, ${SESSIONS} sessions at 200x50`
    )
  }
  for (const id of ids) removeSession(id)
  resetScreens()
  emit({ suite: 'memory', metrics, info: { sessions: SESSIONS } })
  process.exit(0)
}

void main()
