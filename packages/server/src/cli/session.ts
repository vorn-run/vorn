import path from 'node:path'
import type {
  AiAgentType,
  AppConfig,
  CreateTerminalPayload,
  HeadlessSession,
  ProjectConfig,
  TerminalSession
} from '@vornrun/shared/types'
import { getRepoRoot } from '../git-utils'
import { normalizePath } from '../process-utils'
import type { ClientContext } from './deps'
import { EXIT_FAILURE, EXIT_OK, EXIT_UNREACHABLE, EXIT_USAGE } from './exit'
import { asJson, paintStatus, shortId, table, timeAgo } from './output'

const AGENTS: AiAgentType[] = ['claude', 'copilot', 'codex', 'opencode', 'gemini']

export const SESSION_USAGE = `Usage
  vorn session start --agent <agent> [--prompt <text>] [options]
  vorn session list [--recent] [--project <name>] [--json]
  vorn session logs <id> [--lines <n>] [--json]
  vorn session send <id> <text> [--raw]
  vorn session kill <id>

Agents
  ${AGENTS.join(', ')}

Options for start
  --prompt <text>     First thing the agent is told
  --project <name>    A project Vorn knows, or a name for this one
  --path <dir>        Project directory (default: the git root of this one)
  --branch <branch>   Branch to check out
  --worktree          Run in a new git worktree
  --headless          No terminal pane; the agent runs to completion
  --name <label>      Display name for the session
`

function usage(ctx: ClientContext, message: string): number {
  ctx.writeErr(`vorn: ${message}\n\n${SESSION_USAGE}`)
  return EXIT_USAGE
}

/** Turn what the server threw into one line a person can act on. */
function failed(ctx: ClientContext, what: string, err: unknown): number {
  ctx.writeErr(`vorn: ${what}: ${err instanceof Error ? err.message : String(err)}\n`)
  return EXIT_FAILURE
}

/**
 * The project a session belongs to, registered if Vorn has not seen it.
 *
 * A directory is identified by its path, never its name: two checkouts of one
 * repository are two projects, and renaming a directory does not create a third.
 */
async function resolveProject(
  ctx: ClientContext,
  agentType: AiAgentType
): Promise<{ projectName: string; projectPath: string }> {
  const config = await ctx.rpc.call('config:load')
  const projects: ProjectConfig[] = config.projects ?? []

  // A project named with no directory beside it is one Vorn already knows, so
  // this works from anywhere rather than only from inside that checkout.
  if (ctx.args.project && !ctx.args.path) {
    const named = projects.find((p) => p.name === ctx.args.project)
    if (named) return { projectName: named.name, projectPath: named.path }
  }

  const cwd = process.cwd()
  const projectPath = ctx.args.path ? path.resolve(ctx.args.path) : (getRepoRoot(cwd) ?? cwd)
  const known = projects.find((p) => normalizePath(p.path) === normalizePath(projectPath))
  if (known) return { projectName: known.name, projectPath }

  const projectName = ctx.args.project ?? path.basename(projectPath)
  const registered: ProjectConfig = {
    name: projectName,
    path: projectPath,
    preferredAgents: [agentType]
  }
  const next: AppConfig = { ...config, projects: [...projects, registered] }
  await ctx.rpc.call('config:save', next)
  return { projectName, projectPath }
}

/** A session and which half of the server owns it, because they are killed differently. */
interface Addressed {
  id: string
  kind: 'terminal' | 'headless'
}

/**
 * The session an id names, accepting any prefix that names only one.
 *
 * Every list prints eight characters, so those eight have to be enough to act
 * on afterwards -- and the list shows headless sessions too, so this has to
 * look in both places or it would print ids nothing here could address.
 */
