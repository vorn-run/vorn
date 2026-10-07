import crypto from 'node:crypto'
import fs from 'node:fs'
import { EventEmitter } from 'node:events'
import {
  AiAgentType,
  AgentCommandConfig,
  CreateTerminalPayload,
  HeadlessSession,
  IPC,
  supportsSessionIdPinning
} from '@vornrun/shared/types'
import { displayNameFromPrompt } from '@vornrun/shared/string-utils'
import {
  getGitBranch,
  checkoutBranch,
  createWorktree,
  extractWorktreeName,
  isGitRepo
} from './git-utils'
import { getLaunchEnv, shellEscape } from './process-utils'
import { buildHeadlessSpawnArgs } from './agent-launch'
import { DEFAULT_AGENT_COMMANDS } from '@vornrun/shared/agent-defaults'
import log from './logger'
import { holdWorkspace } from './workspace-holds'
import { isDraining, DRAINING_MESSAGE } from './draining'
import { vorndSessions, type HeldSession, type SessionNote, type VorndPty } from './vornd-sessions'
import { applyWorktreeUpdates, type WorktreeUpdates } from './worktree-moves'
import { sessionFeed } from './session-feed'

const MAX_OUTPUT_LINES = 1000
const FORCE_KILL_DELAY_MS = 5000

/**
 * The headless agents: started on pipes in vornd's session holder, their
 * output read here for the clients and the workflow waiting on each.
 *
 * With the Native server switch on (`vorndSessions.createsHeadless`), vornd
 * answers the clients' `headless:create` and `headless:kill` itself and tells
 * this server the record through its copy of the registry (`fromVornd`); the
 * agent is then followed here as one this server started. How an agent ended
 * is read from its session by vornd and mirrored here, and told once, after
 * the last of its output, as every exit is.
 */
class HeadlessManager extends EventEmitter {
  /** Agents running in vornd: they outlive this server. */
  private inVornd = new Map<string, VorndPty>()
  private sessions = new Map<string, HeadlessSession>()
  private outputBuffers = new Map<string, string[]>()
  /** Agents whose exit has been acted on, until their records go. */
  private ended = new Set<string>()
  private agentCommands: Record<AiAgentType, AgentCommandConfig> = { ...DEFAULT_AGENT_COMMANDS }

  constructor() {
    super()
    sessionFeed.setHeadlessSource(() => this.getActiveSessions())
    vorndSessions.on('native', (note: SessionNote) => this.fromVornd(note))
  }

  /** Tell vornd's copy of the registry what this record is now (`session-feed`). */
  private recordChanged(id: string): void {
    const session = this.sessions.get(id)
    if (session) sessionFeed.headless(session, session.status === 'exited')
  }

  /**
   * A change vornd made itself, for a client's call: an agent it started,
   * whose program is up or could not start, and how one ended. Followed as
   * this server's own starts are, so clients and workflows are told alike.
   */
  private fromVornd(note: SessionNote): void {
    if (note.kind !== 'headless' || note.op !== 'upsert' || !note.record) return
    if (!vorndSessions.createsHeadless()) return
    const record = note.record as HeadlessSession
    // One the holder still held from the last run is taken on with its
    // states, from the subscription that lists it (`adoptHeld`).
    if (note.created && !note.adopted) this.adoptCreated(record)
    const agent = this.inVornd.get(record.id)
    if (note.started) agent?.started(note.started.pid, note.started.epoch)
    if (note.failed !== undefined) {
      log.warn({ id: record.id, why: note.failed }, '[headless] vornd could not start this agent')
      agent?.finish(1)
    }
    const moved = note.moved ? this.sessions.get(record.id) : undefined
    if (moved) this.worktreeMoved(moved, record)
    if (record.status === 'exited') this.endedInVornd(record)
  }

  /**
   * An agent vornd started for a client, taken on as `createHeadless` takes on
   * one this server starts: its output read, its record held, and told as created.
   */
  private adoptCreated(record: HeadlessSession): void {
    if (this.sessions.has(record.id)) return
    // vornd's revision and stamps are its copy's, not this server's record.
    const session: HeadlessSession & { rev?: unknown; exitAt?: unknown } = { ...record }
    delete session.rev
    delete session.exitAt
    log.info(`[headless] vornd launched ${session.id}: ${session.launchCommand ?? ''}`)
    this.follow(session, vorndSessions.follow(session.id, true))
    this.emit('session-created', session)
  }

