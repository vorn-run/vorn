import * as pty from 'node-pty'
import crypto from 'node:crypto'
import os from 'node:os'
import fs from 'node:fs'
import path from 'node:path'
import { EventEmitter } from 'node:events'
import { holdWorkspace } from './workspace-holds'
import { HeadRefresh } from './head-commit'
import log from './logger'
import {
  AiAgentType,
  AgentStatus,
  AgentCommandConfig,
  CreateTerminalPayload,
  IPC,
  TerminalSession,
  RecordCursor,
  RemoteHost,
  supportsSessionIdPinning,
  supportsExactSessionResume
} from '@vornrun/shared/types'
import { displayNameFromPrompt } from '@vornrun/shared/string-utils'
import {
  getGitBranch,
  getGitHead,
  checkoutBranch,
  createWorktree,
  extractWorktreeName,
  isGitRepo
} from './git-utils'
import { DEFAULT_AGENT_COMMANDS } from '@vornrun/shared/agent-defaults'
import { buildAgentLaunchLine as buildLaunchLine } from './agent-launch'
import {
  shellEscape,
  getSafeEnv,
  getLaunchEnv,
  getDefaultShell,
  getShellArgs,
  normalizePath
} from './process-utils'

import { getShellIntegration } from './shell-integration'
import { configManager } from './config-manager'
import { nativeCore, NATIVE_STATUS, type NativeAnalyzer } from './native-core'
import { appendScrollback, clearScrollback } from './terminal-scrollback'
import { holdOutput, takeOutput, MAX_FLUSH_UNITS, type HeldOutput } from './output-buffer'
import {
  createScreen,
  hasScreen,
  feedScreen,
  resizeScreen,
  clearScreen,
  setCwdReporter,
  setBellReporter
} from './terminal-screen'
import type { ManagedPty } from './handoff/adopted-pty'
import type { AdoptedPane } from './handoff/heir'
import type { DonorPane } from './handoff/donor'
import {
  startHistory,
  recordOutput,
  recordResize,
  noteOutput,
  noteResize,
  pipelineLost,
  stopHistory
} from './history/writer'
import { pipelineFor } from './core-pipeline'
import type { RecordHeader } from './history/log'
import { isDraining, DRAINING_MESSAGE } from './draining'
import { isHandingOver, HANDOVER_MESSAGE } from './handoff/donor'

/**
 * What a PTY starts at, before any client has fitted itself to a pane.
 *
 * Named rather than repeated at each spawn site: the session record now carries
 * these too, and a literal in one place and a constant in another is how the two
 * come to disagree about what the program is rendering against.
 */
/** The largest geometry a resize may ask for. See `resizePty`. */
const MAX_GEOMETRY = 10_000

const INITIAL_COLS = 80
const INITIAL_ROWS = 24
const IDLE_TIMEOUT_MS = 5000
const IDLE_TIMEOUT_HOOKS_MS = 30_000

/** What `prepareSession` worked out, for `spawnPty` to use without doing any of it again. */
export type PreparedSession = { remoteHost: RemoteHost } | { local: PreparedLocal }

interface PreparedLocal {
  agentSessionId?: string
  launchLine: string
  /** Where the shell starts: the worktree when there is one, else the project. */
  effectivePath: string
  worktreePath?: string
  worktreeName?: string
  branch: string | null
  headCommit: string | null
  /** Lets go of the worktree held while preparing; `spawnPty` calls it. */
  release?: () => void
}

/**
 * Refused rather than created: a session started on an endpoint this process
 * no longer holds is reachable through a name that now points elsewhere, so
 * nobody would ever see it. Existing sessions are untouched -- their clients
 * hold a descriptor, not a name.
 */
function refuseWhileClosing(): void {
  if (isDraining()) throw new Error(DRAINING_MESSAGE)
  // A pane created now would be in neither the manifest nor the replacement.
  if (isHandingOver()) throw new Error(HANDOVER_MESSAGE)
}

type WorktreeSessionCounter = (
  worktreePath: string,
  excludeId?: string
) => { count: number; sessionIds: string[] }

/**
 * node-pty exposes `fd` as a getter its typings never declared, so this asks past
 * the type. Both kinds of pty answer it, because a handoff must work twice.
 */
function masterFd(held: ManagedPty): number | null {
  const fd = (held as unknown as { fd?: unknown }).fd
  return typeof fd === 'number' && Number.isInteger(fd) && fd >= 0 ? fd : null
}

class PtyManager extends EventEmitter {
  /** Recorded HEAD per session, refreshed by the save loop. */
  readonly heads = new HeadRefresh(getGitHead)
  private ptys = new Map<string, ManagedPty>()
  private sessions = new Map<string, TerminalSession>()
  /**
   * PTYs an extension's pane is drawing, which are not sessions.
   *
   * They need everything a session's PTY needs — bytes in, bytes out, a resize,
   * a kill — so they live in the same maps. What they are not is work a person
   * started: no window is told about them, nothing persists them, and nothing
   * that walks the sessions of a project should find one and start settling
   * extensions onto it.
   */
  private extensionPtys = new Set<string>()
  private normalizedPaths = new Map<string, string>()
  private agentCommands: Record<AiAgentType, AgentCommandConfig> = { ...DEFAULT_AGENT_COMMANDS }
  private remoteHosts: RemoteHost[] = []
  private dataBuffers = new Map<string, HeldOutput>()
  private flushTimers = new Map<string, ReturnType<typeof setTimeout>>()
  private tempKeyPaths = new Map<string, string>()
  /**
   * Each session's output analysis on the Rust core: agent status, and the
   * output lines agents read back. Created on its first output.
   */
  private analyzers = new Map<string, NativeAnalyzer>()
  /** Sessions with no analysis: the core is missing, or its analyzer failed for them. */
  private unanalyzed = new Set<string>()
  /** Raw chunks since the last native analysis, joined (a rope, so O(1) per chunk). */
  private pendingAnalysis = new Map<string, HeldOutput>()
  /**
   * Where in a pending batch the last read that switched bracketed paste ends,
   * in units from its front. Status is taken per read: a read with the switch
   * sets it, and the reads after it fall back to the patterns. A batch
   * analyzed whole would let the switch win over every read that followed it,
   * so a batch is never taken past this point in one call.
   */
  private analysisSignalEnd = new Map<string, number>()
  private analysisTimers = new Map<string, ReturnType<typeof setTimeout>>()
  private idleTimers = new Map<string, ReturnType<typeof setTimeout>>()
  private sessionOrder: string[] = []
  private headlessWorktreeCounter?: WorktreeSessionCounter

  /** Provide headless session counter to avoid circular imports */
  setHeadlessWorktreeCounter(counter: WorktreeSessionCounter): void {
    this.headlessWorktreeCounter = counter
  }

  /** Count all sessions (pty + headless) using a worktree, excluding one ID */
  private countWorktreeSessions(worktreePath: string, excludeId?: string): number {
    const pty = this.getActiveSessionsForWorktree(worktreePath, excludeId)
    const headless = this.headlessWorktreeCounter?.(worktreePath, excludeId) ?? {
      count: 0,
      sessionIds: []
    }
    return pty.count + headless.count
  }

  constructor() {
    super()
    setImmediate(() => this.cleanStaleTempKeys())
    // Told when a shell moves, rather than checking after every flush. The
    // report comes from inside xterm's parser, which is the only moment the new
    // directory is actually known -- a flush ends before the bytes it delivered
    // have been parsed, so anything reading there reads the previous value.
    setCwdReporter((id, cwd) => this.noteShellCwd(id, cwd))
    // A terminal on a core thread finds its bells after the flush has gone.
    setBellReporter((id) => this.emit('client-message', IPC.TERMINAL_BELL, { id }))
  }

  /**
   * Keep a shell's record pointing at where the shell actually is.
   *
   * `shellCwd` was written once at spawn and never moved, so restoring a shell
   * put somebody back where they started rather than where they were. This is
   * the record that gets persisted and the one a restored shell is offered.
   *
   * Runs inside the parser, so it stays a map lookup and an event, and the save
   * it triggers is debounced -- a script running `cd` in a loop costs one write
   * rather than hundreds.
   */
  private noteShellCwd(id: string, cwd: string): void {
    const session = this.sessions.get(id)
    if (!session || session.agentType !== 'shell' || session.shellCwd === cwd) return
    session.shellCwd = cwd
    this.emit('session-cwd', id, cwd)
  }

