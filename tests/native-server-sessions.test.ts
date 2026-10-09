/**
 * The terminals and headless agents vornd creates and changes, as the app
 * drives them: shells and agents created, hooks linking them, renames,
 * groups, a reorder, an exit, a resume, a kill and a headless agent, through
 * vornd in front of a real server on a real database. What clients are
 * answered, started and told is checked against a recorded run
 * (`fixtures/vornd/terminals.json`), first made while the server still held
 * the sessions.
 *
 * Runs where vornd and vorn-sessiond have been built (`yarn build:core`, or
 * the binaries in `VORN_CONFORMANCE_VORND`), on a Unix: the agents are shell
 * scripts.
 */
import fs from 'node:fs'
import path from 'node:path'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import type { HeadlessSession, TerminalSession } from '../packages/shared/src/types'
import { spawnsRealServers } from './helpers/one-at-a-time'
import {
  Watcher,
  answered,
  removeRealServerDirs,
  repository,
  runnable,
  startRealServer,
  stopRealServer,
  type Frame,
  type RealServer
} from './helpers/real-server'
import {
  normalizeRun,
  outputWhole,
  withoutHeadlessExits,
  withoutHookLinks
} from './helpers/sessions-parity'
import { recorded } from './helpers/vornd-fixtures'

type Counts = Record<
  string,
  { mode?: string; forwarded?: number; shadowMatched?: number; shadowMismatched?: number }
>

const PATIENCE_MS = 30_000

async function until(what: string, check: () => boolean | Promise<boolean>): Promise<void> {
  const start = Date.now()
  while (!(await check())) {
    if (Date.now() - start > PATIENCE_MS) throw new Error(`timed out waiting for ${what}`)
    await new Promise((r) => setTimeout(r, 50))
  }
}

spawnsRealServers()

const AGENTS = ['claude', 'codex', 'copilot', 'gemini', 'opencode'] as const

/**
 * A stub agent: it says what it was started with, and waits. Headless, on
 * pipes, it reads its prompt and says it, waits if told to, and ends with 3.
 */
const ARGV_AGENT = `#!/bin/sh
printf 'ARGV:%s\\n' "$*"
if [ -t 0 ]; then exec sleep 600; fi
p=$(cat)
printf 'PROMPT:%s\\n' "$p"
case "$p" in *wait*) sleep 600;; esac
exit 3
`