  /**
   * An agent vornd still holds from this server's previous run, under the
   * record vornd's copy carried: followed again from the start of its output,
   * with its states told again. Not announced: it was created in that run.
   */
  adoptHeld(record: HeadlessSession, held: HeldSession): void {
    if (this.sessions.has(record.id)) return
    const session: HeadlessSession = { ...record, pid: held.pid }
    log.info(`[headless] following ${session.id} again, held by vornd from the last run`)
    vorndSessions.adopt(held, true, (agent) => this.follow(session, agent))
  }

  /**
   * vornd read the agent's exit from its session: the record takes it as vornd
   * told it. The exit itself is told when the last of the output has been read.
   */
  private endedInVornd(record: HeadlessSession): void {
    const session = this.sessions.get(record.id)
    if (!session || session.status !== 'running') return
    session.status = 'exited'
    session.exitCode = record.exitCode
    session.endedAt = record.endedAt
    this.recordChanged(record.id)
  }

  setAgentCommands(overrides?: Partial<Record<AiAgentType, AgentCommandConfig>>): void {
    this.agentCommands = { ...DEFAULT_AGENT_COMMANDS }
    if (overrides) {
      for (const [key, val] of Object.entries(overrides)) {
        if (val) {
          this.agentCommands[key as AiAgentType] = val
        }
      }
    }
  }

