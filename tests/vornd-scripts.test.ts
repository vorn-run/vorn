import { describe, it, expect, vi, beforeAll, afterAll } from 'vitest'
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

import { initDatabase, closeDatabase } from '../packages/server/src/database'
import { IPC, type ScriptConfig } from '../packages/shared/src/types'
import {
  executeScript,
  interpreterFor,
  scriptRunnerEvents
} from '../packages/server/src/script-runner'
import { setLaunchDataDir } from '../packages/server/src/process-utils'
import { setDecryptedCreds } from '../packages/server/src/connectors/decrypted-creds'
import { vorndSessions } from '../packages/server/src/vornd-sessions'
import { cancelScripts } from '../packages/server/src/vornd-scripts'
import { FakeVornd, effect } from './helpers/fake-vornd'
import { until } from './helpers/vornd-sessions'
import { normalizeScriptRun, type ScriptRun } from './helpers/scripts-parity'

/**
 * Project scripts with the Native server switch off, where the server runs
 * them, and on, where vornd does and the server follows the session it ran
 * each in: the same scripts answer and tell clients the same, but for the
 * differences `scripts-parity.ts` names. A fake vornd stands in, running each
 * script it is asked to as the session holder would, on pipes read as one
 * stream, and telling its output as records and its end as an exit effect.
 */

const SCENARIOS: Array<[string, Omit<ScriptConfig, 'cwd'>]> = [
  [
    'a bash script with arguments and a secret',
    {
      scriptType: 'bash',
      scriptContent: 'echo "out $1 $API_KEY"; pwd; ls',
      args: ['first', 'second'],
      secretsFrom: 'conn-1'
    }
  ],
  [
    'a bash script that fails, printing to both streams',
    { scriptType: 'bash', scriptContent: 'echo out; echo err >&2; exit 3' }
  ],
  ['a bash script that fails silently', { scriptType: 'bash', scriptContent: 'exit 4' }],
  [
    'a node script read from stdin',
    { scriptType: 'node', scriptContent: 'console.log("node", process.argv.length)' }
  ]
]

