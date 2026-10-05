/**
 * Hotspot 3: git on the server's event loop.
 *
 * Git runs on the core, off the loop. `stall.*` is how long a call holds the
 * loop before the loop can take its next turn: only handing the first request
 * to the core. `wall.*` is how long until the answer arrives.
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
import { gitRunner } from '../../packages/server/src/git-runner'
import { makeRepo } from '../lib/git-fixture'
import { nowMs, repeatAsync, round, timeAsync } from '../lib/stats'
import { emit, metric, QUICK, type Metric } from '../lib/suite'

const REPS = QUICK ? 3 : 15
const repo = makeRepo()
const metrics: Record<string, Metric> = {}

const calls: Array<[string, () => Promise<unknown>]> = [
  ['isGitRepo', () => isGitRepo(repo.dir)],
  ['getGitBranch', () => getGitBranch(repo.dir)],
  ['getGitStatusPorcelain', () => getGitStatusPorcelain(repo.dir)],
  ['getGitDiffStat', () => getGitDiffStat(repo.dir)],
  ['getGitDiffText', () => getGitDiffText(repo.dir)],
  ['getGitDiffFull', () => getGitDiffFull(repo.dir)],
  ['listWorktrees', () => listWorktrees(repo.dir)]
]

const mode = gitRunner().mode
if (mode !== 'native') {
  // The runner falls back to a child process quietly; that would time the wrong thing.
  throw new Error('the vorn core did not load, or has no gitRun')
}

async function main(): Promise<void> {
  for (const [name, call] of calls) {
    const stall = await repeatAsync(
      REPS,
      async () => {
        const started = nowMs()
        const answer = call()
        // A macrotask runs only once the loop is free again.
        await new Promise((resolve) => setImmediate(resolve))
        const ms = nowMs() - started
        await answer
        return ms
      },
      // A native stall is a few microseconds; a minimum sample length would
      // repeat a 70 ms diff thousands of times to fill it.
      { minSampleMs: 0, estimator: 'min' }
    )
    const wall = await repeatAsync(REPS, () => timeAsync(async () => void (await call())), {
      minSampleMs: 100,
      estimator: 'min'
    })
    metrics[`stall.${name}`] = metric(
      round(stall),
      'ms',
      `event-loop stall per ${name} call (${mode} git)`
    )
    metrics[`wall.${name}`] = metric(
      round(wall),
      'ms',
      `time to answer one ${name} call (${mode} git)`
    )
  }

  emit({
    suite: 'git',
    metrics,
    info: {
      mode,
      diffBytes: repo.diffBytes,
      git: execFileSync('git', ['--version'], { encoding: 'utf-8' }).trim()
    }
  })
  repo.cleanup()
  process.exit(0)
}

void main()
