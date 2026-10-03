/**
 * Hotspot 3: synchronous git on the server's event loop.
 *
 * `gitExec` is `execFileSync`, so every call below is time in which no PTY
 * output is flushed, no RPC is answered and no client hears anything -- for
 * every session at once. Each number is a median wall-clock per call, which for
 * a synchronous call is exactly the stall it causes. WP5 moves these off the
 * loop; `event-loop.ts` measures what that stall does to a live burst.
 */
import { execFileSync } from 'node:child_process'
import {
  getGitBranch,
  getGitDiffFull,
  getGitDiffStat,
  getGitDiffText,
  getGitStatusPorcelain,
  isGitRepo,
  listWorktrees
} from '../../packages/server/src/git-utils'
import { makeRepo } from '../lib/git-fixture'
import { repeat, round, time } from '../lib/stats'
import { emit, metric, QUICK, type Metric } from '../lib/suite'

const REPS = QUICK ? 3 : 15
const repo = makeRepo()
const metrics: Record<string, Metric> = {}

const calls: Array<[string, () => unknown]> = [
  ['isGitRepo', () => isGitRepo(repo.dir)],
  ['getGitBranch', () => getGitBranch(repo.dir)],
  ['getGitStatusPorcelain', () => getGitStatusPorcelain(repo.dir)],
  ['getGitDiffStat', () => getGitDiffStat(repo.dir)],
  ['getGitDiffText', () => getGitDiffText(repo.dir)],
  ['getGitDiffFull', () => getGitDiffFull(repo.dir)],
  ['listWorktrees', () => listWorktrees(repo.dir)]
]

for (const [name, call] of calls) {
  const ms = repeat(REPS, () => time(() => void call()), { minSampleMs: 100 })
  metrics[`stall.${name}`] = metric(
    round(ms),
    'ms',
    `event-loop stall per ${name} call (execFileSync)`
  )
}

emit({
  suite: 'git',
  metrics,
  info: {
    diffBytes: repo.diffBytes,
    git: execFileSync('git', ['--version'], { encoding: 'utf-8' }).trim()
  }
})
repo.cleanup()
process.exit(0)
