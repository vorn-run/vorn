import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { TestProject } from 'vitest/node'
import { appLogFiles, linesSince, snapshotLogs } from '../helpers/app-log'
import { inSandbox, leftOver } from '../helpers/daemons'

export default function setup(project: TestProject): () => Promise<void> {
  const realHome = os.homedir()
  const repoRoot = project.config.root
  const sandboxRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-tests-'))
  // Every daemon a test starts inherits it, which is how the teardown tells this run's from another's.
  process.env.VORN_TEST_SANDBOX = sandboxRoot
  project.provide('realHome', realHome)
  project.provide('sandboxRoot', sandboxRoot)
  project.provide('repoRoot', repoRoot)
  // Other checkouts and the running app write here too, so only lines naming this checkout count.
  const logs = snapshotLogs(appLogFiles(realHome))

  return async () => {
    // Daemons told to end with their worker take a moment to notice it ended.
    const left = await leftOver(() => inSandbox(sandboxRoot), 10)
    fs.rmSync(sandboxRoot, { recursive: true, force: true })
    if (left) {
      console.error(left)
      process.exitCode = 1
    }
    const leaked = linesSince(logs, repoRoot)
    if (leaked.length === 0) return
    console.error(`The test run wrote to the real app log:\n${leaked.slice(0, 20).join('\n')}`)
    // Vitest only reports an error thrown from teardown; this is what fails the run.
    process.exitCode = 1
  }
}