  async createHeadless(payload: CreateTerminalPayload): Promise<HeadlessSession> {
    // Refused rather than created: a session started on an endpoint this process
    // no longer holds is reachable through a name that now points elsewhere, so
    // nobody would ever see it. Existing sessions are untouched -- their clients
    // hold a descriptor, not a name.
    if (isDraining()) throw new Error(DRAINING_MESSAGE)
    const id = crypto.randomUUID()
    // Pinned before the arguments are built, so a fresh launch carries the id it will resume by.
    let agentSessionId = payload.agentType === 'codex' ? payload.resumeSessionId : undefined
    if (supportsSessionIdPinning(payload.agentType)) {
      if (payload.resumeSessionId) {
        agentSessionId = payload.resumeSessionId
      } else {
        agentSessionId = crypto.randomUUID()
        payload.sessionId = agentSessionId
      }
    }
    const env = getLaunchEnv()
    // Built before a worktree exists, so arguments that cannot be built create nothing.
    const spawnArgs = buildHeadlessSpawnArgs(payload, this.agentCommands, env)
    let effectivePath = payload.projectPath
    let effectiveBranch: string | undefined
    let worktreeName: string | undefined
    let branch: string | undefined
    // Held while the git below runs, so a worktree action in between sees it in
    // use: the worktree it names, and one it creates.
    const releases: (() => void)[] = []
    const hold = (dir: string): void => {
      releases.push(holdWorkspace(dir))
    }
    if (payload.existingWorktreePath) hold(payload.existingWorktreePath)
    try {
      if (payload.existingWorktreePath && fs.existsSync(payload.existingWorktreePath)) {
        effectivePath = payload.existingWorktreePath
        worktreeName = payload.worktreeName || extractWorktreeName(payload.existingWorktreePath)
        effectiveBranch = payload.branch
      }
      // Handle worktree creation (or fallback if existing path gone)
      else if ((payload.useWorktree || payload.existingWorktreePath) && payload.branch) {
        if (await isGitRepo(payload.projectPath)) {
          const result = await createWorktree(
            payload.projectPath,
            payload.branch,
            payload.worktreeName,
            undefined,
            hold
          )
          effectivePath = result.worktreePath
          worktreeName = result.name
          effectiveBranch = result.branch
        } else {
          log.warn(`[headless] skipping worktree for non-git project: ${payload.projectPath}`)
          payload.useWorktree = false
        }
      } else if (payload.branch) {
        if (await isGitRepo(payload.projectPath)) {
          const currentBranch = await getGitBranch(payload.projectPath)
          if (currentBranch !== payload.branch) {
            await checkoutBranch(payload.projectPath, payload.branch)
          }
          effectiveBranch = payload.branch
        }
      }
      // getGitBranch answers null for a detached head or a non-repo; the session
      // field is optional rather than nullable, so it is normalised here instead of
      // widening the type everything else reads.
      branch = effectiveBranch || (await getGitBranch(effectivePath)) || undefined
    } finally {
      // From here to the session being registered is synchronous.
      releases.forEach((release) => release())
    }
    // Again, after the git above, which with native git lets the loop run.
    if (isDraining()) throw new Error(DRAINING_MESSAGE)

    // Windows needs `shell: true` to run the `.cmd`/`.ps1` shims that
    // npm-installed agents ship as. Under `shell: true`, Node concatenates argv
    // into a single cmd.exe command line WITHOUT quoting, so a multi-word prompt
    // passed as an argument (copilot/codex/gemini/opencode) gets word-split —
    // the agent's `-p` then sees only the first token. Quote each arg so it
    // survives as one argument. On POSIX we spawn without a shell, so args reach
    // execve verbatim and must NOT be quoted. (claude sidesteps this entirely by
    // taking its prompt on stdin — see buildHeadlessSpawnArgs.)
    // `shell: true` on Windows always runs `comspec || cmd.exe`, so quote with
    // the 'cmd' flavor rather than the user's default shell — otherwise a
    // PowerShell default (or unset COMSPEC) would single-quote args that cmd.exe
    // doesn't treat as quoting, re-introducing the word-split.
    const useShell = process.platform === 'win32'
    const spawnArgList = useShell
      ? spawnArgs.args.map((a) => shellEscape(a, 'cmd'))
      : spawnArgs.args
    // Not truncated: this line is the first thing anyone reads when a session
    // produces no output, and the flag that explains it is as likely to be at
    // the end as the start. The prompt isn't here — it goes to stdin.
    const command = useShell ? shellEscape(spawnArgs.command, 'cmd') : spawnArgs.command
    const launchCommand = [command, ...spawnArgList].join(' ')
    log.info(
      `[headless] launching in ${effectivePath}: ${launchCommand}` +
        (spawnArgs.stdin != null ? ` (prompt on stdin, ${spawnArgs.stdin.length} chars)` : '')
    )

    const worktreePath =
      payload.existingWorktreePath ||
      (payload.useWorktree && payload.branch ? effectivePath : undefined)
    const session: HeadlessSession = {
      id,
      pid: 0,
      agentType: payload.agentType,
      projectName: payload.projectName,
      projectPath: payload.projectPath,
      displayName:
        payload.displayName ||
        (payload.initialPrompt ? displayNameFromPrompt(payload.initialPrompt) : undefined),
      branch,
      worktreePath,
      worktreeName,
      isWorktree: !!worktreePath,
      status: 'running',
      startedAt: Date.now(),
      ...(payload.workflowId != null && { workflowId: payload.workflowId }),
      ...(payload.workflowName != null && { workflowName: payload.workflowName }),
      ...(agentSessionId ? { agentSessionId } : {}),
      launchCommand
    }
    // On pipes in vornd's session holder, not a terminal: some agents behave
    // differently on a TTY, and the prompt goes in on stdin, which then closes.
    // What `shell: true` would run on Windows, spelled out: vornd takes an argv.
    const argv = useShell
      ? [process.env.ComSpec || 'cmd.exe', '/d', '/s', '/c', `"${launchCommand}"`]
      : [command, ...spawnArgList]
    const agent = vorndSessions.spawn(id, { argv, cwd: effectivePath, env, piped: true }, true)
    if (spawnArgs.stdin != null) agent.write(spawnArgs.stdin)
    agent.closeStdin()
    this.follow(session, agent)
    return session
  }