  /** Remove stale temp key files from previous crashes (older than 1 hour) */
  private cleanStaleTempKeys(): void {
    try {
      const tmpDir = os.tmpdir()
      const files = fs.readdirSync(tmpDir)
      const now = Date.now()
      for (const f of files) {
        if (!f.startsWith('vorn-key-')) continue
        const fullPath = path.join(tmpDir, f)
        try {
          const stat = fs.statSync(fullPath)
          if (now - stat.mtimeMs > 3600_000) {
            fs.unlinkSync(fullPath)
            log.info(`[pty] cleaned stale temp key: ${f}`)
          }
        } catch {
          /* ignore individual file errors */
        }
      }
    } catch {
      /* tmpdir read failed, not critical */
    }
  }

  private deleteTempKey(sessionId: string): void {
    const keyPath = this.tempKeyPaths.get(sessionId)
    if (keyPath) {
      try {
        fs.unlinkSync(keyPath)
      } catch {
        /* already deleted */
      }
      this.tempKeyPaths.delete(sessionId)
    }
  }

  setRemoteHosts(hosts: RemoteHost[]): void {
    this.remoteHosts = hosts
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

  private buildAgentLaunchLine(payload: CreateTerminalPayload): string {
    return buildLaunchLine(payload, this.agentCommands, getSafeEnv())
  }

  /**
   * @param reuseId Keep an existing session's id instead of minting one.
   *
   * Only resume passes this, and it is what makes a resumed session the same
   * session rather than a replacement for it. Every client keys a pane by this
   * id -- the xterm holding the replayed screen, the subscription carrying its
   * output -- so a new id means a new pane, and the screen the person was
   * looking at is thrown away at the moment they asked for it back. Reusing it
   * also means the new run's history supersedes the old run's under the same
   * name, which `startHistory` does on its own queue.
   */
  async createPty(payload: CreateTerminalPayload, reuseId?: string): Promise<TerminalSession> {
    return this.spawnPty(payload, await this.prepareSession(payload), reuseId)
  }

  /**
   * Everything a session needs before its PTY exists: which host it runs on,
   * and for a local one the agent's launch line and the worktree or branch it
   * runs on. Async because that can mean git, and a worktree add takes seconds
   * on a big repository; with native git it no longer holds the event loop.
   *
   * Split from `spawnPty` so a caller that checks and claims around a spawn
   * (a resume claiming its transcript) can await this first and then do the
   * check, the claim and the spawn in one synchronous step, with nothing able
   * to run between them.
   */
  async prepareSession(payload: CreateTerminalPayload): Promise<PreparedSession> {
    refuseWhileClosing()
    const remoteHost = payload.remoteHostId
      ? this.remoteHosts.find((h) => h.id === payload.remoteHostId)
      : undefined
    if (remoteHost) return { remoteHost }
    // Held from here until `spawnPty` makes it a session, so a worktree action
    // in between sees it as in use: the worktree it names, and one it creates.
    const releases: (() => void)[] = []
    const hold = (dir: string): void => {
      releases.push(holdWorkspace(dir))
    }
    const release = (): void => releases.forEach((r) => r())
    if (payload.existingWorktreePath) hold(payload.existingWorktreePath)
    try {
      const local = await this.prepareLocal(payload, hold)
      local.release = release
      return { local }
    } catch (err) {
      release()
      throw err
    }
  }

  /** @param prepared From `prepareSession` on this same payload. */
  spawnPty(
    payload: CreateTerminalPayload,
    prepared: PreparedSession,
    reuseId?: string
  ): TerminalSession {
    try {
      // Checked again: closing may have begun while the workspace was prepared.
      refuseWhileClosing()
      const id = reuseId ?? crypto.randomUUID()
      const shell = getDefaultShell(configManager.loadConfig().defaults.shell)

      const session =
        'remoteHost' in prepared
          ? this.createRemotePty(id, shell, payload, prepared.remoteHost)
          : this.createLocalPty(id, shell, payload, prepared.local)

      this.emit('session-created', session, payload)
      return session
    } finally {
      if ('local' in prepared) prepared.local.release?.()
    }
  }

  private async prepareLocal(
    payload: CreateTerminalPayload,
    hold: (dir: string) => void
  ): Promise<PreparedLocal> {
    // Session ID pinning: agents that support it (supportsSessionIdPinning) get a
    // UUID assigned on fresh launch via --session-id, enabling exact --resume later.
    // Other agents rely on history-based fallback for resume.
    let agentSessionId = supportsExactSessionResume(payload.agentType)
      ? payload.resumeSessionId
      : undefined
    if (supportsSessionIdPinning(payload.agentType)) {
      if (payload.resumeSessionId) {
        agentSessionId = payload.resumeSessionId
      } else {
        agentSessionId = crypto.randomUUID()
        payload.sessionId = agentSessionId
      }
    }
    // Built before a worktree or PTY exists, so a line that cannot be built creates nothing.
    const launchLine = this.buildAgentLaunchLine(payload)

    let effectivePath = payload.projectPath
    let worktreePath: string | undefined
    let worktreeName: string | undefined
    let effectiveBranch: string | undefined

    if (payload.existingWorktreePath && fs.existsSync(payload.existingWorktreePath)) {
      effectivePath = payload.existingWorktreePath
      effectiveBranch = payload.branch
      const isMainWorktree =
        normalizePath(payload.existingWorktreePath) === normalizePath(payload.projectPath)
      if (!isMainWorktree) {
        worktreePath = payload.existingWorktreePath
        worktreeName = payload.worktreeName || extractWorktreeName(payload.existingWorktreePath)
      }
    } else if ((payload.useWorktree || payload.existingWorktreePath) && payload.branch) {
      if (await isGitRepo(payload.projectPath)) {
        if (payload.existingWorktreePath) {
          log.warn(
            `[pty] worktree path no longer exists, creating new: ${payload.existingWorktreePath}`
          )
        }
        const result = await createWorktree(
          payload.projectPath,
          payload.branch,
          payload.worktreeName,
          undefined,
          hold
        )
        effectivePath = result.worktreePath
        worktreePath = result.worktreePath
        worktreeName = result.name
        effectiveBranch = result.branch
      } else {
        log.warn(`[pty] skipping worktree for non-git project: ${payload.projectPath}`)
      }
    }
    // Handle branch checkout (no worktree)
    else if (payload.branch) {
      if (await isGitRepo(payload.projectPath)) {
        const currentBranch = await getGitBranch(payload.projectPath)
        if (currentBranch !== payload.branch) {
          await checkoutBranch(payload.projectPath, payload.branch)
        }
        effectiveBranch = payload.branch
      }
    }

    const branch = effectiveBranch || (await getGitBranch(effectivePath))
    const headCommit = await getGitHead(effectivePath)
    return {
      agentSessionId,
      launchLine,
      effectivePath,
      worktreePath,
      worktreeName,
      branch,
      headCommit
    }
  }

  private createLocalPty(
    id: string,
    shell: string,
    payload: CreateTerminalPayload,
    prepared: PreparedLocal
  ): TerminalSession {
    const {
      agentSessionId,
      launchLine,
      effectivePath,
      worktreePath,
      worktreeName,
      branch,
      headCommit
    } = prepared
    const ptyProcess = pty.spawn(shell, getShellArgs(), {
      name: 'xterm-256color',
      cols: INITIAL_COLS,
      rows: INITIAL_ROWS,
      cwd: effectivePath,
      // No shell integration: an agent paints its own full-screen interface
      // and is never drawn as command blocks. Installing the shim anyway made
      // the wrapper shell emit boundaries, which hid the terminal cursor and
      // drew block decorations into a card with no spine or input bar.
      //
      // VORN_SESSION_ID is spread in *here*, at the spawn site, never set on the
      // ambient process env: `filterEnv` only strips keys, so an ambient value
      // would be inherited by every child of every session and the browser tools
      // — which resolve their session from this variable alone — would silently
      // lose their isolation.
      env: { ...getLaunchEnv(), VORN_SESSION_ID: id }
    })

    setTimeout(() => ptyProcess.write(launchLine + '\r'), 300)

    this.setupPtyEvents(id, ptyProcess, INITIAL_COLS, INITIAL_ROWS)
    this.ptys.set(id, ptyProcess)

    const session: TerminalSession = {
      id,
      agentType: payload.agentType,
      projectName: payload.projectName,
      projectPath: payload.projectPath,
      status: 'running',
      createdAt: Date.now(),
      cols: INITIAL_COLS,
      rows: INITIAL_ROWS,
      pid: ptyProcess.pid,
      ...(payload.displayName
        ? { displayName: payload.displayName }
        : payload.initialPrompt
          ? { displayName: displayNameFromPrompt(payload.initialPrompt) }
          : {}),
      ...(branch ? { branch } : {}),
      ...(headCommit ? { headCommit } : {}),
      ...(worktreePath ? { worktreePath, worktreeName, isWorktree: true } : {}),
      // Don't set statusSource: 'hooks' eagerly — promoteToHookStatus() sets it
      // when the first hook event actually arrives. This provides graceful
      // degradation: if hooks fail (uninstalled, port conflict, etc.), the
      // pattern-based fallback keeps working instead of leaving status stuck.
      ...(agentSessionId ? { agentSessionId } : {})
    }
    this.sessions.set(id, session)
    this.sessionOrder.push(id)
    this.normalizedPaths.set(id, normalizePath(worktreePath || payload.projectPath))
    return session
  }

  private createRemotePty(
    id: string,
    shell: string,
    payload: CreateTerminalPayload,
    host: RemoteHost
  ): TerminalSession {
    const agentLine = this.buildAgentLaunchLine(payload)
    const ptyProcess = pty.spawn(shell, getShellArgs(), {
      name: 'xterm-256color',
      cols: INITIAL_COLS,
      rows: INITIAL_ROWS,
      cwd: os.homedir(),
      env: getSafeEnv()
    })

    // Build SSH command based on auth method, with a ready marker for reliable prompt detection
    const marker = `__VORN_READY_${id.slice(0, 8)}__`
    const sshParts: string[] = ['ssh', '-t']
    if (host.port !== 22) sshParts.push('-p', String(host.port))

    const authMethod = host.authMethod ?? 'agent'

    if (authMethod === 'key-file' && host.sshKeyPath) {
      sshParts.push('-i', host.sshKeyPath)
    } else if (authMethod === 'key-stored' && !payload._decryptedKeyContent) {
      log.warn(
        `[pty] key-stored auth selected for host ${host.label} but no decrypted key available — falling back to agent`
      )
    } else if (authMethod === 'key-stored' && payload._decryptedKeyContent) {
      // Write decrypted key to a temp file (mode 0600)
      const tmpKeyPath = path.join(os.tmpdir(), `vorn-key-${crypto.randomUUID()}`)
      fs.writeFileSync(tmpKeyPath, payload._decryptedKeyContent, { mode: 0o600 })
      this.tempKeyPaths.set(id, tmpKeyPath)
      sshParts.push('-i', tmpKeyPath)
    } else if (authMethod === 'password') {
      sshParts.push('-o', 'PreferredAuthentications=password')
      sshParts.push('-o', 'PubkeyAuthentication=no')
    }
    // 'agent' auth: no extra flags, rely on ssh-agent

    if (host.sshOptions) {
      const opts = host.sshOptions.split(/\s+/).filter(Boolean)
      sshParts.push(...opts)
    }
    sshParts.push(`${host.user}@${host.hostname}`)
    // Echo a unique marker on connect, then exec a login shell so the session stays alive.
    // Single-quoted so the local shell passes && and $SHELL literally to SSH,
    // which forwards them to the remote shell for interpretation.
    sshParts.push(`'echo ${marker} && exec $SHELL -l'`)

    // Build remote command: cd to project path then launch agent
    const remoteCmd = `cd ${shellEscape(payload.projectPath, 'posix')} && ${agentLine}`

    // Write SSH command after local shell is ready
    setTimeout(() => {
      if (this.ptys.has(id)) ptyProcess.write(sshParts.join(' ') + '\r')
    }, 300)

    // Password prompt auto-detection
    if (authMethod === 'password' && payload._decryptedPassword) {
      // Captured in a closure: the credentials are stripped from the payload a
      // few lines below, long before a prompt ever arrives.
      const password = payload._decryptedPassword
      let passwordSent = false
      const pwListener = ptyProcess.onData((data: string) => {
        if (!passwordSent && /[Pp]ass(word|phrase)[^:]*:\s*$/.test(data)) {
          passwordSent = true
          setTimeout(() => {
            if (this.ptys.has(id)) ptyProcess.write(password + '\r')
          }, 50)
        }
      })
      setTimeout(() => pwListener.dispose(), 15_000)
    }

    // Clear transient credentials from payload
    delete payload._decryptedKeyContent
    delete payload._decryptedPassword

    let connected = false
    let sshOutput = ''

    // Fallback: if marker never arrives (non-standard shell), send command after timeout
    const fallbackTimer = setTimeout(() => {
      if (!connected) {
        connected = true
        log.warn(`[pty] SSH marker not detected for ${id}, using fallback`)
        if (this.ptys.has(id)) ptyProcess.write(remoteCmd + '\r')
        this.deleteTempKey(id)
      }
    }, 8000)

    const promptListener = ptyProcess.onData((data: string) => {
      if (connected) return
      sshOutput += data

      // Primary: detect our unique marker
      if (sshOutput.includes(marker)) {
        connected = true
        clearTimeout(fallbackTimer)
        // Small delay to let the login shell fully initialize
        setTimeout(() => {
          if (this.ptys.has(id)) ptyProcess.write(remoteCmd + '\r')
          this.deleteTempKey(id)
        }, 200)
        return
      }

      // Detect SSH errors early to avoid waiting for full timeout
      const errorPatterns = [
        'Permission denied',
        'Connection refused',
        'Connection timed out',
        'Could not resolve hostname',
        'No route to host',
        'Connection closed',
        'Host key verification failed'
      ]
      for (const pattern of errorPatterns) {
        if (sshOutput.includes(pattern)) {
          log.error(`[pty] SSH connection error for ${id}: ${pattern}`)
          clearTimeout(fallbackTimer)
          this.deleteTempKey(id)
          // Don't set connected — let the PTY show the error to the user
          return
        }
      }
    })

    // Forward all data to the renderer from the start
    this.setupPtyEvents(id, ptyProcess, INITIAL_COLS, INITIAL_ROWS)
    this.ptys.set(id, ptyProcess)

    // Clean up the prompt listener after connection or timeout
    const cleanup = (): void => {
      promptListener.dispose()
    }
    const checkConnected = setInterval(() => {
      if (connected) {
        cleanup()
        clearInterval(checkConnected)
      }
    }, 200)
    setTimeout(() => {
      cleanup()
      clearInterval(checkConnected)
    }, 10000)

    const session: TerminalSession = {
      id,
      agentType: payload.agentType,
      projectName: payload.projectName,
      projectPath: payload.projectPath,
      status: 'running',
      createdAt: Date.now(),
      cols: INITIAL_COLS,
      rows: INITIAL_ROWS,
      pid: ptyProcess.pid,
      remoteHostId: host.id,
      remoteHostLabel: host.label,
      ...(payload.resumeSessionId && supportsExactSessionResume(payload.agentType)
        ? { agentSessionId: payload.resumeSessionId }
        : {}),
      ...(payload.displayName
        ? { displayName: payload.displayName }
        : payload.initialPrompt
          ? { displayName: displayNameFromPrompt(payload.initialPrompt) }
          : {})
    }
    this.sessions.set(id, session)
    this.sessionOrder.push(id)
    this.normalizedPaths.set(id, normalizePath(payload.projectPath))
    return session
  }

  /** @param reuseId As `createPty`: a resumed shell keeps the pane it was in. */
  createShellPty(cwd?: string, reuseId?: string): TerminalSession {
    const id = reuseId ?? crypto.randomUUID()
    const shell = getDefaultShell(configManager.loadConfig().defaults.shell)
    const workingDir = cwd || os.homedir()
    const integration = getShellIntegration({
      shell,
      minimalPrompt: configManager.loadConfig().defaults.minimalShellPrompt
    })
    // bash and PowerShell have no environment variable that injects
    // initialisation, so integration for them replaces the launch arguments.
    const ptyProcess = pty.spawn(shell, integration.args ?? getShellArgs(), {
      name: 'xterm-256color',
      cols: INITIAL_COLS,
      rows: INITIAL_ROWS,
      cwd: workingDir,
      env: {
        ...getSafeEnv(),
        ...integration.env,
        // Spawn-site only — see the note in createLocalPty.
        VORN_SESSION_ID: id
      }
    })
    this.setupPtyEvents(id, ptyProcess, INITIAL_COLS, INITIAL_ROWS)
    this.ptys.set(id, ptyProcess)

    const shellCount =
      Array.from(this.sessions.values()).filter((s) => s.agentType === 'shell').length + 1
    const projectName = path.basename(workingDir) || 'shell'
    const session: TerminalSession = {
      id,
      agentType: 'shell',
      projectName,
      projectPath: workingDir,
      status: 'running',
      createdAt: Date.now(),
      cols: INITIAL_COLS,
      rows: INITIAL_ROWS,
      pid: ptyProcess.pid,
      displayName: `Shell ${shellCount}`,
      shellCwd: workingDir
    }
    this.sessions.set(id, session)
    this.sessionOrder.push(id)
    this.normalizedPaths.set(id, normalizePath(workingDir))
    return session
  }

  /**
   * A terminal running an extension's own program, in the worktree it is about.
   *
   * Its own spawn rather than `createShellPty` with arguments: this runs one
   * named program instead of a login shell, so it takes none of the shell
   * integration, and it carries the extension's bridge names so the program can
   * ask the host the same things a footer can.
   */
  createExtensionPty(params: {
    command: string
    args: string[]
    cwd: string
    displayName: string
    env: Record<string, string>
  }): TerminalSession {
    const id = crypto.randomUUID()
    const ptyProcess = pty.spawn(params.command, params.args, {
      name: 'xterm-256color',
      cols: INITIAL_COLS,
      rows: INITIAL_ROWS,
      cwd: params.cwd,
      env: { ...getSafeEnv(), ...params.env, VORN_SESSION_ID: id }
    })
    this.setupPtyEvents(id, ptyProcess, INITIAL_COLS, INITIAL_ROWS)
    this.ptys.set(id, ptyProcess)

    const session: TerminalSession = {
      id,
      agentType: 'shell',
      projectName: path.basename(params.cwd) || 'extension',
      projectPath: params.cwd,
      status: 'running',
      createdAt: Date.now(),
      cols: INITIAL_COLS,
      rows: INITIAL_ROWS,
      pid: ptyProcess.pid,
      displayName: params.displayName,
      shellCwd: params.cwd
    }
    this.sessions.set(id, session)
    this.extensionPtys.add(id)
    this.normalizedPaths.set(id, normalizePath(params.cwd))
    return session
  }

  /** Whether this PTY is an extension's pane rather than a session someone started. */
  isExtensionPty(id: string): boolean {
    return this.extensionPtys.has(id)
  }

  /** How long a stream is held so its many small reads go out as one flush. */
  private static readonly BUFFER_FLUSH_MS = 8
  /** A read this small after a quiet spell is a keystroke's echo; a TUI's repaint is never this small. */
  private static readonly ECHO_MAX_BYTES = 64

  /**
   * Put bytes into a session's output as though the process had written them.
   *
   * There is exactly one caller and one reason: a resumed session hands a new
   * process a terminal the previous one was still using, and something has to
   * sit between the two runs saying so. Doing it in the client cannot work --
   * the client is not what orders these bytes. A cold pane has not mounted when
   * the resume starts, so it has no terminal to reset yet, and the screen it
   * replays is written when it finally does mount, by which time the new
   * process has been streaming for a second. The two interleave and what
   * arrives is both frames at once with the escapes showing.
   *
   * Through `bufferData` rather than beside it, so this takes a sequence number,
   * a place in the scrollback and a line in the history like any other output.
   * That is what makes it arrive in the right order for a client that attaches
   * in a minute as well as for the one watching now.
   */
  injectOutput(id: string, data: string): void {
    this.bufferData(id, data)
  }

  private bufferData(id: string, data: string): void {
    const existing = this.dataBuffers.get(id)
    const held = holdOutput(existing, data)
    if (!existing) this.dataBuffers.set(id, held)
    // A full flush's worth goes out on the next turn rather than waiting out
    // the timer: holding more would only make that flush longer.
    if (held.units >= MAX_FLUSH_UNITS) this.queueDrain(id)
    // A pending timer means a stream is in flight, and this read joins it.
    if (this.flushTimers.has(id)) return

    // An echo held for company that never comes is what a keystroke feels as lag.
    if (!existing && Buffer.byteLength(data) <= PtyManager.ECHO_MAX_BYTES) this.flushBuffer(id)
    this.armFlush(id)
  }

  /**
   * Sessions with more than one flush's worth held, served one flush per turn.
   *
   * One queue for every session, and one flush per turn of the event loop,
   * rather than draining each session to empty or each on its own
   * `setImmediate`. Immediates queued together all run in the same turn, so a
   * burst across eight terminals would hold the loop for eight flushes at once;
   * this holds it for one, and lets timers -- a keystroke's echo, an RPC reply
   * -- in between. Round-robin, so a session printing a gigabyte does not
   * starve one printing a line.
   */
  private drainQueue = new Set<string>()
  /** The same, for native analysis that a burst left more than 64 KB of. */
  private analysisQueue = new Set<string>()
  private drainScheduled = false
  /** Whether the next drain turn is analysis, when both are waiting. */
  private analyseNext = false

  private queueDrain(id: string): void {
    this.drainQueue.add(id)
    this.scheduleDrain()
  }

  private queueAnalysis(id: string): void {
    this.analysisQueue.add(id)
    this.scheduleDrain()
  }

  private scheduleDrain(): void {
    if (this.drainScheduled) return
    this.drainScheduled = true
    setImmediate(() => this.drainOne())
  }

  /**
   * One flush's worth of flushing and one of analysis, then back to the loop.
   *
   * By size rather than by count: small flushes from many quiet sessions go
   * out together in one turn, and one large one goes out alone.
   */
  private drainOne(): void {
    this.drainScheduled = false
    // Flushing and analysing take turns rather than sharing one: each is up to
    // a budget's worth of work, and the two together in one turn were the
    // longest the loop went without answering anything else.
    const analyse = this.analysisQueue.size > 0 && (this.analyseNext || !this.drainQueue.size)
    this.analyseNext = !analyse
    let budget = MAX_FLUSH_UNITS
    while (budget > 0) {
      const id = first(analyse ? this.analysisQueue : this.drainQueue)
      if (id === undefined) break
      if (analyse) {
        this.analysisQueue.delete(id)
        budget -= this.flushAnalysis(id, budget)
      } else {
        this.drainQueue.delete(id)
        // Flushing re-queues it at the back if a full flush's worth is still
        // held, or if the budget cut it short.
        budget -= this.flushBuffer(id, budget)
      }
    }
    if (this.drainQueue.size || this.analysisQueue.size) this.scheduleDrain()
  }

  /** The hold, re-armed while a stream keeps coming so quiet is the timer lapsing with nothing to send. */
  private armFlush(id: string): void {
    this.flushTimers.set(
      id,
      setTimeout(() => {
        this.flushTimers.delete(id)
        if (!this.dataBuffers.has(id)) return
        // Through the shared queue rather than flushed here: every session's
        // timer can fire in the same turn, and eight flushes at once is the
        // stall the cap exists to prevent.
        this.queueDrain(id)
        this.armFlush(id)
      }, PtyManager.BUFFER_FLUSH_MS)
    )
  }

  /**
   * How many flushes each session has had.
   *
   * The number a client uses to tell what it already has. A pane attaching
   * asks for the scrollback and is told which flush it reflects; every
   * `terminal:data` carries the same counter, so anything at or below that
   * number is already in what it was handed and anything above it is not.
   *
   * This works only because `flushBuffer` below is one synchronous block. The
   * counter moves and the buffer it describes is appended in the same tick, with
   * nothing awaited between them, so a reader that takes both in one turn cannot
   * catch them disagreeing. **Introduce an `await` in there and this silently
   * stops being true**, and the symptom is a terminal that duplicates or loses a
   * few hundred milliseconds of output on attach.
   */
  private flushSeq = new Map<string, number>()

  /** What the last flush of this session was numbered. */
  lastFlushSeq(id: string): number {
    return this.flushSeq.get(id) ?? 0
  }

  /**
   * Where each session's record log has reached: the first record and byte
   * not yet given out. The Session Recovery Contract's cursor, assigned here
   * because this is the one place that sees output and resizes in the order
   * they happened. Moves in the same synchronous block as `flushSeq`, so an
   * attach that reads both in one turn gets numbers that agree.
   */
  private cursors = new Map<string, RecordCursor>()

  /** The cursor after this session's last record, or null when it has none. */
  recordCursor(id: string): RecordCursor | null {
    const at = this.cursors.get(id)
    return at ? { ...at } : null
  }

  /**
   * Start a session's record log again, in an epoch of its own.
   *
   * Random rather than counted, so it cannot repeat across server restarts
   * without anything persisted: a cursor from a previous run of this id names
   * nothing in this one, and comparing epochs is how a reader finds that out.
   */
  private openRecords(id: string): RecordCursor {
    const at = { epoch: crypto.randomInt(1, 0xffffffff), nextRseq: 0, nextOffset: 0 }
    this.cursors.set(id, at)
    return { ...at }
  }

  /** Number the next record, `bytes` long, and move the cursor past it. */
  private nextRecord(id: string, bytes: number): RecordHeader {
    const at = this.cursors.get(id) ?? this.openRecords(id)
    const header = { rseq: at.nextRseq, startOffset: at.nextOffset }
    at.nextRseq += 1
    at.nextOffset += bytes
    return header
  }

  /**
   * Send up to one flush's worth of what is held, or `cap` when a drain turn
   * has less than that left, and say how much that was.
   */
  private flushBuffer(id: string, cap = MAX_FLUSH_UNITS): number {
    const held = this.dataBuffers.get(id)
    if (!held) return 0
    const data = takeOutput(held, Math.min(cap, MAX_FLUSH_UNITS))
    if (held.units === 0) this.dataBuffers.delete(id)
    // Cut short by the turn's budget, it goes on in the next turn rather than
    // waiting out the timer.
    else if (held.units >= MAX_FLUSH_UNITS || cap < MAX_FLUSH_UNITS) this.queueDrain(id)
    if (data) {
      const seq = this.lastFlushSeq(id) + 1
      this.flushSeq.set(id, seq)

      // Clients first, always. What follows models the screen for nobody who is
      // waiting; this line is a person watching their terminal, and it must not
      // be behind anything that can fail or stall.
      this.emit('client-message', IPC.TERMINAL_DATA, { id, data, seq })

      // Fed from here rather than from `onData` for two reasons. `term.write`
      // queues a macrotask per call and node-pty emits a few bytes at a time
      // while somebody types, so this is one queued write per session per flush
      // instead of one per keystroke. And it puts the model in step with the
      // clients rather than ahead of them -- fed from `onData`, a screen read
      // mid-flush would describe something nobody has seen yet.
      // All three from here, on the same bytes, in one place.
      //
      // `appendScrollback` used to sit on `onData` instead, and that was not
      // merely inconsistent -- it put the byte buffer ahead of the screen model
      // by up to one flush. A checkpoint takes both at the same instant, so it
      // could hold bytes in its scrollback that its screen had not seen; those
      // bytes then arrived again as log frames after it, and a restore counted
      // them twice. Fed from one point they cannot disagree.
      const bytes = Buffer.byteLength(data, 'utf-8')
      const at = this.nextRecord(id, bytes)
      const pipeline = pipelineFor(id)
      if (pipeline) {
        // All three in one hand-off to the terminal's thread, which parses,
        // keeps and frames them there, in this order with every other flush.
        // Its bell, if it rings one, comes through the reporter.
        try {
          pipeline.feed(data, noteOutput(id, at, bytes) ? at : null)
        } catch (err) {
          log.warn({ err, id }, '[core] a terminal thread stopped; dropping it')
          pipelineLost(id)
          clearScreen(id)
        }
        return data.length
      }
      appendScrollback(id, data)
      const rang = feedScreen(id, data)
      recordOutput(id, at, data)

      // The bell, said out loud rather than left for whoever happens to be
      // attached. A client only sees bytes for terminals it has opened, so a
      // notification that depended on that was a notification you got for the
      // sessions you were already looking at -- and missed for the one ringing
      // out of view, which is the only one worth interrupting anybody for.
      //
      // On the raw bytes, before any stripping: BEL is exactly what `stripAnsi`
      // exists to remove. Here rather than in `appendOutput`, which returns
      // early for a plain shell -- a shell rings too.
      //
      // The native screen model says whether a BEL actually rang, which tells
      // a bell from the BEL that ends every OSC title an agent sets; without
      // it, any 0x07 counts.
      if (rang ?? data.includes('\x07')) {
        this.emit('client-message', IPC.TERMINAL_BELL, { id })
      }
    }
    return data.length
  }

  private clearBuffer(id: string): void {
    const timer = this.flushTimers.get(id)
    if (timer) clearTimeout(timer)
    this.flushTimers.delete(id)
    this.dataBuffers.delete(id)
    this.drainQueue.delete(id)
  }

  /** Push what is buffered now, without waiting out the timer that would. */
  private drainBuffer(id: string): void {
    const timer = this.flushTimers.get(id)
    if (timer) clearTimeout(timer)
    // Forgotten as well as cleared: a timer left in the map reads as a stream in flight, and every read after it would wait for a flush that never comes.
    this.flushTimers.delete(id)
    this.drainQueue.delete(id)
    // All of it, still in flushes of at most the cap: this is the last chance.
    while (this.dataBuffers.has(id)) this.flushBuffer(id)
    this.drainQueue.delete(id)
  }

  private clearSessionTracking(id: string): void {
    this.analyzers.get(id)?.free()
    this.analyzers.delete(id)
    this.unanalyzed.delete(id)
    this.pendingAnalysis.delete(id)
    this.analysisSignalEnd.delete(id)
    this.analysisQueue.delete(id)
    const analysisTimer = this.analysisTimers.get(id)
    if (analysisTimer) clearTimeout(analysisTimer)
    this.analysisTimers.delete(id)
    this.extensionPtys.delete(id)
    const idleTimer = this.idleTimers.get(id)
    if (idleTimer) clearTimeout(idleTimer)
    this.idleTimers.delete(id)
  }

  /** The session's analyzer, created on its first output, or null when it has none. */
  private analyzerFor(id: string): NativeAnalyzer | null {
    const existing = this.analyzers.get(id)
    if (existing) return existing
    if (this.unanalyzed.has(id)) return null
    const Analyzer = nativeCore()?.Analyzer
    try {
      if (!Analyzer) throw new Error('the vorn core is not loaded')
      const analyzer = new Analyzer()
      this.analyzers.set(id, analyzer)
      return analyzer
    } catch (err) {
      log.warn({ err, id }, '[core] no output analysis for this session')
      this.unanalyzed.add(id)
      return null
    }
  }

  /**
   * How analysis batches: the first read after a quiet spell is
   * analyzed at once, so a prompt reads as waiting the moment it appears and
   * the idle countdown starts from it. Reads that follow within this window
   * wait and go into the core together, one call per window rather than per
   * read, for as long as the stream keeps coming. Status during a stream lags
   * its bytes by at most this long.
   */
  static readonly ANALYSIS_MS = 8

  private appendOutput(id: string, data: string): void {
    const session = this.sessions.get(id)
    if (!session) return

    // Plain shells don't run agents — skip bracketed-paste / pattern / idle analysis.
    // They stay 'running' until the PTY exits (setupPtyEvents sets 'idle').
    if (session.agentType === 'shell') return

    if (!this.analyzerFor(id)) {
      // No status to take, but the session still goes idle when output stops.
      this.armIdle(id, session)
      return
    }
    const pending = this.pendingAnalysis.get(id)
    const held = holdOutput(pending, data)
    if (!pending) this.pendingAnalysis.set(id, held)
    if (data.includes('\x1b[?2004')) this.analysisSignalEnd.set(id, held.units)
    if (held.units >= MAX_FLUSH_UNITS) this.queueAnalysis(id)
    if (this.analysisTimers.has(id)) return
    this.flushAnalysis(id)
    this.armAnalysis(id)
  }

  /** The batching window, re-armed while reads keep arriving so quiet is it lapsing with nothing queued. */
  private armAnalysis(id: string): void {
    this.analysisTimers.set(
      id,
      setTimeout(() => {
        this.analysisTimers.delete(id)
        if (!this.pendingAnalysis.has(id)) return
        this.queueAnalysis(id)
        this.armAnalysis(id)
      }, PtyManager.ANALYSIS_MS)
    )
  }

  /**
   * Analyze what is waiting for a session now, up to one flush's worth.
   * The rest is analyzed on the turns that follow, like a capped flush, so a
   * burst never holds the loop for more than 64 KB of analysis at once.
   */
  private flushAnalysis(id: string, cap = MAX_FLUSH_UNITS): number {
    const held = this.pendingAnalysis.get(id)
    if (held === undefined) return 0
    const data = this.takeAnalysis(id, held, Math.min(cap, MAX_FLUSH_UNITS))
    if (held.units === 0) this.pendingAnalysis.delete(id)
    else this.queueAnalysis(id)
    const session = this.sessions.get(id)
    const analyzer = this.analyzers.get(id)
    if (!session || !analyzer) return data.length
    try {
      const newStatus = NATIVE_STATUS[analyzer.append(data, session.statusSource !== 'hooks')]
      if (newStatus && newStatus !== session.status) this.setStatus(id, newStatus)
    } catch (err) {
      // This can run from a timer, where a throw has nothing behind it. The
      // session carries on without status, rather than ask a faulted analyzer
      // again on every read.
      log.warn({ err, id }, '[core] output analysis failed; this session has no status from here')
      this.dropAnalysis(id)
    }
    // Re-armed per batch rather than per read, which is most of what analysis
    // per read spent on a spinner.
    this.armIdle(id, session)
    return data.length
  }

  /** Take up to `cap` units of a pending batch, never past `analysisSignalEnd`. */
  private takeAnalysis(id: string, held: HeldOutput, cap: number): string {
    const end = this.analysisSignalEnd.get(id)
    const data = takeOutput(held, end === undefined ? cap : Math.min(cap, end))
    if (end !== undefined) {
      if (end > data.length) this.analysisSignalEnd.set(id, end - data.length)
      else this.analysisSignalEnd.delete(id)
    }
    return data
  }

  /** Stop analyzing a session whose analyzer failed, letting go of what it held. */
  private dropAnalysis(id: string): void {
    const analyzer = this.analyzers.get(id)
    this.analyzers.delete(id)
    this.unanalyzed.add(id)
    this.pendingAnalysis.delete(id)
    this.analysisSignalEnd.delete(id)
    this.analysisQueue.delete(id)
    const timer = this.analysisTimers.get(id)
    if (timer) clearTimeout(timer)
    this.analysisTimers.delete(id)
    try {
      analyzer?.free()
    } catch {
      // Nothing more to release.
    }
  }

  // Idle timer — if no output arrives within timeout, mark idle.
  // Hook sessions use a longer timeout as safety net (hooks are primary).
  private armIdle(id: string, session: TerminalSession): void {
    const timeout = session.statusSource === 'hooks' ? IDLE_TIMEOUT_HOOKS_MS : IDLE_TIMEOUT_MS
    const existingTimer = this.idleTimers.get(id)
    if (existingTimer) clearTimeout(existingTimer)
    this.idleTimers.set(
      id,
      setTimeout(() => {
        this.idleTimers.delete(id)
        const s = this.sessions.get(id)
        if (s && s.status === 'running') {
          this.setStatus(id, 'idle')
        }
      }, timeout)
    )
  }

  /**
   * @param cols - what the PTY was spawned at, passed rather than looked up.
   *   Every caller runs this *before* registering the session, so a lookup here
   *   finds nothing and silently falls back -- which is invisible while all
   *   three spawn at the same size and wrong the moment one does not.
   */
  private setupPtyEvents(
    id: string,
    ptyProcess: ManagedPty,
    cols: number,
    rows: number,
    /** An inherited pty already has its screen rebuilt from disk, and `createScreen` clears. */
    adopted = false
  ): void {
    if (!adopted || !hasScreen(id)) createScreen(id, cols, rows)
    // Replaces whatever was left under this id. A recovered session that is
    // being respawned has history describing a process that is gone.
    startHistory(id, this.openRecords(id))

    ptyProcess.onData((data: string) => {
      this.bufferData(id, data)
      // The one consumer that wants raw chunks rather than coalesced ones: it
      // reassembles partial lines and scans for bracketed paste, so it has to
      // see the stream as it arrived. Everything else is fed from the flush.
      this.appendOutput(id, data)
    })

    ptyProcess.onExit(({ exitCode }) => {
      // Whatever is buffered is the last thing this terminal ever printed.
      this.drainBuffer(id)
      this.clearBuffer(id)
      this.deleteTempKey(id)
      this.clearSessionTracking(id)
      this.flushSeq.delete(id)
      this.cursors.delete(id)
      clearScrollback(id)
      // Beside the scrollback it belongs to: the PTY is gone and nothing will
      // draw into it again. The session record survives so the card can show an
      // exit code, but its history does not -- that is pre-existing, and this
      // matches it rather than quietly deciding otherwise.
      clearScreen(id)
      // And the same for what was written for it. A terminal whose process has
      // exited has nothing worth restoring -- refused during shutdown, where the
      // PTYs are killed after the checkpoints have been written.
      stopHistory(id)
      this.sessionOrder = this.sessionOrder.filter((sid) => sid !== id)

      this.ptys.delete(id)
      const session = this.sessions.get(id)
      if (session) {
        this.emit('session-exit', session)
        session.status = 'idle'
        if (session.agentType === 'shell') {
          session.shellExitCode = exitCode
        }
        if (session.worktreePath) {
          // Only prompt cleanup when this is the last session using the worktree
          const remaining = this.countWorktreeSessions(session.worktreePath, session.id)
          if (remaining === 0) {
            this.emit('client-message', IPC.WORKTREE_CONFIRM_CLEANUP, {
              id: session.id,
              projectPath: session.projectPath,
              worktreePath: session.worktreePath
            })
          }
        }
      }
      this.emit('client-message', IPC.TERMINAL_EXIT, { id, exitCode })
    })
  }

  writeToPty(id: string, data: string): void {
    this.ptys.get(id)?.write(data)
    // For non-hook sessions, user input means the session is active.
    // Hook sessions rely on hooks to transition to running (e.g. PreToolUse).
    const session = this.sessions.get(id)
    // A prompt still waiting for analysis must not land after the input.
    if (session && session.statusSource !== 'hooks') this.settleAnalysis(id)
    if (
      session &&
      session.statusSource !== 'hooks' &&
      (session.status === 'idle' || session.status === 'waiting')
    ) {
      this.updateSessionStatus(id, 'running')
    }
  }

  /**
   * Change the geometry a program is rendering against.
   *
   * Guarded before anything is touched. This arrives as an RPC *notification*
   * -- fire-and-forget, no caller to catch a throw -- and `resize(0, 0)` throws
   * inside node-pty, so a client that fitted itself to a collapsed pane would
   * take down the handler rather than be ignored.
   *
   * The session record is updated alongside the PTY, with the same numbers, so
   * anything modelling the screen can agree with what the program is actually
   * drawing against. Two clients still fight over it -- node-pty has always been
   * last-writer-wins here and this does not change that -- but now the record
   * says which of them won.
   */
  resizePty(id: string, cols: number, rows: number): void {
    // An id this manager does not know is not merely a no-op below: the screen
    // model is keyed by session id and a restored one exists without a PTY, so a
    // cold pane fitting itself would reflow a screen nothing is drawing to.
    if (!this.sessions.has(id)) return
    if (!Number.isInteger(cols) || !Number.isInteger(rows) || cols <= 0 || rows <= 0) return
    // Bounded as well as positive, because this now outlives the process. A
    // resize frame stores its dimensions in sixteen bits, so a client asking for
    // seventy thousand columns would be recorded as four thousand -- a durable
    // disagreement between what the program was rendering against and what a
    // replay lays it out at, and one that is re-applied on every subsequent
    // start. Nothing a terminal is actually displayed at comes near this.
    if (cols > MAX_GEOMETRY || rows > MAX_GEOMETRY) return

    const session = this.sessions.get(id)
    if (session) {
      session.cols = cols
      session.rows = rows
    }
    this.ptys.get(id)?.resize(cols, rows)
    // The same numbers, so the model wraps where the program does. Not awaited:
    // this is reached from a fire-and-forget notification, and the model drains
    // its own queue before applying the size.
    const at = this.nextRecord(id, 0)
    const pipeline = pipelineFor(id)
    if (pipeline) {
      try {
        pipeline.resize(cols, rows, noteResize(id, at) ? at : null)
      } catch (err) {
        log.warn({ err, id }, '[core] a terminal thread stopped; dropping it')
        pipelineLost(id)
        clearScreen(id)
      }
      return
    }
    resizeScreen(id, cols, rows)
    recordResize(id, at, cols, rows)
  }

  /**
   * Let go of a session that is about to start again under the same id.
   *
   * `killPty` was doing this job and doing three other things with it. Its
   * process has already gone, so it emits `session-exit` for a session that is
   * coming straight back, and -- when that session was the last one in a
   * worktree -- broadcasts WORKTREE_CONFIRM_CLEANUP, which reaches the person as
   * an offer to delete the worktree the agent is at that moment being resumed
   * into. Taking it removes the tree out from under a running agent.
   *
   * It also called `stopHistory`, which queues a recursive remove of the very
   * directory `startHistory` is about to reset a few lines later.
   *
   * So this releases the id and says nothing: the maps, the buffers and the
   * screen model, which `createPty` is about to replace anyway.
   */
  releaseForResume(id: string): void {
    this.drainBuffer(id)
    this.clearBuffer(id)
    this.sessions.delete(id)
    this.heads.forget(id)
    this.normalizedPaths.delete(id)
    this.clearSessionTracking(id)
    this.flushSeq.delete(id)
    this.cursors.delete(id)
    clearScreen(id)
    this.sessionOrder = this.sessionOrder.filter((sid) => sid !== id)
    this.ptys.delete(id)
  }

  /**
   * Put back a record released for a resume whose spawn then failed.
   *
   * Releasing is destructive on purpose -- it is what lets the replacement take
   * the same id -- but a spawn that throws must not be the end of the session.
   * The restored kind is handed back to `restored-sessions`; this is the other
   * kind, a session that ended during this run and whose record lives here, and
   * without this it was released and never put anywhere. The pane's next attempt
   * found nothing and was told the session was gone.
   *
   * Reachable without malice, the same way the other one is: a project directory
   * renamed, a worktree pruned, a volume unmounted.
   */
  restoreReleased(session: TerminalSession): void {
    this.sessions.set(session.id, session)
    this.normalizedPaths.set(session.id, normalizePath(session.worktreePath || session.projectPath))
    if (!this.sessionOrder.includes(session.id)) this.sessionOrder.push(session.id)
  }

  killPty(id: string): void {
    const p = this.ptys.get(id)

    this.drainBuffer(id)
    this.clearBuffer(id)

    // Delete session and PTY from maps BEFORE killing so the onExit handler
    // (setupPtyEvents) won't find them and emit a duplicate 'session-exit'.
    // Delete-then-check pattern: single removal point prevents races.
    const session = this.sessions.get(id)
    this.sessions.delete(id)
    this.heads.forget(id)
    this.normalizedPaths.delete(id)
    this.clearSessionTracking(id)
    this.flushSeq.delete(id)
    this.cursors.delete(id)
    // Not beside a `clearScrollback`, because there is not one here -- but this
    // path deletes the session outright, so nothing would ever feed or free the
    // model again. A `Terminal` holds buffers; leaving it is a leak per closed
    // session for the life of the server.
    clearScreen(id)
    stopHistory(id)
    this.sessionOrder = this.sessionOrder.filter((sid) => sid !== id)
    this.ptys.delete(id)

    if (session) {
      this.emit('session-exit', session)
      if (session.worktreePath) {
        // Session already removed from map — count remaining sessions
        const remaining = this.countWorktreeSessions(session.worktreePath)
        if (remaining === 0) {
          this.emit('client-message', IPC.WORKTREE_CONFIRM_CLEANUP, {
            id: session.id,
            projectPath: session.projectPath,
            worktreePath: session.worktreePath
          })
        }
      }
    }
    if (p) {
      // Defer the actual kill so the IPC response returns immediately.
      // All state cleanup is already done above, so the renderer can proceed
      // without waiting for the process to die (avoids UI freeze on Windows
      // where conpty termination can block the event loop).
      setImmediate(() => {
        try {
          p.kill()
        } catch (err) {
          log.warn({ err }, `[pty] kill failed for ${id} (already dead?)`)
        }
      })
    } else {
      // Surface an exit event even if the PTY was already gone so the
      // renderer can complete any close-intent cleanup.
      this.emit('client-message', IPC.TERMINAL_EXIT, { id, exitCode: 0 })
    }
  }

  /**
   * Push out whatever is sitting in the flush buffers, without waiting for their
   * timers.
   *
   * For shutdown. The buffers hold up to `BUFFER_FLUSH_MS` of output, and that
   * output is the most recent thing the terminal showed -- the part somebody is
   * most likely to want back. `killAll` below drops it deliberately, which was
   * right while nothing outlived the process.
   */
  flushPendingOutput(): void {
    // Copied because `flushBuffer` deletes from the maps it is walking. Both,
    // because a session can hold output between a drained flush and its timer.
    for (const id of new Set([...this.flushTimers.keys(), ...this.dataBuffers.keys()])) {
      this.drainBuffer(id)
    }
  }

  /**
   * Every live pane, or null if even one cannot be described: a handoff carrying
   * most of the terminals looks exactly like losing the rest.
   */
  describeForHandoff(): DonorPane[] | null {
    const live = [...this.ptys.keys()]
    const ranked = [...live].sort((a, b) => {
      const ai = this.sessionOrder.indexOf(a)
      const bi = this.sessionOrder.indexOf(b)
      return (ai === -1 ? Number.MAX_SAFE_INTEGER : ai) - (bi === -1 ? Number.MAX_SAFE_INTEGER : bi)
    })

    const panes: DonorPane[] = []
    for (const id of ranked) {
      const held = this.ptys.get(id)
      const session = this.sessions.get(id)
      if (!held || !session) {
        // All-or-nothing, the same as a missing descriptor below. A pty whose
        // record has gone is one the replacement could not be told about, and
        // skipping it would hand over a machine quietly missing a pane.
        log.warn({ id }, '[pty] this terminal has no session record to hand over')
        return null
      }
      const fd = masterFd(held)
      if (fd === null) {
        log.warn({ id }, '[pty] this terminal has no descriptor to hand over')
        return null
      }
      panes.push({
        session,
        fd,
        pid: held.pid,
        cols: session.cols ?? INITIAL_COLS,
        rows: session.rows ?? INITIAL_ROWS
      })
    }
    return panes
  }

  /** Stop every reader, so a handoff describes a machine that is holding still. */
  pauseAllForHandoff(): void {
    for (const held of this.ptys.values()) {
      try {
        held.pause()
      } catch (err) {
        log.warn({ err }, '[pty] could not pause a terminal for the handoff')
      }
    }
  }

  /** Start reading again, for a handoff that did not happen. */
  resumeAllForHandoff(): void {
    for (const held of this.ptys.values()) {
      try {
        held.resume()
      } catch (err) {
        log.warn({ err }, '[pty] could not resume a terminal after an abandoned handoff')
      }
    }
  }

  /** Called after `recoverHistory`, so the first byte read is the first with somewhere to go. */
  adoptPanes(panes: AdoptedPane[]): void {
    for (const pane of panes) {
      const { session } = pane
      this.sessions.set(session.id, session)
      if (!this.sessionOrder.includes(session.id)) this.sessionOrder.push(session.id)
      this.normalizedPaths.set(
        session.id,
        normalizePath(session.worktreePath || session.projectPath)
      )
      this.setupPtyEvents(session.id, pane.pty, pane.cols, pane.rows, true)
      this.ptys.set(session.id, pane.pty)
    }
    // Every session wired before any of them reads.
    for (const pane of panes) pane.pty.resume()
    if (panes.length) {
      log.info({ panes: panes.length }, '[pty] adopted terminals from the previous server')
    }
  }

  killAll(): void {
    // Dropped rather than flushed. `shutdown()` calls `flushPendingOutput()`
    // ahead of this precisely because these bytes do matter now that history
    // outlives the process -- by the time this runs they have been written, and
    // what is left is whatever arrived in between, with nowhere to go.
    for (const timer of this.flushTimers.values()) {
      clearTimeout(timer)
    }
    this.dataBuffers.clear()
    this.drainQueue.clear()
    this.analysisQueue.clear()
    this.flushTimers.clear()
    this.flushSeq.clear()
    this.cursors.clear()

    // Clean up any remaining temp key files
    for (const sessionId of this.tempKeyPaths.keys()) {
      this.deleteTempKey(sessionId)
    }

    for (const [id, p] of this.ptys) {
      p.kill()
      this.ptys.delete(id)
    }
    this.sessions.clear()
    for (const analyzer of this.analyzers.values()) analyzer.free()
    this.analyzers.clear()
    this.unanalyzed.clear()
    this.pendingAnalysis.clear()
    this.analysisSignalEnd.clear()
    for (const timer of this.analysisTimers.values()) clearTimeout(timer)
    this.analysisTimers.clear()
    for (const timer of this.idleTimers.values()) clearTimeout(timer)
    this.idleTimers.clear()
    this.sessionOrder = []
  }

  /**
   * How many terminals still have a process behind them.
   *
   * Not `getActiveSessions().length`. That returns session *records*, and a
   * record outlives its process: when a shell exits on its own, `onExit` drops
   * the pty and marks the session `'idle'`, but the record stays so the card can
   * keep showing its exit code until somebody closes it. Only `killPty` removes
   * it. So a finished-but-still-open tab reads as a live session for ever, which
   * is precisely the state the idle check must not treat as busy.
   */
  livePtyCount(): number {
    return this.ptys.size
  }

  /**
   * Whether a process is still behind this session.
   *
   * `getActiveSessions()` cannot answer it. That returns session *records*, and
   * a record outlives its process -- only `killPty` removes one -- so a terminal
   * that exited on its own is still in that list. This is the map that decides.
   */
  hasLivePty(id: string): boolean {
    return this.ptys.has(id)
  }

  /** Records with a process behind them, which `getActiveSessions` alone cannot say. */
  getLiveSessions(): TerminalSession[] {
    return this.getActiveSessions().filter((session) => this.hasLivePty(session.id))
  }

  getActiveSessions(): TerminalSession[] {
    // An extension's pane PTY is deliberately absent: this list is what the app
    // is told about and what is persisted, and a pane is neither.
    if (this.sessionOrder.length === 0) {
      return Array.from(this.sessions.values()).filter((s) => !this.extensionPtys.has(s.id))
    }
    const ordered: TerminalSession[] = []
    const seen = new Set<string>()
    for (const id of this.sessionOrder) {
      const s = this.sessions.get(id)
      if (s && !this.extensionPtys.has(id)) {
        ordered.push(s)
        seen.add(id)
      }
    }
    for (const s of this.sessions.values()) {
      if (!seen.has(s.id) && !this.extensionPtys.has(s.id)) ordered.push(s)
    }
    return ordered
  }

  /**
   * A status from outside the output (a hook, a permission request). Output
   * that arrived before it is analyzed first, so a deferred batch can't land
   * after it and overwrite it.
   */
  updateSessionStatus(id: string, status: AgentStatus): void {
    this.settleAnalysis(id)
    this.setStatus(id, status)
  }

  /** Analyze everything that has arrived for a native session, past the cap. */
  private settleAnalysis(id: string): void {
    while (this.pendingAnalysis.has(id)) this.flushAnalysis(id)
  }

  private setStatus(id: string, status: AgentStatus): void {
    const session = this.sessions.get(id)
    if (session && session.status !== status) {
      session.status = status
      this.emit('client-message', IPC.SESSION_UPDATED, session)
    }
  }

  /** Promote a session to hook-based status detection (disables pattern fallback). */
  promoteToHookStatus(id: string): void {
    const session = this.sessions.get(id)
    if (!session) return
    // Output from before the hook is analyzed as the session was then.
    this.settleAnalysis(id)

    if (session.statusSource !== 'hooks') {
      session.statusSource = 'hooks'
      log.info(`[pty] session ${id} promoted to hook-based status`)
    }

    // Always re-arm idle timer with the longer hook timeout — even if already
    // promoted — so that repeated hook events keep the timer fresh and the
    // short pattern-based timer doesn't linger from before promotion.
    const existingTimer = this.idleTimers.get(id)
    if (existingTimer) {
      clearTimeout(existingTimer)
      this.idleTimers.set(
        id,
        setTimeout(() => {
          this.idleTimers.delete(id)
          if (session.status === 'running') {
            this.setStatus(id, 'idle')
          }
        }, IDLE_TIMEOUT_HOOKS_MS)
      )
    }
  }

  /** `byPerson` records who chose the name, which is what an extension may not overrule. */
  renameSession(id: string, displayName: string, byPerson = true): void {
    const session = this.sessions.get(id)
    if (!session) throw new Error(`Session not found: ${id}`)
    session.displayName = displayName
    if (byPerson) session.renamedByPerson = true
    this.emit('client-message', IPC.SESSION_UPDATED, session)
  }

  /** Files a session under a group, or takes it out of one with null. */
  setSessionGroup(id: string, groupId: string | null): void {
    const session = this.sessions.get(id)
    if (!session) throw new Error(`Session not found: ${id}`)
    if (groupId) session.groupId = groupId
    else delete session.groupId
    this.emit('client-message', IPC.SESSION_UPDATED, session)
  }

  reorderSessions(ids: string[]): void {
    if (new Set(ids).size !== ids.length) throw new Error('Duplicate session IDs')
    for (const id of ids) {
      if (!this.sessions.has(id)) throw new Error(`Session not found: ${id}`)
    }
    this.sessionOrder = ids
    this.emit('client-message', IPC.SESSION_REORDERED, ids)
  }

  getOutput(id: string, lines?: number): string[] {
    if (!this.sessions.has(id)) throw new Error(`Session not found: ${id}`)
    // What has arrived counts, analyzed or not -- all of it, past the cap.
    this.settleAnalysis(id)
    return this.analyzers.get(id)?.output(lines) ?? []
  }

  getActiveSessionsForWorktree(
    worktreePath: string,
    excludeId?: string
  ): { count: number; sessionIds: string[] } {
    const sessionIds: string[] = []
    for (const s of this.sessions.values()) {
      if (s.worktreePath === worktreePath && s.status !== 'idle' && s.id !== excludeId) {
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
        this.heads.invalidate(s.id)
        this.emit('client-message', IPC.SESSION_UPDATED, s)
      }
    }
  }

  /**
   * Finds the most-recently-created terminal matching cwd that:
   * - is NOT already linked to a Claude session (no hookSessionId)
   * - is NOT in the excludeIds set (already claimed by another session_id)
   */
  findUnlinkedSessionByCwd(cwd: string, excludeIds: Set<string>): TerminalSession | undefined {
    const normalizedCwd = normalizePath(cwd)
    let best: TerminalSession | undefined
    let bestTime = 0

    for (const session of this.sessions.values()) {
      if (session.hookSessionId) continue // already linked
      if (excludeIds.has(session.id)) continue
      const sessionPath =
        this.normalizedPaths.get(session.id) ??
        normalizePath(session.worktreePath || session.projectPath)
      if (sessionPath === normalizedCwd && session.createdAt > bestTime) {
        best = session
        bestTime = session.createdAt
      }
    }

    return best
  }
}

export const ptyManager = new PtyManager()

/** The oldest entry of a set, which iterates in insertion order. */
function first<T>(set: Set<T>): T | undefined {
  return set.values().next().value
}
