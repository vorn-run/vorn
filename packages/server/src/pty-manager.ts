import * as pty from 'node-pty'
import crypto from 'node:crypto'
import os from 'node:os'
import fs from 'node:fs'
import path from 'node:path'
import { EventEmitter } from 'node:events'
import log from './logger'
import {
  AiAgentType,
  AgentStatus,
  AgentCommandConfig,
  CreateTerminalPayload,
  IPC,
  TerminalSession,
  RemoteHost,
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
import { stripAnsi } from './ansi-strip'
import { appendScrollback, clearScrollback } from './terminal-scrollback'
import {
  createScreen,
  feedScreen,
  resizeScreen,
  clearScreen,
  setCwdReporter
} from './terminal-screen'
import { startHistory, recordOutput, recordResize, stopHistory } from './history/writer'
import { analyzeOutput, createStatusContext, StatusContext } from './status-parser'
import { isDraining, DRAINING_MESSAGE } from './draining'

const MAX_OUTPUT_LINES = 1000

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

// Bracketed paste mode: programs enable this when ready for input
// eslint-disable-next-line no-control-regex
const BRACKETED_PASTE_ON = /\x1b\[\?2004h/
// eslint-disable-next-line no-control-regex
const BRACKETED_PASTE_OFF = /\x1b\[\?2004l/

type WorktreeSessionCounter = (
  worktreePath: string,
  excludeId?: string
) => { count: number; sessionIds: string[] }

class PtyManager extends EventEmitter {
  private ptys = new Map<string, pty.IPty>()
  private sessions = new Map<string, TerminalSession>()
  private normalizedPaths = new Map<string, string>()
  private agentCommands: Record<AiAgentType, AgentCommandConfig> = { ...DEFAULT_AGENT_COMMANDS }
  private remoteHosts: RemoteHost[] = []
  private dataBuffers = new Map<string, string>()
  private flushTimers = new Map<string, ReturnType<typeof setTimeout>>()
  private tempKeyPaths = new Map<string, string>()
  private outputLines = new Map<string, string[]>()
  private outputPartials = new Map<string, string>()
  private statusContexts = new Map<string, StatusContext>()
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
  createPty(payload: CreateTerminalPayload, reuseId?: string): TerminalSession {
    // Refused rather than created: a session started on an endpoint this process
    // no longer holds is reachable through a name that now points elsewhere, so
    // nobody would ever see it. Existing sessions are untouched -- their clients
    // hold a descriptor, not a name.
    if (isDraining()) throw new Error(DRAINING_MESSAGE)
    const id = reuseId ?? crypto.randomUUID()
    const shell = getDefaultShell(configManager.loadConfig().defaults.shell)

    // Check if this is a remote session
    const remoteHost = payload.remoteHostId
      ? this.remoteHosts.find((h) => h.id === payload.remoteHostId)
      : undefined

    const session = remoteHost
      ? this.createRemotePty(id, shell, payload, remoteHost)
      : this.createLocalPty(id, shell, payload)

    this.emit('session-created', session, payload)
    return session
  }

  private createLocalPty(
    id: string,
    shell: string,
    payload: CreateTerminalPayload
  ): TerminalSession {
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
      if (isGitRepo(payload.projectPath)) {
        if (payload.existingWorktreePath) {
          log.warn(
            `[pty] worktree path no longer exists, creating new: ${payload.existingWorktreePath}`
          )
        }
        const result = createWorktree(payload.projectPath, payload.branch, payload.worktreeName)
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
      if (isGitRepo(payload.projectPath)) {
        const currentBranch = getGitBranch(payload.projectPath)
        if (currentBranch !== payload.branch) {
          checkoutBranch(payload.projectPath, payload.branch)
        }
        effectiveBranch = payload.branch
      }
    }

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

    // Session ID pinning: agents that support it (supportsSessionIdPinning) get a
    // UUID assigned on fresh launch via --session-id, enabling exact --resume later.
    // Other agents rely on history-based fallback for resume.
    let agentSessionId: string | undefined
    if (supportsSessionIdPinning(payload.agentType)) {
      if (payload.resumeSessionId) {
        agentSessionId = payload.resumeSessionId
      } else {
        agentSessionId = crypto.randomUUID()
        payload.sessionId = agentSessionId
      }
    }

    const launchLine = this.buildAgentLaunchLine(payload)
    setTimeout(() => ptyProcess.write(launchLine + '\r'), 300)

    this.setupPtyEvents(id, ptyProcess, INITIAL_COLS, INITIAL_ROWS)
    this.ptys.set(id, ptyProcess)

    const branch = effectiveBranch || getGitBranch(effectivePath)
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
    const agentLine = this.buildAgentLaunchLine(payload)
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

  private static readonly BUFFER_FLUSH_MS = 8

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
    this.dataBuffers.set(id, existing ? existing + data : data)

    if (!this.flushTimers.has(id)) {
      this.flushTimers.set(
        id,
        setTimeout(() => this.flushBuffer(id), PtyManager.BUFFER_FLUSH_MS)
      )
    }
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

  private flushBuffer(id: string): void {
    const data = this.dataBuffers.get(id)
    this.dataBuffers.delete(id)
    this.flushTimers.delete(id)
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
      appendScrollback(id, data)
      feedScreen(id, data)
      recordOutput(id, data)
    }
  }

  private clearBuffer(id: string): void {
    const timer = this.flushTimers.get(id)
    if (timer) clearTimeout(timer)
    this.flushTimers.delete(id)
    this.dataBuffers.delete(id)
  }

  /** Push what is buffered now, without waiting out the timer that would. */
  private drainBuffer(id: string): void {
    const timer = this.flushTimers.get(id)
    if (timer) clearTimeout(timer)
    this.flushBuffer(id)
  }

  private clearSessionTracking(id: string): void {
    this.outputLines.delete(id)
    this.outputPartials.delete(id)
    this.statusContexts.delete(id)
    const idleTimer = this.idleTimers.get(id)
    if (idleTimer) clearTimeout(idleTimer)
    this.idleTimers.delete(id)
  }

  private appendOutput(id: string, data: string): void {
    const session = this.sessions.get(id)
    if (!session) return

    // Plain shells don't run agents — skip bracketed-paste / pattern / idle analysis.
    // They stay 'running' until the PTY exits (setupPtyEvents sets 'idle').
    if (session.agentType === 'shell') return

    let buf = this.outputLines.get(id)
    if (!buf) {
      buf = []
      this.outputLines.set(id, buf)
    }

    const clean = stripAnsi(data)
    const partial = this.outputPartials.get(id) ?? ''
    const combined = partial + clean
    const segments = combined.split('\n')

    // Last segment is incomplete (no trailing \n) — save for next chunk
    this.outputPartials.set(id, segments.pop()!)

    for (const line of segments) {
      buf.push(line)
    }
    if (buf.length > MAX_OUTPUT_LINES) {
      buf.splice(0, buf.length - MAX_OUTPUT_LINES)
    }

    // Bracketed paste mode detection — works for all agents using readline.
    // Programs enable \x1b[?2004h when ready for input, disable with 'l' when executing.
    const hasBracketedOn = BRACKETED_PASTE_ON.test(data)
    const hasBracketedOff = BRACKETED_PASTE_OFF.test(data)

    if (hasBracketedOn || hasBracketedOff) {
      // Use the last signal in the chunk (a chunk may contain both off then on)
      const lastOn = data.lastIndexOf('\x1b[?2004h')
      const lastOff = data.lastIndexOf('\x1b[?2004l')
      const newStatus = lastOn > lastOff ? 'waiting' : 'running'
      if (newStatus !== session.status) {
        this.updateSessionStatus(id, newStatus as AgentStatus)
      }
    } else if (session.statusSource !== 'hooks') {
      // Pattern-based fallback for non-hook sessions without bracketed paste
      let ctx = this.statusContexts.get(id)
      if (!ctx) {
        ctx = createStatusContext()
        this.statusContexts.set(id, ctx)
      }
      const newStatus = analyzeOutput(ctx, clean)
      if (newStatus !== session.status) {
        this.updateSessionStatus(id, newStatus)
      }
    }

    // Idle timer — if no output arrives within timeout, mark idle.
    // Hook sessions use a longer timeout as safety net (hooks are primary).
    const timeout = session.statusSource === 'hooks' ? IDLE_TIMEOUT_HOOKS_MS : IDLE_TIMEOUT_MS
    const existingTimer = this.idleTimers.get(id)
    if (existingTimer) clearTimeout(existingTimer)
    this.idleTimers.set(
      id,
      setTimeout(() => {
        this.idleTimers.delete(id)
        const s = this.sessions.get(id)
        if (s && s.status === 'running') {
          this.updateSessionStatus(id, 'idle')
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
  private setupPtyEvents(id: string, ptyProcess: pty.IPty, cols: number, rows: number): void {
    createScreen(id, cols, rows)
    // Replaces whatever was left under this id. A recovered session that is
    // being respawned has history describing a process that is gone.
    startHistory(id)

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
    void resizeScreen(id, cols, rows)
    recordResize(id, cols, rows)
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
    this.normalizedPaths.delete(id)
    this.clearSessionTracking(id)
    this.flushSeq.delete(id)
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
    // Copied because `flushBuffer` deletes from the map it is walking.
    for (const id of [...this.flushTimers.keys()]) this.drainBuffer(id)
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
    this.flushTimers.clear()
    this.flushSeq.clear()

    // Clean up any remaining temp key files
    for (const sessionId of this.tempKeyPaths.keys()) {
      this.deleteTempKey(sessionId)
    }

    for (const [id, p] of this.ptys) {
      p.kill()
      this.ptys.delete(id)
    }
    this.sessions.clear()
    this.outputLines.clear()
    this.outputPartials.clear()
    this.statusContexts.clear()
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

  getActiveSessions(): TerminalSession[] {
    if (this.sessionOrder.length === 0) {
      return Array.from(this.sessions.values())
    }
    const ordered: TerminalSession[] = []
    const seen = new Set<string>()
    for (const id of this.sessionOrder) {
      const s = this.sessions.get(id)
      if (s) {
        ordered.push(s)
        seen.add(id)
      }
    }
    for (const s of this.sessions.values()) {
      if (!seen.has(s.id)) ordered.push(s)
    }
    return ordered
  }

  updateSessionStatus(id: string, status: AgentStatus): void {
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
            this.updateSessionStatus(id, 'idle')
          }
        }, IDLE_TIMEOUT_HOOKS_MS)
      )
    }
  }

  renameSession(id: string, displayName: string): void {
    const session = this.sessions.get(id)
    if (!session) throw new Error(`Session not found: ${id}`)
    session.displayName = displayName
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
    const buf = this.outputLines.get(id) ?? []
    if (lines && lines < buf.length) {
      return buf.slice(-lines)
    }
    return [...buf]
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