async function resolveSession(ctx: ClientContext, given: string): Promise<Addressed> {
  const [terminals, headless] = await Promise.all([
    ctx.rpc.call('terminal:listActive'),
    ctx.rpc.call('headless:list')
  ])
  const candidates: Addressed[] = [
    ...terminals.map((s) => ({ id: s.id, kind: 'terminal' as const })),
    ...headless.map((s) => ({ id: s.id, kind: 'headless' as const }))
  ]

  const exact = candidates.find((c) => c.id === given)
  if (exact) return exact

  const matches = candidates.filter((c) => c.id.startsWith(given))
  if (matches.length === 1) return matches[0]
  if (matches.length === 0) throw new Error(`no session matches "${given}"`)
  throw new Error(
    `"${given}" matches ${matches.length} sessions: ${matches.map((c) => shortId(c.id)).join(', ')}`
  )
}

/** A headless run has no terminal behind it, so two of the verbs cannot reach one. */
function noTerminal(ctx: ClientContext, id: string, verb: string): number {
  ctx.writeErr(
    `vorn: ${shortId(id)} is a headless session, which has no terminal to ${verb}. ` +
      `Its output streams to Vorn while it runs; kill it with: vorn session kill ${shortId(id)}\n`
  )
  return EXIT_FAILURE
}

async function startSession(ctx: ClientContext): Promise<number> {
  const agent = ctx.args.agent
  if (!agent) return usage(ctx, 'session start needs --agent')
  if (!AGENTS.includes(agent as AiAgentType)) {
    return usage(ctx, `unknown agent "${agent}". Try: ${AGENTS.join(', ')}`)
  }
  const agentType = agent as AiAgentType

  try {
    const { projectName, projectPath } = await resolveProject(ctx, agentType)
    const payload: CreateTerminalPayload = {
      agentType,
      projectName,
      projectPath,
      ...(ctx.args.prompt ? { initialPrompt: ctx.args.prompt } : {}),
      ...(ctx.args.branch ? { branch: ctx.args.branch } : {}),
      ...(ctx.args.worktree ? { useWorktree: true } : {}),
      ...(ctx.args.name ? { displayName: ctx.args.name } : {})
    }

    const session: TerminalSession | HeadlessSession = ctx.args.headless
      ? await ctx.rpc.call('headless:create', payload)
      : await ctx.rpc.call('terminal:create', payload)

    if (ctx.args.json) {
      ctx.write(asJson(session))
      return EXIT_OK
    }

    const branch = 'branch' in session ? session.branch : undefined
    ctx.write(
      [
        `session  ${session.id}`,
        `agent    ${session.agentType}`,
        `project  ${session.projectName}`,
        `path     ${'worktreePath' in session && session.worktreePath ? session.worktreePath : session.projectPath}`,
        ...(branch ? [`branch   ${branch}`] : []),
        ''
      ].join('\n')
    )
    return EXIT_OK
  } catch (err) {
    return failed(ctx, 'could not start the session', err)
  }
}

/** Headless sessions are sessions; a list that hid them would hide `--headless`. */
async function listSessions(ctx: ClientContext): Promise<number> {
  try {
    if (ctx.args.recent) {
      const recent = await ctx.rpc.call(
        'sessions:getRecent',
        ctx.args.path ? path.resolve(ctx.args.path) : undefined
      )
      if (ctx.args.json) {
        ctx.write(asJson(recent))
        return EXIT_OK
      }
      if (recent.length === 0) {
        ctx.writeErr('No recent sessions.\n')
        return EXIT_OK
      }
      ctx.write(
        table(
          ['ID', 'AGENT', 'PROJECT', 'WHEN', 'ACTIVITY'],
          recent.map((s) => [
            shortId(s.sessionId),
            s.agentType,
            path.basename(s.projectPath),
            timeAgo(s.timestamp),
            s.activityLabel
          ])
        )
      )
      return EXIT_OK
    }

    const [terminals, headless] = await Promise.all([
      ctx.rpc.call('terminal:listActive'),
      ctx.rpc.call('headless:list')
    ])
    const running = headless.filter((s) => s.status === 'running')
    const wanted = ctx.args.project
    const sessions = [...terminals, ...running].filter((s) => !wanted || s.projectName === wanted)

    if (ctx.args.json) {
      ctx.write(asJson(sessions))
      return EXIT_OK
    }
    if (sessions.length === 0) {
      ctx.writeErr('No sessions running.\n')
      return EXIT_OK
    }

    const statusColumn = 4
    ctx.write(
      table(
        ['ID', 'AGENT', 'PROJECT', 'BRANCH', 'STATUS'],
        sessions.map((s) => [shortId(s.id), s.agentType, s.projectName, s.branch ?? '-', s.status]),
        (cell, column) => (column === statusColumn ? paintStatus(cell, ctx.plain) : cell)
      )
    )
    return EXIT_OK
  } catch (err) {
    return failed(ctx, 'could not list sessions', err)
  }
}