/** The same calls, on one server, through its vornd; answers the transcript. */
async function scenario(server: RealServer): Promise<Record<string, unknown>> {
  const { work } = server.dirs
  const stub = path.join(work, 'bin', 'argv-agent')
  fs.mkdirSync(path.dirname(stub))
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
  const created = async (step: string, method: string, params?: unknown): Promise<string> => {
    const frame = await call(step, method, params)
    if (frame.error) throw new Error(`${step}: ${JSON.stringify(frame.error)}`)
    return (frame.result as TerminalSession).id
  }
  const listed = (): Promise<TerminalSession[]> =>
    through.result<TerminalSession[]>('terminal:listActive')
  const live = async (ids: string[]): Promise<void> => {
    await until('the sessions to start', async () => {
      const all = await listed()
      return ids.every((id) => (all.find((s) => s.id === id)?.pid ?? 0) > 0)
    })
  }
  try {
    const config = await direct.result<Record<string, unknown>>('config:load')
    await direct.result('config:save', {
      ...config,
      defaults: { ...(config.defaults as object), shell: '/bin/sh' },
      agentCommands: Object.fromEntries(AGENTS.map((a) => [a, { command: stub, args: [] }]))
    })

    // An agent of every kind, each in a project of its own.
    const agents: Record<string, string> = {}
    for (const agent of AGENTS) {
      const project = path.join(work, agent)
      fs.mkdirSync(project)
      agents[agent] = await created(`create ${agent}`, 'terminal:create', {
        agentType: agent,
        projectName: agent,
        projectPath: project,
        displayName: agent === 'gemini' ? 'Named' : undefined,
        initialPrompt: agent === 'opencode' ? 'write the parity test' : undefined
      })
    }
    await live(Object.values(agents))
    const argv: Record<string, string> = {}
    const shown: Record<string, string[]> = {}
    await until('every agent to say what it was started with', async () => {
      for (const agent of AGENTS) {
        const out = await through.result<string[]>('terminal:readOutput', { id: agents[agent] })
        shown[agent] = out
        // After the shell's prompt, when the agent was started before the shell drew it.
        const line = out.find((l) => l.includes('ARGV:'))
        if (!line) return false
        argv[agent] = line.slice(line.indexOf('ARGV:')).trimEnd()
      }
      return true
    }).catch((err: Error) => {
      throw new Error(`${err.message}; their screens: ${JSON.stringify(shown)}`)
    })

    // Shells: in a project, and in the home directory.
    const shellA = await created('shell in a project', 'shell:create', path.join(work, 'claude'))
    await live([shellA])
    const shellB = await created('shell at home', 'shell:create')
    await live([shellB])

    // An agent in a new worktree, which is the only session there.
    const inWorktree = await created('create in a worktree', 'terminal:create', {
      agentType: 'claude',
      projectName: 'repo',
      projectPath: repo,
      useWorktree: true,
      branch: 'feature',
      worktreeName: 'wt-one'
    })
    await live([inWorktree])

    // The worktree's branch renamed and the worktree moved, and a rename refused.
    const worktreeOf = async (): Promise<string> =>
      (await listed()).find((s) => s.id === inWorktree)!.worktreePath!
    const renamed = { worktreePath: await worktreeOf(), newBranch: 'feature-two' }
    await call('rename the worktree branch', 'git:renameWorktreeBranch', renamed)
    await call('rename it to a branch that is taken', 'git:renameWorktreeBranch', {
      worktreePath: renamed.worktreePath,
      newBranch: 'main'
    })
    await call('rename the worktree', 'git:renameWorktree', {
      worktreePath: renamed.worktreePath,
      newName: 'wt two'
    })
    await call('rename a worktree that moved', 'git:renameWorktree', {
      worktreePath: renamed.worktreePath,
      newName: 'wt three'
    })
    await until(
      'the moved worktree to be listed',
      async () => (await worktreeOf()) !== renamed.worktreePath
    )
    // One conversation asked for twice at once: one session, both answered with it.
    const named = {
      agentType: 'codex',
      projectName: 'codex',
      projectPath: path.join(work, 'codex'),
      resumeSessionId: 'conversation-twice'
    }
    const [first, second] = await Promise.all([
      through.call('terminal:create', named),
      through.call('terminal:create', named)
    ])
    const once = (first.result as TerminalSession).id
    replies['one conversation twice at once'] = {
      same: once === (second.result as TerminalSession).id
    }
    await live([once])
    const third = await through.result<TerminalSession>('terminal:create', named)
    replies['one conversation again'] = { same: third.id === once }

    // What a person does to the cards, and what is refused.
    await call('rename', 'terminal:rename', { id: shellA, displayName: 'Build' })
    await call('group', 'terminal:setGroup', { id: shellB, groupId: 'group-1' })
    const order = [shellB, ...Object.values(agents), inWorktree, once, shellA]
    await call('reorder', 'terminal:reorder', order)
    await call('ungroup', 'terminal:setGroup', { id: shellB, groupId: '' })
    await call('rename a card that is not there', 'terminal:rename', {
      id: 'no-such-card',
      displayName: 'x'
    })
    await call('reorder twice over', 'terminal:reorder', [shellA, shellA])
    await call('reorder with one missing', 'terminal:reorder', [shellA, 'no-such-card'])

    // The last session in the worktree closed: one offer to clean it up.
    // Each close waits for its exit, as the server tells it and as a client
    // through vornd hears it (from whoever tells it, so in an order of its own).
    const exited = async (id: string): Promise<void> => {
      const told = (w: Watcher): boolean =>
        w.toldBy('terminal:exit').some((p) => (p as { id?: string }).id === id)
      await until(`the exit of ${id}`, () => told(direct) && told(through))
    }
    await call('close the worktree agent', 'terminal:kill', inWorktree)
    await exited(inWorktree)
    await call('close a shell', 'terminal:kill', shellB)
    await until('the shell to go', async () => !(await listed()).some((s) => s.id === shellB))
    await exited(shellB)
    // A card that is not there: the server tells its exit anyway.
    await call('close a card that is not there', 'terminal:kill', 'no-such-card')
    await exited('no-such-card')

    // A shell opened again after the closes is numbered after the ones left.
    const shellC = await created('shell after a close', 'shell:create', path.join(work, 'claude'))
    await live([shellC])

    // A headless agent of every kind, each run to its end, and one stopped.
    const headless: Record<string, string> = {}
    for (const agent of AGENTS) {
      headless[agent] = await created(`headless ${agent}`, 'headless:create', {
        agentType: agent,
        projectName: agent,
        projectPath: path.join(work, agent),
        initialPrompt: `print the ${agent} parity\nline two`,
        workflowId: 'wf-1',
        workflowName: 'Parity'
      })
    }
    const stopped = await created('headless to stop', 'headless:create', {
      agentType: 'claude',
      projectName: 'claude',
      projectPath: path.join(work, 'claude'),
      displayName: 'Waits',
      initialPrompt: 'wait here'
    })
    const headlessEnded = async (ids: string[]): Promise<void> => {
      await until('the headless agents to end', async () => {
        const all = await through.result<HeadlessSession[]>('headless:list')
        const exits = direct.toldBy('headless:exit') as { id: string }[]
        return ids.every(
          (id) =>
            all.find((s) => s.id === id)?.status === 'exited' && exits.some((e) => e.id === id)
        )
      })
    }
    await headlessEnded(Object.values(headless))
    // Stopped once it has said its prompt: a stop can reach an agent before it prints.
    await until('the waiting agent to say its prompt', () =>
      direct
        .toldBy('headless:data')
        .some(
          (p) =>
            (p as { id: string; data: string }).id === stopped &&
            /wait here/.test((p as { data: string }).data)
        )
    )
    await call('stop a headless agent', 'headless:kill', stopped)
    await headlessEnded([stopped])
    await call('stop one that ended', 'headless:kill', headless.claude)
    await call('stop one that is not there', 'headless:kill', 'no-such-agent')
    const agentsListed = await through.result<HeadlessSession[]>('headless:list')
    const byAgent = { ...headless, stopped }

    // Settled: the registry stops changing.
    let last = ''
    let since = Date.now()
    await until('the registry to settle', async () => {
      const now = JSON.stringify(await listed())
      if (now !== last) {
        last = now
        since = Date.now()
      }
      return Date.now() - since > 500
    })
    const toldOf = (method: string): unknown[] => direct.toldBy(method)
    const health = await fetch(`http://127.0.0.1:${server.vornd}/vornd/health`)
    const groups = (
      (await health.json()) as { groups: Counts & Record<string, { native?: number }> }
    ).groups
    const by = (group: string): { native: number; forwarded: number } => ({
      native: groups[group]?.native ?? 0,
      forwarded: groups[group]?.forwarded ?? 0
    })
    const exits = toldOf('headless:exit') as { id: string; exitCode: number }[]
    const headlessIds = Object.values(byAgent)
    const exitsTold = withoutHeadlessExits(
      toldOf('terminal:exit').map((p) => (p as { id: string }).id),
      headlessIds
    )
    const heardThrough = withoutHeadlessExits(
      through.toldBy('terminal:exit').map((p) => (p as { id: string }).id),
      headlessIds
    )
    return {
      answeredBy: {
        terminal: by('terminal'),
        shell: by('shell'),
        headless: by('headless'),
        git: by('git')
      },
      replies: withoutHookLinks(replies),
      argv,
      listed: await listed(),
      agentsListed: Object.fromEntries(
        Object.entries(byAgent).map(([name, id]) => [name, agentsListed.find((s) => s.id === id)])
      ),
      agentsOutput: outputWhole(toldOf('headless:data') as { id: string; data: string }[], byAgent),
      agentsExits: Object.fromEntries(
        Object.entries(byAgent).map(([name, id]) => [name, exits.find((e) => e.id === id)])
      ),
      told: {
        created: toldOf('session:created'),
        reordered: toldOf('session:reordered'),
        cleanup: toldOf('worktree:confirmCleanup'),
        // In the order they were closed: each waited for the one before.
        exits: exitsTold,
        // As a client through vornd hears them: once each, whoever tells it,
        // in an order of its own, so listed in the server's.
        exitsThrough: {
          once: new Set(heardThrough).size === heardThrough.length,
          ids: exitsTold.filter((id) => heardThrough.includes(id)),
          unheard: exitsTold.filter((id) => !heardThrough.includes(id))
        },
        renamed: toldOf('session:updated')
          .map((p) => p as TerminalSession)
          .filter((s) => s.displayName === 'Build' || s.groupId === 'group-1')
          .map((s) => ({ id: s.id, displayName: s.displayName, groupId: s.groupId })),
        moved: changesOf(
          [...toldOf('session:created'), ...toldOf('session:updated')]
            .map((p) => p as TerminalSession)
            .filter((s) => s.id === inWorktree)
            .map((s) => JSON.stringify([s.branch, s.worktreePath, s.worktreeName]))
        )
      }
    }
  } finally {
    direct.close()
    through.close()
  }
}