describe.skipIf(process.platform === 'win32')('project scripts run by vornd', () => {
  let dataDir: string
  let cwd: string
  let fake: FakeVornd
  let runs = 0

  /** Runs the script as vornd's session holder would, telling the server as vornd does. */
  function runAsVornd(params: Record<string, unknown>): void {
    const id = String(params.id)
    const interpreter = interpreterFor(String(params.scriptType))!
    let file = ''
    if (interpreter.file) {
      file = path.join(fs.mkdtempSync(path.join(dataDir, 'fake-')), interpreter.file)
      fs.writeFileSync(file, String(params.scriptContent))
    }
    const child = spawn(
      interpreter.command(false),
      [...interpreter.args(file), ...(params.args as string[])],
      {
        cwd: String(params.cwd),
        env: { ...process.env, ...(params.secretEnv as Record<string, string>) },
        stdio: [file ? 'ignore' : 'pipe', 'pipe', 'pipe']
      }
    )
    let rseq = 0
    const told = (data: Buffer): void => fake.sendOutput(id, 7, rseq++, data.toString())
    child.stdout!.on('data', told)
    child.stderr!.on('data', told)
    child.on('close', (code) => {
      fake.send('vornd:effect', effect(id, 'exit', rseq, { exitCode: code ?? 1 }))
      fake.send('terminal:exit', { id, exitCode: code ?? 1 })
    })
    if (!file) child.stdin!.end(String(params.scriptContent))
  }

  async function run(config: ScriptConfig): Promise<ScriptRun> {
    const runId = `run-${++runs}`
    const told: string[] = []
    const exits: number[] = []
    const onData = (p: { runId: string; data: string }): void => {
      if (p.runId === runId) told.push(p.data)
    }
    const onExit = (p: { runId: string; exitCode: number }): void => {
      if (p.runId === runId) exits.push(p.exitCode)
    }
    scriptRunnerEvents.on(IPC.SCRIPT_DATA, onData)
    scriptRunnerEvents.on(IPC.SCRIPT_EXIT, onExit)
    try {
      const result = await executeScript({ ...config, runId })
      return { result, told, exits }
    } finally {
      scriptRunnerEvents.off(IPC.SCRIPT_DATA, onData)
      scriptRunnerEvents.off(IPC.SCRIPT_EXIT, onExit)
    }
  }

  /** Connects again, so the server reads what `vornd:hello` now says of scripts. */
  async function scripts(mode: FakeVornd['scripts']): Promise<void> {
    fake.scripts = mode
    await vorndSessions.connect(fake.endpoint)
    expect(vorndSessions.scriptMode()).toBe(mode)
  }

  beforeAll(async () => {
    dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vornd-scripts-'))
    cwd = fs.mkdtempSync(path.join(os.tmpdir(), 'vornd-scripts-cwd-'))
    fs.writeFileSync(path.join(cwd, 'present.txt'), '')
    initDatabase(dataDir)
    setLaunchDataDir(dataDir)
    setDecryptedCreds('conn-1', { apiKey: 'sk-test' })
    fake = new FakeVornd(dataDir)
    fake.onScript = runAsVornd
    await fake.start()
  })

  afterAll(async () => {
    vorndSessions.close()
    await fake.stop()
    closeDatabase()
    fs.rmSync(dataDir, { recursive: true, force: true })
    fs.rmSync(cwd, { recursive: true, force: true })
  })

  it.each(SCENARIOS)('answers %s as the server does', async (_, script) => {
    const config = { ...script, cwd }
    const before = fake.made('vornd:script').length
    await scripts(null)
    const server = await run(config)
    expect(fake.made('vornd:script')).toHaveLength(before)

    await scripts('native')
    const vornd = await run(config)
    const asked = fake.made('vornd:script').slice(before)
    expect(asked).toHaveLength(1)

    expect(normalizeScriptRun(vornd, 'vornd')).toEqual(normalizeScriptRun(server, 'server'))
    expect(server.exits).toHaveLength(1)
    // vornd is sent the step resolved: its cwd, and its secrets by value.
    expect(asked[0]).toEqual({
      id: expect.stringMatching(/^script-/),
      scriptType: script.scriptType,
      scriptContent: script.scriptContent,
      cwd,
      args: script.args ?? [],
      secretEnv: script.secretsFrom ? { API_KEY: 'sk-test' } : {}
    })
  })

  it('tells vornd what it started, to compare, and runs it itself in shadow mode', async () => {
    await scripts('shadow')
    const before = fake.made('vornd:script').length
    const { result } = await run({
      scriptType: 'bash',
      scriptContent: 'echo shadowed',
      args: ['x'],
      cwd,
      secretsFrom: 'conn-1'
    })
    expect(result).toEqual({ success: true, output: 'shadowed\n', exitCode: 0 })
    expect(fake.made('vornd:script')).toHaveLength(before)
    await until('the plan', () => fake.made('vornd:scriptPlan').length === 1)
    const [told] = fake.made('vornd:scriptPlan')
    const plan = told.plan as { argv: string[]; cwd: string; envKeys: string[] }
    expect(told).toMatchObject({ scriptType: 'bash', cwd, args: ['x'], secretKeys: ['API_KEY'] })
    expect(plan.argv).toEqual(['bash', '<script>', 'x'])
    expect(plan.cwd).toBe(cwd)
    expect(plan.envKeys).toContain('API_KEY')
    expect(plan.envKeys).toContain('VORN_DATA_DIR')
    expect(plan.envKeys).toEqual([...plan.envKeys].sort())
    // Names only: the secret's value never leaves this server in shadow mode.
    expect(JSON.stringify(told)).not.toContain('sk-test')
  })

  it('runs a script itself when vornd refuses it, as nothing ran there', async () => {
    await scripts('native')
    fake.scriptError = 'vornd cannot read the settings'
    try {
      const { result, exits } = await run({ scriptType: 'bash', scriptContent: 'echo here', cwd })
      expect(result).toEqual({ success: true, output: 'here\n', exitCode: 0 })
      expect(exits).toEqual([0])
    } finally {
      fake.scriptError = null
    }
  })

  it('runs a script itself while vornd is not connected', async () => {
    await scripts('native')
    vorndSessions.close()
    expect(vorndSessions.scriptMode()).toBeNull()
    const before = fake.made('vornd:script').length
    const { result } = await run({ scriptType: 'bash', scriptContent: 'echo alone', cwd })
    expect(result.output).toBe('alone\n')
    expect(fake.made('vornd:script')).toHaveLength(before)
  })

  it('cancels the scripts vornd runs when the server shuts down', async () => {
    await scripts('native')
    let id = ''
    fake.onScript = (params) => {
      id = String(params.id)
    }
    try {
      const running = run({ scriptType: 'bash', scriptContent: 'sleep 60', cwd })
      await until('the script', () => id !== '')
      cancelScripts()
      await until('the cancel', () => fake.made('vornd:scriptCancel').length === 1)
      expect(fake.made('vornd:scriptCancel')).toEqual([{ id }])
      fake.sendOutput(id, 7, 0, 'stopping\n')
      fake.send('vornd:effect', effect(id, 'exit', 1, { exitCode: 143 }))
      fake.send('terminal:exit', { id, exitCode: 143 })
      const { result, told, exits } = await running
      expect(result).toEqual({
        success: false,
        output: 'stopping\n',
        error: 'stopping\n',
        exitCode: 143
      })
      expect(told).toEqual(['stopping\n'])
      expect(exits).toEqual([143])
      cancelScripts()
      expect(fake.made('vornd:scriptCancel')).toHaveLength(1)
    } finally {
      fake.onScript = runAsVornd
    }
  })
})