async function readLogs(ctx: ClientContext, given: string | undefined): Promise<number> {
  if (!given) return usage(ctx, 'session logs needs a session id')
  try {
    const target = await resolveSession(ctx, given)
    if (target.kind === 'headless') return noTerminal(ctx, target.id, 'read')

    const lines = await ctx.rpc.call('terminal:readOutput', {
      id: target.id,
      lines: ctx.args.lines
    })
    if (ctx.args.json) {
      ctx.write(asJson(lines))
      return EXIT_OK
    }
    // Empty is an answer, and silence reads as a broken command.
    if (lines.length === 0) {
      ctx.writeErr(`Nothing kept for ${shortId(target.id)} yet.\n`)
      return EXIT_OK
    }
    ctx.write(`${lines.join('\n')}\n`)
    return EXIT_OK
  } catch (err) {
    return failed(ctx, 'could not read the session', err)
  }
}

async function sendInput(
  ctx: ClientContext,
  given: string | undefined,
  text: string | undefined
): Promise<number> {
  if (!given || text === undefined) return usage(ctx, 'session send needs a session id and text')
  try {
    const target = await resolveSession(ctx, given)
    if (target.kind === 'headless') return noTerminal(ctx, target.id, 'send to')

    // Enter is what submits a prompt; --raw is for sending control sequences instead.
    const data = ctx.args.raw ? text : `${text.replace(/[\r\n]+$/, '')}\r`
    await ctx.rpc.notify('terminal:write', { id: target.id, data })
    ctx.writeErr(`Sent to ${shortId(target.id)}.\n`)
    return EXIT_OK
  } catch (err) {
    return failed(ctx, 'could not send to the session', err)
  }
}

async function killSession(ctx: ClientContext, given: string | undefined): Promise<number> {
  if (!given) return usage(ctx, 'session kill needs a session id')
  try {
    const target = await resolveSession(ctx, given)
    // Two registries, two kill methods: a headless run is not a pty.
    await ctx.rpc.call(target.kind === 'headless' ? 'headless:kill' : 'terminal:kill', target.id)
    ctx.writeErr(`Killed ${shortId(target.id)}.\n`)
    return EXIT_OK
  } catch (err) {
    return failed(ctx, 'could not kill the session', err)
  }
}

export async function runSessionCommand(ctx: ClientContext): Promise<number> {
  const [, verb, ...rest] = ctx.args.positionals

  if (ctx.args.help) {
    ctx.write(SESSION_USAGE)
    return EXIT_OK
  }
  if (!verb) {
    ctx.writeErr(SESSION_USAGE)
    return EXIT_USAGE
  }
  if (!['start', 'list', 'logs', 'send', 'kill'].includes(verb)) {
    return usage(ctx, `unknown session command "${verb}"`)
  }
  if (!(await ctx.server())) return EXIT_UNREACHABLE

  switch (verb) {
    case 'start':
      return startSession(ctx)
    case 'list':
      return listSessions(ctx)
    case 'logs':
      return readLogs(ctx, rest[0])
    case 'send':
      return sendInput(ctx, rest[0], rest.slice(1).join(' ') || undefined)
    default:
      return killSession(ctx, rest[0])
  }
}
