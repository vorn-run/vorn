import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { TestProject } from 'vitest/node'
import { appLogFiles, linesSince, snapshotLogs } from '../helpers/app-log'

export default function setup(project: TestProject): () => void {
  const realHome = os.homedir()
  const repoRoot = project.config.root
  const sandboxRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-tests-'))
  project.provide('realHome', realHome)
  project.provide('sandboxRoot', sandboxRoot)
  project.provide('repoRoot', repoRoot)
  // Other checkouts and the running app write here too, so only lines naming this checkout count.
  const logs = snapshotLogs(appLogFiles(realHome))

  return () => {
    fs.rmSync(sandboxRoot, { recursive: true, force: true })
    const leaked = linesSince(logs, repoRoot)
    if (leaked.length === 0) return
    console.error(`The test run wrote to the real app log:\n${leaked.slice(0, 20).join('\n')}`)
    // Vitest only reports an error thrown from teardown; this is what fails the run.
    process.exitCode = 1
  }
}