  /** Keep `session`'s record and read its program's output and exit from `agent`. */
  private follow(session: HeadlessSession, agent: VorndPty): void {
    const id = session.id
    this.inVornd.set(id, agent)
    this.outputBuffers.set(id, [])
    this.sessions.set(id, session)
    this.recordChanged(id)
    agent.on('started', (pid: number) => {
      session.pid = pid
      this.recordChanged(id)
    })
    agent.onData((data: string) => {
      this.appendOutput(id, data)
      this.emit('client-message', IPC.HEADLESS_DATA, { id, data })
    })
    agent.onExit(({ exitCode, repeated }) => {
      this.inVornd.delete(id)
      sessionFeed.exitAt('headless', id, agent.exitAt ?? null)
      this.exited(id, exitCode, repeated)
    })
  }

  /**
   * The agent ended. Told once to the windows and the workflow waiting on it;
   * an exit told again after vornd restarted (`repeated`) was told the first
   * time, and running the workflow's next step twice is the one thing a
   * receipt is for. How it ended is what vornd's copy says of it, when vornd
   * read the exit itself (`endedInVornd`); else what the exit said.
   */
  private exited(id: string, exitCode: number | undefined, repeated = false): void {
    const sess = this.sessions.get(id)
    if (!sess || this.ended.has(id)) return
    this.ended.add(id)
    log.info(`[headless] process ${id} exited with code ${exitCode}`)
    if (sess.status === 'running') {
      const copy = vorndSessions.createsHeadless()
        ? vorndSessions.mirror.headlessRecord(id)
        : undefined
      sess.status = 'exited'
      sess.exitCode = copy?.status === 'exited' ? copy.exitCode : exitCode
      sess.endedAt = copy?.status === 'exited' ? copy.endedAt : Date.now()
      this.recordChanged(id)
    }
    if (!repeated) {
      this.emit('client-message', IPC.HEADLESS_EXIT, { id, exitCode: sess.exitCode ?? 1 })
    }
    // Clean up output buffer and session after a short delay to allow
    // final reads from the renderer, preventing unbounded memory growth.
    setTimeout(() => {
      this.outputBuffers.delete(id)
      this.sessions.delete(id)
      this.ended.delete(id)
      sessionFeed.remove('headless', id)
    }, 30_000)
  }

  killHeadless(id: string): void {
    const agent = this.inVornd.get(id)
    if (!agent) return
    agent.kill('SIGTERM')
    setTimeout(() => {
      if (!agent.isEnded) agent.kill('SIGKILL')
    }, FORCE_KILL_DELAY_MS)
  }

  getOutput(id: string): string[] {
    return this.outputBuffers.get(id) || []
  }

  getActiveSessions(): HeadlessSession[] {
    return Array.from(this.sessions.values())
  }

  getActiveSessionsForWorktree(
    worktreePath: string,
    excludeId?: string
  ): { count: number; sessionIds: string[] } {
    const sessionIds: string[] = []
    for (const s of this.sessions.values()) {
      if (s.worktreePath === worktreePath && s.status === 'running' && s.id !== excludeId) {
        sessionIds.push(s.id)
      }
    }
    return { count: sessionIds.length, sessionIds }
  }

  updateSessionsForWorktree(worktreePath: string, updates: WorktreeUpdates): void {
    for (const s of this.sessions.values()) {
      if (s.worktreePath === worktreePath) this.worktreeMoved(s, updates)
    }
  }

  private worktreeMoved(s: HeadlessSession, updates: WorktreeUpdates): void {
    applyWorktreeUpdates(s, updates)
    this.recordChanged(s.id)
    this.emit('client-message', IPC.SESSION_UPDATED, s)
  }

  /** Let go of every agent for a server on its way out. */
  killAll(): void {
    // Left running: an agent in vornd outlives this server.
    for (const id of this.inVornd.keys()) vorndSessions.release(id)
    this.inVornd.clear()
    for (const id of this.sessions.keys()) sessionFeed.remove('headless', id)
    this.sessions.clear()
    this.outputBuffers.clear()
    this.ended.clear()
  }

  private appendOutput(id: string, data: string): void {
    const buf = this.outputBuffers.get(id)
    if (!buf) return
    const lines = data.split('\n')
    buf.push(...lines)
    // Trim to ring buffer limit
    if (buf.length > MAX_OUTPUT_LINES) {
      buf.splice(0, buf.length - MAX_OUTPUT_LINES)
    }
  }
}

export const headlessManager = new HeadlessManager()
