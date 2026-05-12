import { spawn, ChildProcess } from 'node:child_process'
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
import { getSafeEnv } from './process-utils'
import { buildHeadlessSpawnArgs } from './agent-launch'
import { DEFAULT_AGENT_COMMANDS } from '@vornrun/shared/agent-defaults'
import log from './logger'

const MAX_OUTPUT_LINES = 1000
const FORCE_KILL_DELAY_MS = 5000

/** Force-kill a Windows process tree via taskkill (best-effort). */
function forceKillWin(pid: number): void {
  const child = spawn('taskkill', ['/F', '/T', '/PID', String(pid)], {
    stdio: 'ignore',
    windowsHide: true
  })
  child.on('error', () => {})
  child.unref()
}

class HeadlessManager extends EventEmitter {
  private processes = new Map<string, ChildProcess>()
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

  createHeadless(payload: CreateTerminalPayload): HeadlessSession {
    const id = crypto.randomUUID()
    let effectivePath = payload.projectPath
    let effectiveBranch: string | undefined
    let worktreeName: string | undefined

    if (payload.existingWorktreePath && fs.existsSync(payload.existingWorktreePath)) {
      effectivePath = payload.existingWorktreePath
      worktreeName = payload.worktreeName || extractWorktreeName(payload.existingWorktreePath)
      effectiveBranch = payload.branch
    }
    // Handle worktree creation (or fallback if existing path gone)
    else if ((payload.useWorktree || payload.existingWorktreePath) && payload.branch) {
      if (isGitRepo(payload.projectPath)) {
        const result = createWorktree(payload.projectPath, payload.branch, payload.worktreeName)
        effectivePath = result.worktreePath
        worktreeName = result.name
        effectiveBranch = result.branch
      } else {
        log.warn(`[headless] skipping worktree for non-git project: ${payload.projectPath}`)
        payload.useWorktree = false
      }
    } else if (payload.branch) {
      if (isGitRepo(payload.projectPath)) {
        const currentBranch = getGitBranch(payload.projectPath)
        if (currentBranch !== payload.branch) {
          checkoutBranch(payload.projectPath, payload.branch)
        }
        effectiveBranch = payload.branch
      }
    }

    // Pre-generate the session id before buildHeadlessSpawnArgs so the --session-id
    // flag can be injected; keeps parity with the interactive PTY path.
    let agentSessionId: string | undefined
    if (supportsSessionIdPinning(payload.agentType)) {
      if (payload.resumeSessionId) {
        agentSessionId = payload.resumeSessionId
      } else {
        agentSessionId = crypto.randomUUID()
        payload.sessionId = agentSessionId
      }
    }

    const env = getSafeEnv()
    const spawnArgs = buildHeadlessSpawnArgs(payload, this.agentCommands, env)
    log.info(
      `[headless] launching: ${spawnArgs.command} ${spawnArgs.args.join(' ').slice(0, 100)}...`
    )

    const child = spawn(spawnArgs.command, spawnArgs.args, {
      cwd: effectivePath,
      env,
      stdio: ['pipe', 'pipe', 'pipe'],
      windowsHide: true,
      shell: process.platform === 'win32'
    })

    // Close stdin immediately so the process doesn't hang waiting for input
    child.stdin?.on('error', () => {}) // prevent EPIPE if process exits early
    child.stdin?.end()

    this.processes.set(id, child)
    this.outputBuffers.set(id, [])

    const branch = effectiveBranch || getGitBranch(effectivePath)
    const worktreePath =
      payload.existingWorktreePath ||
      (payload.useWorktree && payload.branch ? effectivePath : undefined)
    const session: HeadlessSession = {
      id,
      pid: child.pid || 0,
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
      ...(agentSessionId ? { agentSessionId } : {})
    }
    this.sessions.set(id, session)

    // Stream stdout
    child.stdout?.on('data', (chunk: Buffer) => {
      const data = chunk.toString()
      this.appendOutput(id, data)
      this.emit('client-message', IPC.HEADLESS_DATA, { id, data })
    })

    // Stream stderr
    child.stderr?.on('data', (chunk: Buffer) => {
      const data = chunk.toString()
      this.appendOutput(id, data)
      this.emit('client-message', IPC.HEADLESS_DATA, { id, data })
    })

    // Handle exit
    child.on('exit', (exitCode) => {
      log.info(`[headless] process ${id} exited with code ${exitCode}`)
      const sess = this.sessions.get(id)
      if (sess && sess.status === 'running') {
        sess.status = 'exited'
        sess.exitCode = exitCode ?? undefined
        sess.endedAt = Date.now()
        this.emit('client-message', IPC.HEADLESS_EXIT, { id, exitCode: exitCode ?? 1 })
      }
      this.processes.delete(id)

      // Clean up output buffer and session after a short delay to allow
      // final reads from the renderer, preventing unbounded memory growth.
      setTimeout(() => {
        this.outputBuffers.delete(id)
        this.sessions.delete(id)
      }, 30_000)
    })

    child.on('error', (err) => {
      log.error(`[headless] process ${id} error:`, err.message)
      this.appendOutput(id, `Error: ${err.message}\n`)
      this.emit('client-message', IPC.HEADLESS_DATA, { id, data: `Error: ${err.message}\n` })

      // Mark session as exited so workflow steps detect the failure
      const sess = this.sessions.get(id)
      if (sess && sess.status === 'running') {
        sess.status = 'exited'
        sess.exitCode = 1
        sess.endedAt = Date.now()
        this.processes.delete(id)
        this.emit('client-message', IPC.HEADLESS_EXIT, { id, exitCode: 1 })
        setTimeout(() => {
          this.outputBuffers.delete(id)
          this.sessions.delete(id)
        }, 30_000)
      }
    })

    return session
  }

  killHeadless(id: string): void {
    const proc = this.processes.get(id)
    if (!proc) return
    if (process.platform === 'win32') {
      proc.kill()
      setTimeout(() => {
        if (this.processes.has(id) && proc.pid) forceKillWin(proc.pid)
      }, FORCE_KILL_DELAY_MS)
    } else {
      proc.kill('SIGTERM')
      setTimeout(() => {
        if (this.processes.has(id)) proc.kill('SIGKILL')
      }, FORCE_KILL_DELAY_MS)
    }
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

  killAll(): void {
    for (const [id, proc] of this.processes) {
      if (process.platform === 'win32') {
        proc.kill()
        if (proc.pid) forceKillWin(proc.pid)
      } else {
        proc.kill('SIGKILL')
      }
      this.processes.delete(id)
    }
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
