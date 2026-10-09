/**
 * Sessions carried over a server restart, which vornd lists, attaches,
 * offers and resumes.
 *
 * One server starts sessions and stops; a second starts on the same
 * directories while the session holder still holds them, and must list them
 * and answer an attach for them through vornd as it does for a fresh one; a
 * third starts after the holder is gone too, and must offer them to resume,
 * start each again under its id, and close one without a program. The run is
 * one transcript, normalized by `tests/helpers/sessions-parity.ts`.
 *
 * Runs where vornd and vorn-sessiond have been built (`yarn build:core`, or
 * the binaries in `VORN_CONFORMANCE_VORND`), on a Unix.
 */
import fs from 'node:fs'
import path from 'node:path'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import type {
  HeadlessSession,
  RestoredSession,
  TerminalSession
} from '../packages/shared/src/types'
import { spawnsRealServers } from './helpers/one-at-a-time'
import {
  Watcher,
  answered,
  removeRealServerDirs,
  repository,
  runnable,
  startRealServer,
  stopRealServer,
  until,
  type Frame,
  type RealServer
} from './helpers/real-server'
import { normalizeRun, withoutRecordedHeads, type RunDirs } from './helpers/sessions-parity'
import { recorded } from './helpers/vornd-fixtures'

spawnsRealServers()

/** An agent that says what it was started with and waits; headless, it reads its prompt and waits. */
const ARGV_AGENT = `#!/bin/sh
printf 'ARGV:%s\\n' "$*"
if [ -t 0 ]; then exec sleep 600; fi
p=$(cat)
printf 'PROMPT:%s\\n' "$p"
sleep 600
`

interface AttachAnswer {
  live?: boolean
  replies?: string
  cols?: number
  rows?: number
  cursor?: object
  data?: string
}

/** What the restart scenario records, beside the transcript compared whole. */
interface Observed {
  transcript: Record<string, unknown>
  /** Whether the client attached to the cold pane was told to attach again once it ran. */
  resyncTold: boolean
  /** The headless agents a server lists after a restart that kept the holder. */
  headlessAfterRestart: number
  /** How often the shell echoed the resumed agent's launch line. */
  launchLinesEchoed: number
}

function byName(sessions: TerminalSession[]): TerminalSession[] {
  return [...sessions].sort((a, b) => (a.displayName ?? '').localeCompare(b.displayName ?? ''))
}

async function listed(client: Watcher): Promise<TerminalSession[]> {
  return client.result<TerminalSession[]>('terminal:listActive')
}

async function live(client: Watcher, ids: string[]): Promise<void> {
  await until('the sessions to start', async () => {
    const all = await listed(client)
    return ids.every((id) => (all.find((s) => s.id === id)?.pid ?? 0) > 0)
  })
}

async function shown(client: Watcher, id: string, text: string): Promise<void> {
  await until(`${text} on the screen`, async () => {
    const out = await client.result<string[]>('terminal:readOutput', { id })
    return out.some((line) => line.includes(text))
  })
}

/** The first run: a shell renamed and grouped, an agent in a repository, a second shell, a headless agent. */
async function firstRun(server: RealServer): Promise<{
  ids: Record<string, string>
  replies: Record<string, unknown>
}> {
  const { work } = server.dirs
  const stub = path.join(work, 'bin', 'argv-agent')
  fs.mkdirSync(path.dirname(stub), { recursive: true })
  fs.writeFileSync(stub, ARGV_AGENT, { mode: 0o755 })
  const repo = path.join(work, 'repo')
  repository(repo)
  const direct = await Watcher.open(server.port)
  const through = await Watcher.open(server.vornd)
  const replies: Record<string, unknown> = {}
  const call = async (step: string, method: string, params?: unknown): Promise<Frame> => {
    const frame = await through.call(method, params)
    replies[step] = answered(frame)
    return frame
  }
  try {
    const config = await direct.result<Record<string, unknown>>('config:load')
    await direct.result('config:save', {
      ...config,
      defaults: { ...(config.defaults as object), shell: '/bin/sh' },
      agentCommands: { claude: { command: stub, args: [] } }
    })
    const shellA = ((await call('shell A', 'shell:create', repo)).result as TerminalSession).id
    const shellB = ((await call('shell B', 'shell:create', repo)).result as TerminalSession).id
    const agent = (
      (
        await call('agent', 'terminal:create', {
          agentType: 'claude',
          projectName: 'repo',
          projectPath: repo,
          displayName: 'Agent'
        })
      ).result as TerminalSession
    ).id
    await live(through, [shellA, shellB, agent])
    await call('rename', 'terminal:rename', { id: shellA, displayName: 'Build' })
    await call('group', 'terminal:setGroup', { id: shellA, groupId: 'group-1' })
    through.notify('terminal:write', { id: shellA, data: 'echo carried-$((6*7))\r' })
    await shown(through, shellA, 'carried-42')
    await shown(through, agent, 'ARGV:')
    const headless = (
      (
        await call('headless', 'headless:create', {
          agentType: 'claude',
          projectName: 'repo',
          projectPath: repo,
          initialPrompt: 'wait here'
        })
      ).result as HeadlessSession
    ).id
    await until('the headless agent to say its prompt', () =>
      direct
        .toldBy('headless:data')
        .some(
          (p) =>
            (p as { id: string; data: string }).id === headless &&
            /wait here/.test((p as { data: string }).data)
        )
    )
    return { ids: { shellA, shellB, agent, headless }, replies }
  } finally {
    direct.close()
    through.close()
  }
}

