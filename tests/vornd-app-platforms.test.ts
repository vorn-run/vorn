import { describe, it, expect, vi, beforeAll, afterAll } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))
// The Native daemon switch on, as Settings › Experimental sets it.
vi.mock('../packages/server/src/config-manager', () => ({
  configManager: {
    loadConfig: () => ({ defaults: { experimental: { vornd: true } } }),
    onChange: () => () => {}
  }
}))

import { initDatabase, closeDatabase } from '../packages/server/src/database'
import { IPC } from '../packages/shared/src/types'
import { ptyManager } from '../packages/server/src/pty-manager'
import { headlessManager } from '../packages/server/src/headless-manager'
import { onHeadlessExit } from '../packages/server/src/workflows/host'
import {
  VorndSessions,
  vorndSessions,
  announcedEndpoint,
  type VorndExit
} from '../packages/server/src/vornd-sessions'
import {
  vorndBinariesAvailable,
  upstream,
  Vornd,
  BytesClient,
  until,
  home as testHome,
  killPid
} from './helpers/vornd-sessions'

/**
 * The server driving sessions through the real vornd on every platform the
 * app ships on, Windows included, where vornd's channel is a named pipe and
 * its sessions run in ConPTY. The programs are Node scripts, so nothing here
 * depends on a POSIX shell.
 */
describe.skipIf(!vorndBinariesAvailable)('sessions through vornd on this platform', () => {
  let up: Awaited<ReturnType<typeof upstream>>
  let h: ReturnType<typeof testHome>
  let vornd: Vornd
  const messages: Array<{ channel: string; payload: Record<string, unknown> }> = []
  const record = (channel: string, payload: Record<string, unknown>): void => {
    messages.push({ channel, payload })
  }

  /** A Node script under the test's home, run with this Node. */
  const script = (name: string, body: string): string[] => {
    const file = path.join(h.dir, `${name}.cjs`)
    fs.writeFileSync(file, body)
    return [process.execPath, file]
  }

  beforeAll(async () => {
    up = await upstream()
    h = testHome()
    initDatabase(h.dir)
    vornd = await Vornd.start(up.port, h.dir)
    await until('the announcement', () => announcedEndpoint(h.dir) !== null)
    expect(await vorndSessions.connect()).toBe(true)
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

  it('runs a shell in vornd: its output read back, its exit code told', async () => {
    const session = ptyManager.createShellPty(os.tmpdir())
    expect(ptyManager.isInVornd(session.id)).toBe(true)
    await until(
      'its pid',
      () => (ptyManager.getActiveSessions().find((s) => s.id === session.id)?.pid ?? 0) > 0
    )
    // Every shell the app starts on any platform runs `echo` and `exit`.
    ptyManager.writeToPty(session.id, 'echo vorn-shell-ok\r')
    await until('the echo', async () =>
      (await ptyManager.readOutput(session.id)).some((l) => l.includes('vorn-shell-ok'))
    )
    expect(
      messages.some((m) => m.channel === IPC.TERMINAL_DATA && m.payload.id === session.id)
    ).toBe(false)

    ptyManager.writeToPty(session.id, 'exit 4\r')
    await until('the exit', () =>
      messages.some((m) => m.channel === IPC.TERMINAL_EXIT && m.payload.id === session.id)
    )
    const exit = messages.find(
      (m) => m.channel === IPC.TERMINAL_EXIT && m.payload.id === session.id
    )!
    expect(exit.payload.exitCode).toBe(4)
  }, 30_000)

  it('runs a headless agent on pipes: its prompt in, its output out, its exit told once', async () => {
    const [command, ...args] = script(
      'fake-agent',
      "process.stdout.write('prompt: ')\n" +
        'process.stdin.pipe(process.stdout)\n' +
        "process.stdin.on('end', () => process.exit(5))\n"
    )
    headlessManager.setAgentCommands({ claude: { command: command!, args } })
    const exits: Array<{ id: string; exitCode: number }> = []
    const stop = onHeadlessExit((e) => exits.push(e))
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
    expect(exits.filter((e) => e.id === session.id)).toEqual([{ id: session.id, exitCode: 5 }])
  }, 30_000)

  it('keeps a session when vornd is killed, and carries on with the next vornd', async () => {
    const sessions = new VorndSessions()
    expect(await sessions.connect(announcedEndpoint(h.dir)!)).toBe(true)
    const pty = sessions.spawn(
      'pane-survives',
      {
        argv: script(
          'echo-lines',
          "console.log('ready-' + (40 + 2))\n" +
            "require('readline').createInterface({ input: process.stdin })" +
            ".on('line', (l) => console.log('echo:' + l.trim()))\n"
        ),
        cwd: os.tmpdir(),
        env: {},
        cols: 80,
        rows: 24
      },
      false
    )
    await until('the spawn', () => pty.pid !== 0)
    const ended: VorndExit[] = []
    pty.onExit((e) => ended.push(e))
    await until('the first line', async () =>
      (await sessions.readOutput('pane-survives')).some((l) => l.includes('ready-42'))
    )

    await vornd.kill()
    vornd = await Vornd.start(up.port, h.dir)
    await until('the new announcement', () => announcedEndpoint(h.dir) !== null)
    expect(await sessions.connect(announcedEndpoint(h.dir)!)).toBe(true)
    expect(ended).toEqual([])

    const client = new BytesClient()
    await client.connect(vornd.port)
    await client.attach('pane-survives')
    expect(await client.text()).toContain('ready-42')
    pty.write('after-restart\r')
    await until('the echo', async () => (await client.text()).includes('echo:after-restart'))
    client.close()

    pty.kill('SIGKILL')
    await until('the exit', () => ended.length === 1)
    sessions.close()
    // The server's own channel, killed with the old vornd, finds the new one.
    expect(await vorndSessions.connect()).toBe(true)
  }, 30_000)
})
