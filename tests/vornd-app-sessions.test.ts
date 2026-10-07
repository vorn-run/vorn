import { describe, it, expect, vi, beforeAll, afterAll } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))
vi.mock('../packages/server/src/config-manager', () => ({
  configManager: {
    loadConfig: () => ({ defaults: {} }),
    onChange: () => () => {}
  }
}))

import { initDatabase, closeDatabase } from '../packages/server/src/database'
import { IPC } from '../packages/shared/src/types'
import { ptyManager } from '../packages/server/src/pty-manager'
import { headlessManager } from '../packages/server/src/headless-manager'
import { vorndSessions } from '../packages/server/src/vornd-sessions'
import {
  vorndSessionsAvailable,
  upstream,
  Vornd,
  BytesClient,
  until,
  home as testHome,
  killPid,
  announcedEndpoint
} from './helpers/vornd-sessions'

/**
 * The app's terminals and headless agents, started in vornd's holder rather
 * than here, and outliving vornd.
 */

/** Every headless exit the server tells clients of, until the returned stop. */
function onHeadlessExit(cb: (e: { id: string; exitCode: number }) => void): () => void {
  const listener = (channel: string, payload: unknown): void => {
    if (channel === IPC.HEADLESS_EXIT) cb(payload as { id: string; exitCode: number })
  }
  headlessManager.on('client-message', listener)
  return () => headlessManager.off('client-message', listener)
}

