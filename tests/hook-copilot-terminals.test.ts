import { describe, it, expect, vi, beforeAll, afterAll, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { runCopilotHook } from './helpers/copilot-hook'

// Two Copilot terminals in one folder used to share, and overwrite, one hook session id.

const log = vi.hoisted(() => ({ info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }))
vi.mock('../packages/server/src/logger', () => ({ default: log }))
vi.mock('../packages/server/src/vornd-sessions', async () =>
  (await import('./helpers/fake-vornd-pty')).vorndModule()
)
vi.mock('../packages/server/src/git-utils', () => ({
  getGitBranch: vi.fn(async () => 'main'),
  getGitHead: vi.fn(async () => 'cafe0000'),
  checkoutBranch: vi.fn(async () => {}),
  createWorktree: vi.fn(),
  isGitRepo: vi.fn(async () => false)
}))

import type { HookEvent, TerminalSession } from '@vornrun/shared/types'
import { ptyManager } from '../packages/server/src/pty-manager'
import { hookStatusMapper } from '../packages/server/src/hook-status-mapper'
import { fakeVornd } from './helpers/fake-vornd-pty'
import {
  installCopilotHooks,
  uninstallAllCopilotHooks
} from '../packages/server/src/copilot-hook-installer'

// `hook-server` reads `homedir()` at module scope, so it is imported only after the fake home.
const saved: Record<string, string | undefined> = {}
let home = ''
let project = ''

function restore(name: string): void {
  if (saved[name] === undefined) delete process.env[name]
  else process.env[name] = saved[name]
}

beforeAll(() => {
  for (const name of ['HOME', 'USERPROFILE', 'COPILOT_HOME']) saved[name] = process.env[name]
  home = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-copilot-home-'))
  process.env.HOME = home
  process.env.USERPROFILE = home
  delete process.env.COPILOT_HOME
})

afterAll(() => {
  for (const name of Object.keys(saved)) restore(name)
  fs.rmSync(home, { recursive: true, force: true })
})

let stopServer: (() => void) | null = null
const seen: HookEvent[] = []

beforeEach(() => {
  fakeVornd.reset()
  hookStatusMapper.clear()
  log.warn.mockClear()
  seen.length = 0
  project = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-copilot-project-'))
})

afterEach(() => {
  uninstallAllCopilotHooks()
  stopServer?.()
  stopServer = null
  fs.rmSync(project, { recursive: true, force: true })
})

async function startHookServer(): Promise<number> {
  const { HookServer } = await import('../packages/server/src/hook-server')
  const instance = new HookServer()
  const port = await instance.start(0)
  instance.on('hook-event', (event: HookEvent) => seen.push(event))
  stopServer = () => instance.stop()
  return port
}

async function launchCopilot(): Promise<TerminalSession> {
  const session = await ptyManager.createPty({
    agentType: 'copilot',
    projectName: 'p',
    projectPath: project
  } as never)
  await new Promise((resolve) => setTimeout(resolve, 5))
  return session
}

/** Runs the `bash` command Copilot would for `event`, in a terminal's environment. */
function runHook(
  hooksJsonPath: string,
  event: string,
  env: Record<string, string | undefined>,
  payload: string | Buffer = JSON.stringify({ cwd: project, toolName: 'bash' })
): Promise<NodeJS.ErrnoException | null> {
  const hooks = JSON.parse(fs.readFileSync(hooksJsonPath, 'utf-8'))
  return runCopilotHook(hooks.hooks[event][0].bash, env, payload)
}

async function until(check: () => boolean): Promise<void> {
  for (let i = 0; i < 300 && !check(); i++) await new Promise((r) => setTimeout(r, 10))
}

const listed = (id: string): TerminalSession | undefined =>
  ptyManager.getActiveSessions().find((s) => s.id === id)

describe.skipIf(process.platform === 'win32')('two Copilot terminals in one folder', () => {
  it(
    "each terminal's hooks reach that terminal under its own hook session",
    { timeout: 20_000 },
    async () => {
      await startHookServer()
      const first = await launchCopilot()
      const second = await launchCopilot()

      // As the server does when each terminal is created.
      const installs = [first, second].map((terminal) => {
        const installation = installCopilotHooks(terminal.id)
        hookStatusMapper.forceLink(installation.sessionId, terminal.id)
        ptyManager.linkHookSession(terminal.id, installation.sessionId)
        return installation
      })

      await runHook(installs[0].hooksJsonPath, 'preToolUse', { VORN_SESSION_ID: first.id })
      await until(() => seen.length === 1)
      await runHook(installs[1].hooksJsonPath, 'preToolUse', { VORN_SESSION_ID: second.id })
      await until(() => seen.length === 2)

      expect(seen.map((event) => hookStatusMapper.mapEventToStatus(event)?.terminalId)).toEqual([
        first.id,
        second.id
      ])
      expect(seen[0].session_id).not.toBe(seen[1].session_id)
      expect(listed(first.id)?.hookSessionId).toBe(seen[0].session_id)
      expect(listed(second.id)?.hookSessionId).toBe(seen[1].session_id)
      expect(installs[0].sessionId).toBe(seen[0].session_id)
      expect(installs[1].sessionId).toBe(seen[1].session_id)
    }
  )

  it("share one file in Copilot's user hooks, written once and naming no terminal", async () => {
    const first = await launchCopilot()
    const second = await launchCopilot()
    const { hooksJsonPath } = installCopilotHooks(first.id)
    const written = fs.readFileSync(hooksJsonPath, 'utf-8')
    const mtime = fs.statSync(hooksJsonPath).mtimeMs
    await new Promise((resolve) => setTimeout(resolve, 20))

    expect(installCopilotHooks(second.id).hooksJsonPath).toBe(hooksJsonPath)
    expect(hooksJsonPath).toBe(path.join(home, '.copilot', 'hooks', 'vorn.json'))
    expect(fs.readFileSync(hooksJsonPath, 'utf-8')).toBe(written)
    expect(fs.statSync(hooksJsonPath).mtimeMs).toBe(mtime)
    expect(written).not.toContain(first.id)
    expect(fs.readdirSync(project)).toEqual([])
  })
})

describe.skipIf(process.platform === 'win32')('the shared Copilot hooks file', () => {
  it('posts nothing for a Copilot started outside Vorn', { timeout: 20_000 }, async () => {
    await startHookServer()
    const { hooksJsonPath } = installCopilotHooks('term-x')
    await runHook(hooksJsonPath, 'sessionStart', { VORN_SESSION_ID: undefined })
    await runHook(hooksJsonPath, 'sessionStart', { VORN_SESSION_ID: 'term-x' })
    await until(() => seen.length === 1)
    await new Promise((resolve) => setTimeout(resolve, 50))
    expect(seen).toEqual([expect.objectContaining({ vorn_terminal_id: 'term-x' })])
  })

  it('reads the whole event for a Copilot started outside Vorn', { timeout: 20_000 }, async () => {
    const { hooksJsonPath } = installCopilotHooks('term-x')
    // Larger than a pipe's buffer, so a hook that exits unread would fail the write.
    const big = Buffer.alloc(256 * 1024, ' ')
    expect(
      await runHook(hooksJsonPath, 'preToolUse', { VORN_SESSION_ID: undefined }, big)
    ).toBeNull()
  })

  it('is written under COPILOT_HOME when that is set', () => {
    const copilotHome = path.join(home, 'elsewhere')
    process.env.COPILOT_HOME = copilotHome
    try {
      const { hooksJsonPath } = installCopilotHooks('term-x')
      expect(hooksJsonPath).toBe(path.join(copilotHome, 'hooks', 'vorn.json'))
      expect(fs.existsSync(hooksJsonPath)).toBe(true)
    } finally {
      delete process.env.COPILOT_HOME
    }
  })

  it('is removed at shutdown, once, and only when this process installed it', () => {
    uninstallAllCopilotHooks()
    const { hooksJsonPath } = installCopilotHooks('term-x')
    uninstallAllCopilotHooks()
    expect(fs.existsSync(hooksJsonPath)).toBe(false)

    fs.mkdirSync(path.dirname(hooksJsonPath), { recursive: true })
    fs.writeFileSync(hooksJsonPath, JSON.stringify({ version: 1, _vorn: true, hooks: {} }))
    uninstallAllCopilotHooks()
    expect(fs.existsSync(hooksJsonPath)).toBe(true)
    fs.rmSync(hooksJsonPath)
  })

  it.each([
    ['hooks of its own', JSON.stringify({ version: 1, hooks: {} })],
    ['a file it cannot parse', '{ not json']
  ])('leaves a vorn.json holding %s alone', (_label, content) => {
    const hooksJsonPath = path.join(home, '.copilot', 'hooks', 'vorn.json')
    fs.mkdirSync(path.dirname(hooksJsonPath), { recursive: true })
    fs.writeFileSync(hooksJsonPath, content)
    try {
      expect(installCopilotHooks('term-x').hooksJsonPath).toBe(hooksJsonPath)
      uninstallAllCopilotHooks()
      expect(fs.readFileSync(hooksJsonPath, 'utf-8')).toBe(content)
      expect(log.warn).toHaveBeenCalled()
    } finally {
      fs.rmSync(hooksJsonPath)
    }
  })
})
