/**
 * Terminals on a remote host, created and resumed by vornd.
 *
 * `ssh` is a stub first on the login shell's PATH: it records its arguments
 * and the password it is given, prints the ready marker and runs what it was
 * asked to on this machine, as a remote login shell would. Each run creates a
 * session on a host that logs in by password and one by key file, lets the
 * second end and resumes it, and is compared as one transcript, normalized by
 * `tests/helpers/sessions-parity.ts`, with `tests/fixtures/vornd/ssh.json`.
 *
 * Runs where vornd and vorn-sessiond have been built (`yarn build:core`, or
 * the binaries in `VORN_CONFORMANCE_VORND`), on a Unix.
 */
import fs from 'node:fs'
import path from 'node:path'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import type { TerminalSession } from '../packages/shared/src/types'
import { spawnsRealServers } from './helpers/one-at-a-time'
import {
  Watcher,
  answered,
  removeRealServerDirs,
  runnable,
  startRealServer,
  stopRealServer,
  until,
  type Frame,
  type RealServer
} from './helpers/real-server'
import { PASSWORD_BEFORE_PROMPT, normalizeRun } from './helpers/sessions-parity'
import { recorded } from './helpers/vornd-fixtures'

// Booting a server probes Tailscale with a real process; nothing here needs it.
vi.mock('../packages/server/src/tailscale', () => ({
  getTailscaleStatus: vi.fn(async () => ({ running: false, selfIP: '', selfDNSName: '' })),
  clearBinaryCache: vi.fn()
}))

spawnsRealServers()

const PASSWORD = 'pw-parity-secret'

/** ssh as these runs see it; `LOG` and `REMOTE_SHELL` are filled in per run. */
const FAKE_SSH = `#!/bin/sh
us=$(printf '\\037')
line=
for a in "$@"; do line="$line$a$us"; done
printf '%s\\n' "$line" >> "LOG/argv"
case " $* " in *PreferredAuthentications=password*)
  printf "me@box.example's password: "
  stty -echo 2>/dev/null
  IFS= read -r pw
  stty echo 2>/dev/null
  printf '\\r\\n'
  printf '%s\\n' "$pw" >> "LOG/passwords";;
esac
for last in "$@"; do :; done
SHELL="REMOTE_SHELL" exec /bin/sh -c "$last"
`

/** The remote login shell: runs the one line typed into it, then ends. */
const REMOTE_SHELL = `#!/bin/sh
IFS= read -r line
printf '%s\\n' "$line" >> "LOG/remote"
eval "$line"
`

/** An agent that says what it was started with and ends. */
const ARGV_AGENT = `#!/bin/sh
printf 'ARGV:%s\\n' "$*"
`

/** One run's lines of a log the stubs write, or none yet. */
function lines(file: string): string[] {
  try {
    return fs.readFileSync(file, 'utf-8').split('\n').filter(Boolean)
  } catch {
    return []
  }
}

