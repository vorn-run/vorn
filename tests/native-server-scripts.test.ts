/**
 * Project scripts on a real server and its vornd: vornd runs each one in its
 * directory with its arguments, and tells clients what it printed and how it
 * ended, read as `tests/helpers/scripts-parity.ts` names.
 *
 * Runs where vornd and vorn-sessiond have been built (`yarn build:core`, or
 * the binaries in `VORN_CONFORMANCE_VORND`), on a Unix.
 */
import fs from 'node:fs'
import path from 'node:path'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { spawnsRealServers } from './helpers/one-at-a-time'
import {
  Watcher,
  removeRealServerDirs,
  runnable,
  startRealServer,
  stopRealServer,
  until,
  type RealServer
} from './helpers/real-server'
import {
  normalizeScriptRun,
  type ScriptExecutionResult,
  type ScriptRun
} from './helpers/scripts-parity'
import { normalizeRun } from './helpers/sessions-parity'

spawnsRealServers()

const SCRIPTS = {
  succeeds: 'echo "in $(basename "$PWD")"; echo two',
  fails: 'echo out; echo err >&2; exit 3',
  args: 'printf "%s|" "$@"; echo'
} as const

let unsupported: unknown

async function scenario(server: RealServer): Promise<Record<string, ScriptRun>> {
  const project = path.join(server.dirs.work, 'proj')
  fs.mkdirSync(project)
  const through = await Watcher.open(server.vornd)
  try {
    // The server's first answer is what shows vornd the socket was admitted.
    await through.result('config:load')
    const runs: Record<string, ScriptRun> = {}
    for (const [name, scriptContent] of Object.entries(SCRIPTS)) {
      const runId = `run-${name}`
      const result = await through.result<ScriptExecutionResult>('script:execute', {
        scriptType: 'bash',
        scriptContent,
        cwd: project,
        args: name === 'args' ? ['a b', 'c'] : undefined,
        runId
      })
      const mine = (p: unknown): boolean => (p as { runId?: string }).runId === runId
      await until(`the exit of ${name}`, () => through.toldBy('script:exit').some(mine))
      runs[name] = {
        result,
        told: through
          .toldBy('script:data')
          .filter(mine)
          .map((p) => (p as { data: string }).data),
        exits: through
          .toldBy('script:exit')
          .filter(mine)
          .map((p) => (p as { exitCode: number }).exitCode)
      }
    }
    unsupported = await through.result('script:execute', {
      scriptType: 'ruby',
      scriptContent: 'puts 1',
      cwd: project
    })
    return runs
  } finally {
    through.close()
  }
}

describe.skipIf(!runnable)('project scripts through vornd', () => {
  let run: unknown
  let answered: unknown

  beforeAll(async () => {
    const server = await startRealServer()
    try {
      const runs = await scenario(server)
      run = normalizeRun(
        Object.fromEntries(
          Object.entries(runs).map(([k, r]) => [k, normalizeScriptRun(r, 'vornd')])
        ),
        server.dirs
      )
      const health = await fetch(`http://127.0.0.1:${server.vornd}/vornd/health`)
      const groups = (await health.json()) as {
        groups: Record<string, { native?: number; forwarded?: number }>
      }
      answered = groups.groups.script
    } catch (err) {
      throw new Error(`${(err as Error).message}\n${server.log.join('').slice(-4000)}`, {
        cause: err
      })
    } finally {
      await stopRealServer(server)
    }
  }, 240_000)

  afterAll(() => removeRealServerDirs())

  it('runs each script in its directory with its arguments', () => {
    expect(run).toEqual({
      succeeds: {
        success: true,
        exitCode: 0,
        failed: false,
        printed: ['in proj', 'two'],
        told: ['in proj', 'two'],
        exits: [0]
      },
      fails: {
        success: false,
        exitCode: 3,
        failed: true,
        printed: ['err', 'out'],
        told: ['err', 'out'],
        exits: [3]
      },
      args: {
        success: true,
        exitCode: 0,
        failed: false,
        printed: ['a b|c|'],
        told: ['a b|c|'],
        exits: [0]
      }
    })
  })

  it('says which type it does not run', () => {
    expect(unsupported).toEqual({
      success: false,
      output: '',
      error: 'Unsupported script type: ruby'
    })
  })

  it('has vornd run them', () => {
    expect(answered).toMatchObject({ native: 4 })
    expect((answered as { forwarded?: number }).forwarded ?? 0).toBe(0)
  })
})