/** The second run, while the holder still holds everything: listed and attached through vornd. */
async function warmRun(
  server: RealServer,
  ids: Record<string, string>
): Promise<{ replies: Record<string, unknown>; headless: number }> {
  const direct = await Watcher.open(server.port)
  const through = await Watcher.open(server.vornd)
  const replies: Record<string, unknown> = {}
  try {
    await live(through, [ids.shellA, ids.shellB, ids.agent])
    const all = await listed(through)
    // Taken on in the order the holder lists them, which is no order: by name.
    replies['listed after the restart'] = byName(all).map((s) => ({
      id: s.id,
      agentType: s.agentType,
      displayName: s.displayName,
      groupId: s.groupId,
      projectName: s.projectName,
      live: s.pid > 0
    }))
    // The acceptance: a carried session attaches through vornd with its screen, size and cursor.
    const attach = (await through.call('terminal:attach', { id: ids.shellA }))
      .result as AttachAnswer
    replies['attach to a carried shell'] = {
      replies: attach.replies,
      live: attach.live,
      sized: typeof attach.cols === 'number' && typeof attach.rows === 'number',
      cursor: typeof attach.cursor === 'object',
      showsItsScreen: (attach.data ?? '').includes('carried-42')
    }
    through.notify('terminal:write', { id: ids.shellA, data: 'echo again-$((7*6))\r' })
    await shown(through, ids.shellA, 'again-42')
    const fresh = (await through.call('terminal:attach', { id: ids.agent })).result as AttachAnswer
    replies['attach to a carried agent'] = { replies: fresh.replies, live: fresh.live }
    // Followed again a moment after the server takes stock of the holder.
    let carried: HeadlessSession[] = []
    const asked = Date.now()
    while (carried.length === 0 && Date.now() - asked < 5_000) {
      const headless = await direct.result<HeadlessSession[]>('headless:list')
      carried = headless.filter((h) => h.id === ids.headless && h.status === 'running')
      if (carried.length === 0) await new Promise((r) => setTimeout(r, 100))
    }
    for (const h of carried) await through.call('headless:kill', h.id)
    return { replies, headless: carried.length }
  } finally {
    direct.close()
    through.close()
  }
}

