// Runs the RPC test files through vornd: the same assertions, with every client
// connecting to vornd instead of the server. `yarn test` runs them direct.
//
// Uses VORN_CONFORMANCE_VORND when it names a binary; otherwise builds vornd.
import { spawnSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

/** Every test file that talks to a started server over a real WebSocket. */
const FILES = [
  'tests/server-integration.test.ts',
  'tests/task-write-methods.test.ts',
  'tests/workflow-methods.test.ts'
]

function run(command, args, env = process.env) {
  const result = spawnSync(command, args, {
    cwd: root,
    stdio: 'inherit',
    env,
    shell: process.platform === 'win32'
  })
  if (result.status !== 0) process.exit(result.status ?? 1)
}

let binary = process.env.VORN_CONFORMANCE_VORND
if (!binary) {
  run('cargo', [
    'build',
    '--release',
    '--locked',
    '-p',
    'vornd',
    '--manifest-path',
    'packages/core/Cargo.toml'
  ])
  binary = path.join(
    root,
    'packages/core/target/release',
    process.platform === 'win32' ? 'vornd.exe' : 'vornd'
  )
}
if (!existsSync(binary)) {
  console.error(`conformance: no vornd at ${binary}`)
  process.exit(1)
}

run('yarn', ['vitest', 'run', ...FILES], {
  ...process.env,
  VORN_CONFORMANCE_VORND: path.resolve(binary)
})
