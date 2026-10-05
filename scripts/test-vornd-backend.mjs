// Runs the suites that exercise the server's process backend again with the
// Native daemon switch on: terminals and piped agents start through the vornd
// link, answered in-process (tests/helpers/loopback-vornd.ts). `yarn test`
// runs the same files with the switch off.
import { spawnSync } from 'node:child_process'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

const FILES = [
  'tests/pty-manager-screen.test.ts',
  'tests/pty-manager-recovery.test.ts',
  'tests/headless-manager.test.ts',
  'tests/extension-pane-pty.test.ts',
  'tests/launch-agent-from-shell.test.ts',
  'tests/agent-detector.test.ts',
  'tests/agent-launch.test.ts',
  'tests/workflow-host.test.ts',
  'tests/workflow-run-lifecycle.test.ts',
  'tests/workflow-resume.test.ts'
]

const result = spawnSync('yarn', ['vitest', 'run', ...FILES, ...process.argv.slice(2)], {
  cwd: root,
  stdio: 'inherit',
  env: { ...process.env, VORN_TEST_BACKEND: 'vornd' },
  shell: process.platform === 'win32'
})
process.exit(result.status ?? 1)
