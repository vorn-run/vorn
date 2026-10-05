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
import { vorndSessions, type VorndPty } from './vornd-sessions'

const MAX_OUTPUT_LINES = 1000
const FORCE_KILL_DELAY_MS = 5000

class HeadlessManager extends EventEmitter {
  /** Agents running in vornd: they outlive this server. */
  private inVornd = new Map<string, VorndPty>()
  private sessions = new Map<string, HeadlessSession>()
  private outputBuffers = new Map<string, string[]>()
  private agentCommands: Record<AiAgentType, AgentCommandConfig> = { ...DEFAULT_AGENT_COMMANDS }

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
    const output = (data: string): void => {
      this.appendOutput(id, data)
      this.emit('client-message', IPC.HEADLESS_DATA, { id, data })
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
    this.inVornd.set(id, agent)
    this.outputBuffers.set(id, [])
    this.sessions.set(id, session)
    agent.on('started', (pid: number) => {
      session.pid = pid
    })
    agent.onData(output)
    agent.onExit(({ exitCode, repeated }) => {
      this.inVornd.delete(id)
      this.exited(id, exitCode, repeated)
    })
    return session
  }

  /**
   * The agent ended. Told once to the windows and the workflow waiting on it;
   * an exit told again after vornd restarted (`repeated`) was told the first
   * time, and running the workflow's next step twice is the one thing a
   * receipt is for.
   */
  private exited(id: string, exitCode: number | undefined, repeated = false): void {
    const sess = this.sessions.get(id)
    if (!sess || sess.status !== 'running') return
    log.info(`[headless] process ${id} exited with code ${exitCode}`)
    sess.status = 'exited'
    sess.exitCode = exitCode
    sess.endedAt = Date.now()
    if (!repeated) {
      this.emit('client-message', IPC.HEADLESS_EXIT, { id, exitCode: exitCode ?? 1 })
    }
    // Clean up output buffer and session after a short delay to allow
    // final reads from the renderer, preventing unbounded memory growth.
    setTimeout(() => {
      this.outputBuffers.delete(id)
      this.sessions.delete(id)
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

  updateSessionsForWorktree(
    worktreePath: string,
    updates: { branch?: string; worktreePath?: string; worktreeName?: string }
  ): void {
    for (const s of this.sessions.values()) {
      if (s.worktreePath === worktreePath) {
        if (updates.branch !== undefined) s.branch = updates.branch
        if (updates.worktreeName !== undefined) s.worktreeName = updates.worktreeName
        if (updates.worktreePath !== undefined) s.worktreePath = updates.worktreePath
        this.emit('client-message', IPC.SESSION_UPDATED, s)
      }
    }
  }

  /** Let go of every agent for a server on its way out. */
  killAll(): void {
    // Left running: an agent in vornd outlives this server.
    for (const id of this.inVornd.keys()) vorndSessions.release(id)
    this.inVornd.clear()
    this.sessions.clear()
    this.outputBuffers.clear()
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