/** Each change in a session's told values, from how it was created: status updates that change none of them are told at whatever moment they happen. */
function changesOf(told: string[]): string[] {
  return told.filter((value, i) => i > 0 && value !== told[i - 1])
}

describe.skipIf(!runnable)('the terminals vornd creates and changes, against the server', () => {
  let run: Record<string, unknown>

  beforeAll(async () => {
    const server = await startRealServer()
    try {
      run = normalizeRun(await scenario(server), server.dirs)
    } catch (err) {
      throw new Error(`${(err as Error).message}\n${server.log.join('').slice(-4000)}`, {
        cause: err
      })
    } finally {
      await stopRealServer(server)
    }
  }, 240_000)

  afterAll(() => removeRealServerDirs())

  it('creates, starts and changes terminals', () => {
    const seen = run as {
      replies: Record<string, unknown>
      told: { cleanup: unknown[]; moved: string[] }
      agentsExits: Record<string, { exitCode: number }>
    }
    expect(seen.replies['one conversation twice at once']).toEqual({ same: true })
    expect(seen.replies['one conversation again']).toEqual({ same: true })
    expect(seen.told.cleanup).toHaveLength(1)
    expect(JSON.parse(seen.told.moved.at(-1)!)).toEqual([
      'feature-two',
      expect.stringMatching(/\/wt-two-<id>$/),
      'wt-two'
    ])
    expect(Object.values(seen.agentsExits).map((e) => e.exitCode)).toEqual([3, 3, 3, 3, 3, 143])
  })

  it('has vornd answer every one of them', () => {
    // Refusals included; the reads are polled, so only that none was forwarded counts.
    const answeredBy = run.answeredBy as Record<string, { native: number; forwarded: number }>
    expect(Object.keys(answeredBy).sort()).toEqual(['git', 'headless', 'shell', 'terminal'])
    for (const [group, counts] of Object.entries(answeredBy)) {
      expect({ group, forwarded: counts.forwarded }).toEqual({ group, forwarded: 0 })
      expect(counts.native).toBeGreaterThan(0)
    }
  })

  it('answers, starts, tells and lists as the app expects', () => {
    const parts = [
      'replies',
      'argv',
      'told',
      'listed',
      'agentsListed',
      'agentsOutput',
      'agentsExits'
    ]
    const seen = Object.fromEntries(parts.map((part) => [part, run[part]]))
    expect(seen).toEqual(recorded('terminals', seen))
  })
})