/** The third run, with the holder gone: offered, resumed under the same ids, one closed cold. */
async function coldRun(
  server: RealServer,
  ids: Record<string, string>
): Promise<{ replies: Record<string, unknown>; resyncTold: boolean; echoed: number }> {
  const direct = await Watcher.open(server.port)
  const through = await Watcher.open(server.vornd)
  const watcher = await Watcher.open(server.vornd)
  const replies: Record<string, unknown> = {}
  const call = async (step: string, method: string, params?: unknown): Promise<Frame> => {
    const frame = await through.call(method, params)
    replies[step] = answered(frame)
    return frame
  }
  try {
    let offered: RestoredSession[] = []
    await until('the offered sessions, looked at', async () => {
      offered = await through.result<RestoredSession[]>('sessions:restored')
      return (
        [ids.shellA, ids.shellB, ids.agent].every((id) =>
          offered.some((o) => o.session.id === id && o.environment !== undefined)
        ) && offered.length === 3
      )
    })
    replies['offered after the restart'] = withoutRecordedHeads(
      byName(offered.map((o) => o.session)).map((s) => {
        const o = offered.find((one) => one.session.id === s.id)!
        return {
          id: s.id,
          displayName: s.displayName,
          groupId: s.groupId,
          rebooted: o.rebooted,
          environment: o.environment
        }
      })
    )
    expect(await listed(through)).toEqual([])

    // A pane attached to the cold shell before anyone resumes it.
    const cold = (await watcher.call('terminal:attach', { id: ids.shellA })).result as AttachAnswer
    replies['attach to a cold shell'] = { live: cold.live, data: cold.data }

    const resumed = await call('resume the shell', 'sessions:resume', { id: ids.shellA })
    const session = (resumed.result as { session: TerminalSession }).session
    expect(session.id).toBe(ids.shellA)
    await live(through, [ids.shellA])
    const resyncTold = await (async () => {
      try {
        await until('the pane to be told to attach again', () =>
          watcher.toldBy('terminal:resync').some((p) => (p as { id: string }).id === ids.shellA)
        )
        return true
      } catch {
        return false
      }
    })()
    const again = (await watcher.call('terminal:attach', { id: ids.shellA })).result as AttachAnswer
    replies['attach after the resume'] = { replies: again.replies, live: again.live }
    through.notify('terminal:write', { id: ids.shellA, data: 'echo resumed-$((6*7))\r' })
    await shown(through, ids.shellA, 'resumed-42')

    // The agent, started again on its conversation: the launch line typed once.
    const agent = await call('resume the agent', 'sessions:resume', { id: ids.agent })
    expect((agent.result as { ok: boolean }).ok).toBe(true)
    await live(through, [ids.agent])
    await shown(through, ids.agent, 'ARGV:')
    const screen = await through.result<string[]>('terminal:readOutput', { id: ids.agent })
    replies['the agent was started once'] = screen.filter((l) => l.includes('ARGV:')).length
    // Kept apart from the transcript (TYPED_BEFORE_PROMPT).
    const echoed = screen.filter(
      (l) => l.includes('argv-agent --resume') && !l.includes('ARGV:')
    ).length
    replies['what the agent was resumed with'] = screen
      .filter((l) => l.includes('ARGV:'))
      .map((l) => l.slice(l.indexOf('ARGV:')).trimEnd())

    await call('resume one already running', 'sessions:resume', { id: ids.shellA })
    await call('resume one that is not there', 'sessions:resume', { id: 'no-such-session' })
    await call('close a cold shell', 'terminal:kill', ids.shellB)
    await new Promise((r) => setTimeout(r, 300))
    replies['exits told for the cold shell'] = direct
      .toldBy('terminal:exit')
      .filter((p) => (p as { id: string }).id === ids.shellB).length
    replies['offered after the close'] = (
      await through.result<RestoredSession[]>('sessions:restored')
    ).map((o) => o.session.id)
    await call('clear', 'sessions:clear')
    replies['offered after the clear'] =
      await through.result<RestoredSession[]>('sessions:restored')
    replies['listed at the end'] = byName(await listed(through)).map((s) => ({
      id: s.id,
      displayName: s.displayName,
      groupId: s.groupId,
      live: s.pid > 0
    }))
    return { replies, resyncTold, echoed }
  } finally {
    direct.close()
    through.close()
    watcher.close()
  }
}

/** Runs `phase` on `server`; a failure names the phase and that server's log. */
async function phase<T>(name: string, server: RealServer, run: () => Promise<T>): Promise<T> {
  try {
    return await run()
  } catch (err) {
    const log = server.log.join('').slice(-4000)
    throw new Error(`${name}: ${(err as Error).message}\n${log}`, { cause: err })
  }
}

async function scenario(): Promise<Observed> {
  const first = await startRealServer()
  const dirs: RunDirs = first.dirs
  const { ids, replies } = await phase('first run', first, () => firstRun(first))
  await stopRealServer(first, true)
  const second = await startRealServer(dirs)
  let warm: Awaited<ReturnType<typeof warmRun>>
  try {
    warm = await phase('restart with the holder', second, () => warmRun(second, ids))
  } finally {
    await stopRealServer(second)
  }
  const third = await startRealServer(dirs)
  let cold: Awaited<ReturnType<typeof coldRun>>
  try {
    cold = await phase('restart without the holder', third, () => coldRun(third, ids))
  } finally {
    await stopRealServer(third)
  }
  return {
    transcript: normalizeRun({ ...replies, ...warm.replies, ...cold.replies }, dirs),
    resyncTold: cold.resyncTold,
    headlessAfterRestart: warm.headless,
    launchLinesEchoed: cold.echoed
  }
}

describe.skipIf(!runnable)('sessions carried over a restart, against the server', () => {
  let run: Observed

  beforeAll(async () => {
    run = await scenario()
  }, 360_000)

  afterAll(() => removeRealServerDirs())

  it('lists, attaches, offers, resumes and closes them', () => {
    expect(run.transcript).toEqual(recorded('restore', run.transcript))
  })

  it('attaches a carried session through vornd like a fresh one', () => {
    const attach = run.transcript['attach to a carried shell'] as Record<string, unknown>
    expect(attach).toEqual({
      replies: 'vornd',
      live: true,
      sized: true,
      cursor: true,
      showsItsScreen: true
    })
    expect(run.transcript['the agent was started once']).toBe(1)
    expect(run.launchLinesEchoed).toBe(1)
    expect(run.transcript['what the agent was resumed with']).toEqual([
      expect.stringMatching(/^ARGV:--resume <minted \d+>$/)
    ])
  })

  it('tells a pane attached to a cold session to attach again once it runs', () => {
    expect(run.resyncTold).toBe(true)
  })

  it('follows a headless agent across the restart', () => {
    expect(run.headlessAfterRestart).toBe(1)
  })
})