async function scenario(server: RealServer): Promise<Record<string, unknown>> {
  const { work, home } = server.dirs
  const log = path.join(work, 'ssh-log')
  const bin = path.join(work, 'bin')
  fs.mkdirSync(log)
  fs.mkdirSync(bin)
  const remoteShell = path.join(bin, 'remote-shell')
  fs.writeFileSync(remoteShell, REMOTE_SHELL.replaceAll('LOG', log), { mode: 0o755 })
  fs.writeFileSync(
    path.join(bin, 'ssh'),
    FAKE_SSH.replaceAll('LOG', log).replace('REMOTE_SHELL', remoteShell),
    { mode: 0o755 }
  )
  const stub = path.join(bin, 'argv-agent')
  fs.writeFileSync(stub, ARGV_AGENT, { mode: 0o755 })
  // The login shell reads this after the system's profile, which reorders PATH.
  fs.writeFileSync(path.join(home, '.profile'), `PATH="${bin}:$PATH"; export PATH\n`)
  const project = path.join(work, 'far')
  fs.mkdirSync(project)

  const direct = await Watcher.open(server.port)
  const through = await Watcher.open(server.vornd)
  const replies: Record<string, unknown> = {}
  const call = async (step: string, method: string, params?: unknown): Promise<Frame> => {
    const frame = await through.call(method, params)
    replies[step] = answered(frame)
    return frame
  }
  const created = async (step: string, params: unknown): Promise<string> => {
    const frame = await call(step, 'terminal:create', params)
    if (frame.error) throw new Error(`${step}: ${JSON.stringify(frame.error)}`)
    return (frame.result as TerminalSession).id
  }
  const listed = (): Promise<TerminalSession[]> =>
    direct.result<TerminalSession[]>('terminal:listActive')
  const said = async (id: string, what: RegExp): Promise<string> => {
    let found = ''
    let shown: string[] = []
    await until(`${id} to show ${what}`, async () => {
      // Not started yet: the holder has no session under the id until then.
      const frame = await through.call('terminal:readOutput', { id })
      shown = (frame.result as string[] | undefined) ?? []
      found = shown.find((l) => what.test(l))?.trim() ?? ''
      return found !== ''
    }).catch((err: Error) => {
      throw new Error(`${err.message}; its screen: ${JSON.stringify(shown)}`)
    })
    return found.slice(found.search(what))
  }
  const exited = async (id: string): Promise<void> => {
    await until(`the exit of ${id}`, () =>
      direct.toldBy('terminal:exit').some((p) => (p as { id?: string }).id === id)
    )
  }
  try {
    const config = await direct.result<Record<string, unknown>>('config:load')
    await direct.result('config:save', {
      ...config,
      defaults: { ...(config.defaults as object), shell: '/bin/sh' },
      agentCommands: { claude: { command: stub, args: [] }, codex: { command: stub, args: [] } },
      remoteHosts: [
        {
          id: 'pw',
          label: 'Password box',
          hostname: 'box.example',
          user: 'me',
          port: 2222,
          authMethod: 'password',
          sshOptions: '-o StrictHostKeyChecking=no'
        },
        {
          id: 'key',
          label: 'Key box',
          hostname: 'keys.example',
          user: 'you',
          port: 22,
          authMethod: 'key-file',
          sshKeyPath: '/keys/id_test'
        }
      ]
    })

    const byPassword = await created('create on a password host', {
      agentType: 'claude',
      projectName: 'far',
      projectPath: project,
      remoteHostId: 'pw',
      initialPrompt: 'fix the remote build',
      _decryptedPassword: PASSWORD
    })
    const byKey = await created('create on a key host', {
      agentType: 'codex',
      projectName: 'far',
      projectPath: project,
      remoteHostId: 'key',
      displayName: 'Keyed'
    })
    const shown = { key: await said(byKey, /ARGV:/) }
    const passwordShown = await said(byPassword, /ARGV:/)
    const listedAtFirst = await listed()

    // The agent ended and ssh with it: the local shell is told to end too.
    through.notify('terminal:write', { id: byKey, data: 'exit\r' })
    await exited(byKey)
    await call('resume the ended session', 'sessions:resume', { id: byKey })
    const resumedShown = await said(byKey, /ARGV:/)
    await until('the resume to log in', () => lines(path.join(log, 'argv')).length === 3).catch(
      (err: Error) => {
        throw new Error(
          `${err.message}; ${JSON.stringify({ replies, log: lines(path.join(log, 'argv')) })}`
        )
      }
    )

    await call('close the password session', 'terminal:kill', byPassword)
    await exited(byPassword)
    await call('close the resumed session', 'terminal:kill', byKey)
    // vornd answers the close before the server hears of it.
    await until('the server to drop the resumed session', async () =>
      (await listed()).every((s) => s.id !== byKey)
    )

    return {
      replies,
      shown: { ...shown, resumed: resumedShown },
      passwordLogin: {
        shown: passwordShown,
        passwords: lines(path.join(log, 'passwords')),
        remote: lines(path.join(log, 'remote')).filter((l) => l.includes('fix the remote build'))
      },
      // The two creates log in at once, in an order of their own.
      sshArgv: lines(path.join(log, 'argv'))
        .sort()
        .map((l) => l.split('\x1f').filter(Boolean)),
      remote: lines(path.join(log, 'remote'))
        .filter((l) => !l.includes('fix the remote build'))
        .sort(),
      listedAtFirst,
      listed: await listed(),
      created: direct.toldBy('session:created')
    }
  } finally {
    direct.close()
    through.close()
  }
}

describe.skipIf(!runnable)('terminals on a remote host, through vornd', () => {
  let run: Record<string, unknown> = {}
  let counts: unknown

  beforeAll(async () => {
    const server = await startRealServer()
    try {
      run = normalizeRun(await scenario(server), server.dirs)
      const health = await fetch(`http://127.0.0.1:${server.vornd}/vornd/health`)
      const groups = ((await health.json()) as { groups: Record<string, { native?: number }> })
        .groups
      counts = { terminal: groups.terminal?.native, sessions: groups.sessions?.native }
    } catch (err) {
      throw new Error(`${(err as Error).message}\n${server.log.join('').slice(-4000)}`, {
        cause: err
      })
    } finally {
      await stopRealServer(server)
    }
  }, 240_000)

  afterAll(() => removeRealServerDirs())

  it('logs in by ssh with the options of each host', () => {
    const { sshArgv } = run as { sshArgv: string[][] }
    expect(sshArgv[2]).toEqual([
      '-t',
      '-p',
      '2222',
      '-o',
      'PreferredAuthentications=password',
      '-o',
      'PubkeyAuthentication=no',
      '-o',
      'StrictHostKeyChecking=no',
      'me@box.example',
      'echo __VORN_READY_<id>__ && exec $SHELL -l'
    ])
    expect(sshArgv[0]).toEqual([
      '-t',
      '-i',
      '/keys/id_test',
      'you@keys.example',
      'echo __VORN_READY_<id>__ && exec $SHELL -l'
    ])
    // The resume logs in again as the session did.
    expect(sshArgv[1]).toEqual(sshArgv[0])
  })

  it(`types the password once ssh asks, then the agent there (${PASSWORD_BEFORE_PROMPT})`, () => {
    expect(run.passwordLogin).toEqual({
      shown: 'ARGV:fix the remote build',
      passwords: [PASSWORD],
      remote: ["cd <work>/far && <work>/bin/argv-agent 'fix the remote build'"]
    })
  })

  it('has vornd create and resume them', () => {
    expect(counts).toEqual({ terminal: 4, sessions: 1 })
  })

  it('logs in, answers, tells and lists what it recorded', () => {
    const { replies, shown, sshArgv, remote, listedAtFirst, listed, created } = run
    const seen = { replies, shown, sshArgv, remote, listedAtFirst, listed, created }
    expect(seen).toEqual(recorded('ssh', seen))
    expect(JSON.stringify({ replies, listed, created })).not.toContain(PASSWORD)
  })
})