describe.skipIf(!vorndSessionsAvailable)('sessions through vornd', () => {
  let up: Awaited<ReturnType<typeof upstream>>
  let h: ReturnType<typeof testHome>
  let vornd: Vornd
  const messages: Array<{ channel: string; payload: Record<string, unknown> }> = []
  const record = (channel: string, payload: Record<string, unknown>): void => {
    messages.push({ channel, payload })
  }

  beforeAll(async () => {
    up = await upstream()
    h = testHome()
    initDatabase(h.dir)
    vornd = await Vornd.start(up.port, h.dir)
    await until('the announcement', () => announcedEndpoint(h.dir) !== null)
    expect(await vorndSessions.connect(announcedEndpoint(h.dir)!)).toBe(true)
    ptyManager.on('client-message', record)
    headlessManager.on('client-message', record)
  })

  afterAll(async () => {
    ptyManager.off('client-message', record)
    headlessManager.off('client-message', record)
    vorndSessions.close()
    const pid = await vornd.sessiondPid().catch(() => null)
    await vornd.kill()
    killPid(pid)
    closeDatabase()
    up.close()
    h.remove()
  })

  it('starts a shell in vornd, under the session id, and nothing here holds a PTY', async () => {
    const session = ptyManager.createShellPty(os.tmpdir())
    await until(
      'its pid',
      () => (ptyManager.getActiveSessions().find((s) => s.id === session.id)?.pid ?? 0) > 0
    )

    const client = new BytesClient()
    await client.connect(vornd.port)
    await client.attach(session.id)
    ptyManager.writeToPty(session.id, 'echo shell-$((6*7))\r')
    await until('the echo', async () => (await client.text()).includes('shell-42'))
    // Nothing of its output came through this server.
    expect(
      messages.some((m) => m.channel === IPC.TERMINAL_DATA && m.payload.id === session.id)
    ).toBe(false)
    expect(await ptyManager.readOutput(session.id)).toEqual(
      expect.arrayContaining([expect.stringContaining('shell-42')])
    )

    ptyManager.writeToPty(session.id, 'exit 4\r')
    await until('the exit', () =>
      messages.some((m) => m.channel === IPC.TERMINAL_EXIT && m.payload.id === session.id)
    )
    const exit = messages.find(
      (m) => m.channel === IPC.TERMINAL_EXIT && m.payload.id === session.id
    )!
    expect(exit.payload.exitCode).toBe(4)
    expect(ptyManager.hasLivePty(session.id)).toBe(false)
    expect(ptyManager.getActiveSessions().find((s) => s.id === session.id)?.shellExitCode).toBe(4)
    client.close()
  })

  // A login shell on a busy runner can take seconds to answer.
  it('tells a shell in vornd it runs in xterm-256color, as a PTY here would', async () => {
    const before = process.env.TERM
    process.env.TERM = 'dumb'
    const session = ptyManager.createShellPty(os.tmpdir())
    if (before === undefined) delete process.env.TERM
    else process.env.TERM = before
    await until(
      'its pid',
      () => (ptyManager.getActiveSessions().find((s) => s.id === session.id)?.pid ?? 0) > 0
    )

    const client = new BytesClient()
    await client.connect(vornd.port)
    await client.attach(session.id)
    ptyManager.writeToPty(session.id, 'echo "term=[$TERM]"\r')
    // The command line echoes back as `[$TERM]`: wait for what the shell printed.
    await until('the echo', async () => /term=\[[^$]*\]/.test(await client.text()))
    expect(await client.text()).toContain('term=[xterm-256color]')
    ptyManager.writeToPty(session.id, 'exit\r')
    client.close()
  }, 20_000)

  it('reports where a shell moved from what vornd parsed', async () => {
    const cwds: string[] = []
    const onCwd = (_id: string, cwd: string): void => {
      cwds.push(cwd)
    }
    ptyManager.on('session-cwd', onCwd)
    const session = ptyManager.createShellPty(os.tmpdir())
    const target = fs.realpathSync(h.dir)
    // Moved for real, so a shell whose integration reports each prompt's
    // directory agrees with the report written here.
    ptyManager.writeToPty(
      session.id,
      `cd '${target}' && printf '\\033]5522;cwd;%s\\007' '${target}'\r`
    )
    await until('the directory', () => cwds.includes(target))
    expect(ptyManager.getActiveSessions().find((s) => s.id === session.id)?.shellCwd).toBe(target)
    ptyManager.off('session-cwd', onCwd)
    ptyManager.killPty(session.id)
  })

  it("takes an agent's status from vornd, and resumes it under the same id", async () => {
    const agent = path.join(h.dir, 'fake-tui')
    fs.writeFileSync(agent, "#!/bin/sh\nprintf '\\033[?2004h'\nexec sleep 30\n", { mode: 0o755 })
    ptyManager.setAgentCommands({ claude: { command: agent, args: [] } })
    const payload = { agentType: 'claude' as const, projectName: 'p', projectPath: h.dir }
    const session = await ptyManager.createPty({ ...payload })
    await until('waiting', () =>
      messages.some(
        (m) =>
          m.channel === IPC.SESSION_UPDATED &&
          m.payload.id === session.id &&
          m.payload.status === 'waiting'
      )
    )

    // Resumed: the old run is let go and a new one starts under the same name.
    ptyManager.killPty(session.id)
    await until('the old run to end', async () =>
      (await vornd.report()).sessions.every((s) => s.session !== session.id)
    )
    const again = await ptyManager.createPty({ ...payload }, session.id)
    expect(again.id).toBe(session.id)
    await until('the new run', async () =>
      (await vornd.report()).sessions.some((s) => s.session === session.id)
    )
    ptyManager.killPty(session.id)
  }, 15_000)

  it('runs a headless agent in vornd: its prompt in, its output out, its exit told once', async () => {
    const agent = path.join(h.dir, 'fake-agent')
    fs.writeFileSync(agent, '#!/bin/sh\nprintf "prompt: "\ncat\nexit 5\n', { mode: 0o755 })
    headlessManager.setAgentCommands({ claude: { command: agent, args: [] } })
    const exits: Array<{ id: string; exitCode: number }> = []
    const stop = onHeadlessExit((e: { id: string; exitCode: number }) => exits.push(e))

    const session = await headlessManager.createHeadless({
      agentType: 'claude',
      projectName: 'p',
      projectPath: h.dir,
      initialPrompt: 'write the tests'
    })
    await until('the exit', () => exits.some((e) => e.id === session.id))
    stop()
    const data = messages
      .filter((m) => m.channel === IPC.HEADLESS_DATA && m.payload.id === session.id)
      .map((m) => m.payload.data)
      .join('')
    expect(data).toContain('prompt: write the tests')
    expect(headlessManager.getOutput(session.id).join('\n')).toContain('write the tests')
    expect(exits.filter((e) => e.id === session.id)).toEqual([{ id: session.id, exitCode: 5 }])
    expect(headlessManager.getActiveSessions().find((s) => s.id === session.id)?.status).toBe(
      'exited'
    )
  })

  it('leaves its sessions running when this server stops, and vornd keeps them', async () => {
    const session = ptyManager.createShellPty(os.tmpdir())
    await until(
      'its pid',
      () => (ptyManager.getActiveSessions().find((s) => s.id === session.id)?.pid ?? 0) > 0
    )
    ptyManager.killAll()
    // A report taken while vornd is busy can come back empty; one that lists
    // the session shows it outlived this server's sessions.
    let listed = false
    await until('vornd to report the session', async () => {
      listed = (await vornd.report()).sessions.some((s) => s.session === session.id)
      return listed
    })
    expect(listed).toBe(true)
  }, 20_000)
})
