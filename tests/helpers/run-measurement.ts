import { spawnSync } from 'node:child_process'
import path from 'node:path'

/**
 * Run a measurement script in a process of its own and read its answer.
 *
 * A child rather than this worker, because a vitest worker holds the module
 * graph, React and whatever the rest of the suite left behind -- noise larger
 * than most of what is worth measuring. The script prints one line of JSON on
 * stdout; everything before it is noise from tsx and is ignored.
 */
export function runMeasurement<T>(
  script: string,
  options: { env?: NodeJS.ProcessEnv; timeoutMs?: number } = {}
): T {
  const full = path.join(__dirname, script)
  const run = spawnSync('npx', ['tsx', full], {
    cwd: path.join(__dirname, '..', '..'),
    encoding: 'utf-8',
    env: { ...process.env, ...options.env },
    timeout: options.timeoutMs ?? 300_000
  })
  if (run.status !== 0) {
    throw new Error(`measurement ${script} failed (${run.status}):\n${run.stderr}`)
  }
  const line = run.stdout.trim().split('\n').pop() ?? ''
  return JSON.parse(line) as T
}
