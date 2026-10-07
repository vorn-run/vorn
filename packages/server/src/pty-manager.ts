import crypto from 'node:crypto'
import os from 'node:os'
import fs from 'node:fs'
import path from 'node:path'
import { EventEmitter } from 'node:events'
import { holdWorkspace, setVorndHolds } from './workspace-holds'
import { HeadRefresh } from './head-commit'
import log from './logger'
import {
  AiAgentType,
  AgentStatus,
  AgentCommandConfig,
  CreateTerminalPayload,
  IPC,
  TerminalSession,
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
import { NATIVE_STATUS } from './native-core'
import { isDraining, DRAINING_MESSAGE } from './draining'
import { isHandingOver, HANDOVER_MESSAGE } from './handoff/donor'
import {
  vorndSessions,
  VorndPty,
  type HeldSession,
  type MirroredHow,
  type SessionNote,
  type VorndExit
} from './vornd-sessions'
import { applyWorktreeUpdates, type WorktreeUpdates } from './worktree-moves'
import { sessionFeed, type Stamp } from './session-feed'

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
/** The terminal type programs are told they run in. */
const PTY_TERM = 'xterm-256color'
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

/**
 * The fields of a terminal's record that vornd keeps while it decides the
 * statuses, whatever this server's upserts say: set here only through a patch
 * (`setRecordFields`), and taken from what vornd tells.
 */
const PATCHED_FIELDS = ['displayName', 'renamedByPerson', 'groupId', 'agentSessionId'] as const

type PatchedKey = (typeof PATCHED_FIELDS)[number]

/** Sets one of `PATCHED_FIELDS` on a record held here; null or undefined takes it away. */
function setPatched(session: TerminalSession, key: PatchedKey, value: unknown): void {
  if (value === null || value === undefined) delete session[key]
  else Object.assign(session, { [key]: value })
}

type PatchedFields = Partial<{
  displayName: string
  renamedByPerson: boolean
  groupId: string | null
  agentSessionId: string
}>

type WorktreeSessionCounter = (
  worktreePath: string,
  excludeId?: string
) => { count: number; sessionIds: string[] }

class PtyManager extends EventEmitter {
  /** Recorded HEAD per session, refreshed by the save loop. */
  readonly heads = new HeadRefresh(getGitHead, undefined, (s) => this.recordChanged(s.id))
  private ptys = new Map<string, VorndPty>()
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
  private tempKeyPaths = new Map<string, string>()
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
    sessionFeed.setTerminalSource({
      terminals: () => this.getActiveSessions(),
      order: () => this.sessionOrder,
      ended: () => this.getActiveSessions().flatMap((s) => (this.ptys.has(s.id) ? [] : [s.id]))
    })
    vorndSessions.on('mirrored', (record: TerminalSession, how: MirroredHow) =>
      this.fromMirror(record, how)
    )
    vorndSessions.on('native', (note: SessionNote) => this.fromVornd(note))
    vorndSessions.on('ask', (method: string, params: unknown) => {
      // The last terminal in a worktree vornd closed: offered as `killPty` offers it.
      if (method === 'vornd:cleanupOffer' && vorndSessions.createsTerminals())
        this.emit('client-message', IPC.WORKTREE_CONFIRM_CLEANUP, params)
    })
  }

  /**
   * Tell vornd's copy of the registry what this record is now (`session-feed`).
   *
   * Called after every change to a record: here, and by the places outside
   * that change one in place (a hook linking it, an agent's id captured, a
   * resume carrying fields over). Telling it twice costs nothing. An
   * extension's pane is not a session and is never told.
   */
  recordChanged(id: string): void {
    const session = this.sessions.get(id)
    if (!session || this.extensionPtys.has(id)) return
    sessionFeed.terminal(session, !this.ptys.has(id))
  }

  /**
   * A terminal record as vornd's copy holds it, while vornd decides the
   * statuses (`vorndSessions.decidesStatus`): what it decided is taken here,
   * and a status that changed is broadcast as `setStatus` did. The copy's
   * changes arrive in the order it made them, so the record is always the
   * newer word. A session whose program ended keeps the status it ended with:
   * a change told before vornd heard of the end is older than it.
   *
   * The fields only vornd sets while it decides (`PATCHED_FIELDS`) are taken
   * from a snapshot and from a change vornd made for a client (a rename, a
   * group), ended or not, and told as `renameSession` and `setSessionGroup` tell
   * them. Changes this server asked for itself it has already made.
   */
  private fromMirror(record: TerminalSession, how: MirroredHow = 'note'): void {
    if (!vorndSessions.decidesStatus()) return
    const session = this.sessions.get(record.id)
    if (!session || this.extensionPtys.has(record.id)) return
    let updated = false
    let linked = false
    let renamed = false
    if (how !== 'note') {
      for (const key of PATCHED_FIELDS) {
        if (record[key] === session[key]) continue
        setPatched(session, key, record[key])
        if (key === 'agentSessionId') linked = true
        else updated = true
        if (key === 'displayName') renamed = true
      }
    }
    if (this.ptys.has(record.id)) {
      if (record.hookSessionId !== session.hookSessionId) {
        if (record.hookSessionId) session.hookSessionId = record.hookSessionId
        else delete session.hookSessionId
        linked = true
      }
      if (record.statusSource !== session.statusSource) {
        if (record.statusSource) session.statusSource = record.statusSource
        else delete session.statusSource
        linked = true
      }
      if (record.status !== session.status) {
        session.status = record.status
        updated = true
      }
    }
    if (updated) {
      this.recordChanged(record.id)
      this.emit('client-message', IPC.SESSION_UPDATED, session)
    } else if (linked) {
      this.recordChanged(record.id)
    }
    if (how === 'note') return
    if (renamed && session.displayName !== undefined)
      this.emit('session-renamed', session.id, session.displayName)
    if (updated || linked) this.emit('records-changed')
  }

  /**
   * A change vornd made itself, for a client's call: a terminal it created,
   * whose program started or could not, one it closed, the order a client set,
   * and the workspaces it holds while it prepares a session. Followed as this
   * server's own calls are, so clients are told and the records saved alike.
   */
  private fromVornd(note: SessionNote): void {
    if (note.op === 'holds') {
      setVorndHolds(note.nativeHolds ?? {})
      return
    }
    if (!vorndSessions.createsTerminals()) return
    if (note.op === 'upsert' && note.kind === 'terminal' && note.record) {
      const id = note.record.id
      // One the holder still held from the last run is taken on with its
      // states, from the subscription that lists it (`takeOnHeld`).
      if (note.created && note.resumed) this.adoptResumed(note.record as TerminalSession)
      else if (note.created && !note.adopted) this.adoptCreated(note.record as TerminalSession)
      if (note.started) this.ptys.get(id)?.started(note.started.pid, note.started.epoch)
      if (note.failed !== undefined) {
        log.warn({ id, why: note.failed }, '[pty] vornd could not start this session')
        this.ptys.get(id)?.finish(1)
      }
      const moved = note.moved ? this.sessions.get(id) : undefined
      if (moved) this.worktreeMoved(moved, note.record)
    } else if (note.op === 'remove' && note.kind === 'terminal' && note.id) {
      if (note.released) this.letGo(note.id)
      else this.closedByVornd(note.id)
    } else if (note.op === 'order' && note.reordered && note.order) {
      this.sessionOrder = [...note.order]
      this.orderChanged()
      this.emit('client-message', IPC.SESSION_REORDERED, [...note.order])
      this.emit('records-changed')
    }
  }

  /**
   * A terminal vornd created for a client, taken on as `spawnPty` takes on one
   * this server starts: its program is followed, its record held and listed
   * last, and `session-created` runs what hangs off a new session.
   */
  private adoptCreated(record: TerminalSession): void {
    if (this.sessions.has(record.id)) return
    // vornd's revision and stamps are its copy's, not this server's record.
    const session: TerminalSession & { statusAt?: unknown; exitAt?: unknown } = { ...record }
    delete session.rev
    delete session.statusAt
    delete session.exitAt
    const id = session.id
    const program = vorndSessions.follow(id)
    this.setupVorndEvents(id, program)
    this.ptys.set(id, program)
    this.sessions.set(id, session)
    if (!this.sessionOrder.includes(id)) this.sessionOrder.push(id)
    this.normalizedPaths.set(id, normalizePath(session.worktreePath || session.projectPath))
    this.recordChanged(id)
    this.orderChanged()
    const payload = {
      agentType: session.agentType,
      projectName: session.projectName,
      projectPath: session.projectPath
    } as CreateTerminalPayload
    this.emit('session-created', session, payload)
  }

  /**
   * A terminal vornd started again under the id it had, for a client's
   * `sessions:resume`: the record it replaces goes without a word, as
   * `releaseForResume` lets one go, and the new one is taken on as a terminal
   * vornd created. Nothing is told the copy: it made the change.
   */
  private adoptResumed(record: TerminalSession): void {
    this.letGo(record.id)
    this.adoptCreated(record)
  }

  /**
   * Let go of a record whose session starts again or runs elsewhere: the maps
   * only, without an exit or a word to the copy, which let go of it first.
   */
  private letGo(id: string): void {
    this.sessions.delete(id)
    this.heads.forget(id)
    this.normalizedPaths.delete(id)
    this.clearSessionTracking(id)
    this.sessionOrder = this.sessionOrder.filter((sid) => sid !== id)
    if (this.ptys.delete(id)) vorndSessions.release(id)
  }

  /**
   * A terminal vornd closed for a client: let go of here as `killPty` lets go of
   * one, but vornd hangs its program up and offers to clean up its worktree.
   * Its program is still followed until it ends, which tells clients as the
   * exit of one `killPty` hung up does.
   */
  private closedByVornd(id: string): void {
    const live = this.ptys.has(id)
    const session = this.sessions.get(id)
    this.sessions.delete(id)
    this.heads.forget(id)
    this.normalizedPaths.delete(id)
    this.clearSessionTracking(id)
    this.sessionOrder = this.sessionOrder.filter((sid) => sid !== id)
    this.ptys.delete(id)
    this.recordRemoved(id)
    this.orderChanged()
    if (session) this.emit('session-exit', session)
    if (!live) this.emit('client-message', IPC.TERMINAL_EXIT, { id, exitCode: 0 })
  }

  /** `sessionOrder` changed. */
  private orderChanged(): void {
    sessionFeed.order(this.sessionOrder)
  }

  /** A record was let go of. */
  private recordRemoved(id: string): void {
    sessionFeed.remove('terminal', id)
  }

  /**
   * Keep a shell's record pointing at where the shell actually is.
   *
   * `shellCwd` was written once at spawn and never moved, so restoring a shell
   * put somebody back where they started rather than where they were. This is
   * the record that gets persisted and the one a restored shell is offered.
   *
   * Told by vornd as the shell reports it, so it stays a map lookup and an
   * event, and the save it triggers is debounced -- a script running `cd` in a loop costs one write
   * rather than hundreds.
   */
  private noteShellCwd(id: string, cwd: string): void {
    const session = this.sessions.get(id)
    if (!session || session.agentType !== 'shell' || session.shellCwd === cwd) return
    session.shellCwd = cwd
    this.recordChanged(id)
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

      this.recordChanged(session.id)
      this.orderChanged()
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
    const ptyProcess = this.startProcess(id, shell, getShellArgs(), {
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
    // Read here as well as shown: the login is answered from what it prints.
    const ptyProcess = this.startProcess(
      id,
      shell,
      getShellArgs(),
      { cwd: os.homedir(), env: getSafeEnv() },
      true
    )

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
    const ptyProcess = this.startProcess(id, shell, integration.args ?? getShellArgs(), {
      cwd: workingDir,
      env: {
        ...getSafeEnv(),
        ...integration.env,
        // Spawn-site only — see the note in createLocalPty.
        VORN_SESSION_ID: id
      }
    })
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
    this.recordChanged(id)
    this.orderChanged()
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
    const ptyProcess = this.startProcess(id, params.command, params.args, {
      cwd: params.cwd,
      env: { ...getSafeEnv(), ...params.env, VORN_SESSION_ID: id }
    })
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

  /**
   * Start a session's program in vornd, which keeps it in its session holder so
   * it outlives this server.
   *
   * @param watched Whether this server reads the session's output. Only what
   *   answers from the output needs to: a remote login.
   */
  private startProcess(
    id: string,
    file: string,
    args: string[] | string,
    opts: { cwd: string; env: Record<string, string> },
    watched = false
  ): VorndPty {
    const argv = [file, ...(typeof args === 'string' ? [args] : args)]
    // vornd starts the program with exactly this environment, so it carries
    // the terminal type programs are told they run in.
    const env = process.platform === 'win32' ? opts.env : { ...opts.env, TERM: PTY_TERM }
    const spec = { argv, cwd: opts.cwd, env, cols: INITIAL_COLS, rows: INITIAL_ROWS }
    const program = vorndSessions.spawn(id, spec, watched)
    this.setupVorndEvents(id, program)
    return program
  }

  /** The status vornd last reported for each of its sessions. */
  private vorndStatus = new Map<string, AgentStatus>()

  /**
   * A session in vornd. Its output never comes here: vornd keeps its screen
   * and history and serves its clients, and says what the output meant.
   */
  private setupVorndEvents(id: string, held: VorndPty): void {
    held.on('started', (pid: number) => {
      const session = this.sessions.get(id)
      if (session) session.pid = pid
      this.recordChanged(id)
    })
    held.on('status', (code: number, note?: Stamp) => {
      // vornd's copy decides it, and says so in its changes (`fromMirror`).
      if (vorndSessions.decidesStatus()) return
      const session = this.sessions.get(id)
      const status = NATIVE_STATUS[code]
      if (!session || !status) return
      this.vorndStatus.set(id, status)
      if (session.agentType === 'shell' || session.statusSource === 'hooks') return
      this.setStatus(
        id,
        status,
        note ? { epoch: note.epoch, rseq: note.rseq, index: note.index } : null
      )
    })
    held.on('cwd', (cwd: string) => this.noteShellCwd(id, cwd))
    held.on('activity', () => {
      if (vorndSessions.decidesStatus()) return
      const session = this.sessions.get(id)
      if (!session || session.agentType === 'shell') return
      // Printing again after going idle, with nothing new to say: running.
      if (
        session.status === 'idle' &&
        session.statusSource !== 'hooks' &&
        this.vorndStatus.get(id) === 'running'
      ) {
        this.setStatus(id, 'running')
      }
      this.armIdle(id, session)
    })
    held.onExit((exit) => this.processEnded(id, exit, held.exitAt ?? null))
  }

  /**
   * Take on a terminal vornd still holds from this server's previous run,
   * under the record that run saved.
   */
  adoptVornd(session: TerminalSession, held: HeldSession): void {
    const watched = !!session.remoteHostId
    session.pid = held.pid
    session.status = 'running'
    this.sessions.set(session.id, session)
    if (!this.sessionOrder.includes(session.id)) this.sessionOrder.push(session.id)
    this.normalizedPaths.set(session.id, normalizePath(session.worktreePath || session.projectPath))
    const program = vorndSessions.adopt(held, watched, (p) => this.setupVorndEvents(session.id, p))
    this.ptys.set(session.id, program)
    this.recordChanged(session.id)
    this.orderChanged()
  }

  /** Whether this PTY is an extension's pane rather than a session someone started. */
  isExtensionPty(id: string): boolean {
    return this.extensionPtys.has(id)
  }

  private clearSessionTracking(id: string): void {
    this.extensionPtys.delete(id)
    this.vorndStatus.delete(id)
    const idleTimer = this.idleTimers.get(id)
    if (idleTimer) clearTimeout(idleTimer)
    this.idleTimers.delete(id)
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

  /** A session's program ended. */
  private processEnded(id: string, { exitCode, repeated }: VorndExit, exitAt: Stamp | null): void {
    this.deleteTempKey(id)
    this.clearSessionTracking(id)
    this.sessionOrder = this.sessionOrder.filter((sid) => sid !== id)
    this.orderChanged()

    this.ptys.delete(id)
    this.vorndStatus.delete(id)
    const session = this.sessions.get(id)
    if (session) {
      this.emit('session-exit', session)
      session.status = 'idle'
      if (session.agentType === 'shell') {
        session.shellExitCode = exitCode
      }
      sessionFeed.statusAt(id, null)
      sessionFeed.exitAt('terminal', id, exitAt)
      this.recordChanged(id)
      // An exit told again, after vornd restarted, was acted on the first time.
      if (session.worktreePath && !repeated) {
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
    if (!repeated) this.emit('client-message', IPC.TERMINAL_EXIT, { id, exitCode })
  }

  writeToPty(id: string, data: string): void {
    // For non-hook sessions, user input means the session is active.
    // Hook sessions rely on hooks to transition to running (e.g. PreToolUse).
    const session = this.sessions.get(id)
    const wakes =
      !!session &&
      session.statusSource !== 'hooks' &&
      (session.status === 'idle' || session.status === 'waiting')
    // Told before the write, on the same channel, so vornd has it running
    // before anything the write makes the program print.
    const decides = wakes && vorndSessions.decidesStatus()
    if (decides) vorndSessions.input(id)
    this.ptys.get(id)?.write(data)
    if (wakes && !decides) this.updateSessionStatus(id, 'running')
  }

  /**
   * Note the size a client fitted a session to.
   *
   * vornd decides a session's size from the clients watching it and answers
   * their resizes itself; this only keeps the session record's numbers, guarded
   * because it arrives as a fire-and-forget notification.
   */
  resizePty(id: string, cols: number, rows: number): void {
    const session = this.sessions.get(id)
    if (!session) return
    if (!Number.isInteger(cols) || !Number.isInteger(rows) || cols <= 0 || rows <= 0) return
    if (cols > MAX_GEOMETRY || rows > MAX_GEOMETRY) return
    session.cols = cols
    session.rows = rows
    this.recordChanged(id)
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
   * So this releases the id and says nothing: the maps, which `createPty` is
   * about to replace anyway.
   */
  releaseForResume(id: string): void {
    this.sessions.delete(id)
    this.heads.forget(id)
    this.normalizedPaths.delete(id)
    this.clearSessionTracking(id)
    this.sessionOrder = this.sessionOrder.filter((sid) => sid !== id)
    this.ptys.delete(id)
    this.recordRemoved(id)
    this.orderChanged()
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
    this.recordChanged(session.id)
    this.orderChanged()
  }

  killPty(id: string): void {
    const p = this.ptys.get(id)

    // Delete session and PTY from maps BEFORE killing so the exit handler
    // (setupVorndEvents) won't find them and emit a duplicate 'session-exit'.
    // Delete-then-check pattern: single removal point prevents races.
    const session = this.sessions.get(id)
    this.sessions.delete(id)
    this.heads.forget(id)
    this.normalizedPaths.delete(id)
    this.clearSessionTracking(id)
    this.sessionOrder = this.sessionOrder.filter((sid) => sid !== id)
    this.ptys.delete(id)
    this.recordRemoved(id)
    this.orderChanged()

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
      // Defer the actual kill so the IPC response returns immediately: all
      // state cleanup is already done above.
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

  /** Let go of every session for a server on its way out: vornd keeps them running. */
  killAll(): void {
    for (const sessionId of this.tempKeyPaths.keys()) {
      this.deleteTempKey(sessionId)
    }
    for (const id of this.ptys.keys()) vorndSessions.release(id)
    this.ptys.clear()
    this.vorndStatus.clear()
    // Not vornd's records to let go of while it keeps them for the next server.
    if (!vorndSessions.restoresSessions()) {
      for (const id of this.sessions.keys()) this.recordRemoved(id)
    }
    this.sessions.clear()
    for (const timer of this.idleTimers.values()) clearTimeout(timer)
    this.idleTimers.clear()
    this.sessionOrder = []
    this.orderChanged()
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

  /** A status from outside the output (a hook, a permission request). */
  updateSessionStatus(id: string, status: AgentStatus): void {
    if (vorndSessions.decidesStatus()) vorndSessions.hookStatus(id, status, false)
    else this.setStatus(id, status)
  }

  /**
   * What an agent's hook said: its status, if it named one, and then the
   * session's status taken from its hooks from now on (`promoteToHookStatus`).
   * While vornd decides the statuses both go to it as one call, applied in
   * that order, and come back as its changes.
   */
  hookStatus(id: string, status: AgentStatus | null, promote: boolean): void {
    if (vorndSessions.decidesStatus()) {
      if (this.sessions.has(id)) vorndSessions.hookStatus(id, status, promote)
      return
    }
    if (status) this.setStatus(id, status)
    if (promote) this.promoteToHookStatus(id)
  }

  /**
   * Link a session to the conversation an agent's hooks name it by. While
   * vornd decides the statuses the link is its to set, and arrives back with
   * its changes.
   */
  linkHookSession(id: string, hookSessionId: string): void {
    const session = this.sessions.get(id)
    if (!session) return
    if (vorndSessions.decidesStatus()) {
      vorndSessions.patch(id, { hookSessionId })
      return
    }
    session.hookSessionId = hookSessionId
    this.recordChanged(id)
  }

  /** @param at The effect that told it, when vornd did; null for a hook, a timer or input. */
  private setStatus(id: string, status: AgentStatus, at: Stamp | null = null): void {
    const session = this.sessions.get(id)
    if (session && session.status !== status) {
      session.status = status
      sessionFeed.statusAt(id, at)
      this.recordChanged(id)
      this.emit('client-message', IPC.SESSION_UPDATED, session)
    }
  }

  /** Promote a session to hook-based status detection (disables pattern fallback). */
  promoteToHookStatus(id: string): void {
    const session = this.sessions.get(id)
    if (!session) return
    if (vorndSessions.decidesStatus()) {
      vorndSessions.hookStatus(id, null, true)
      return
    }

    if (session.statusSource !== 'hooks') {
      session.statusSource = 'hooks'
      this.recordChanged(id)
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

  /**
   * Set fields of a terminal's record that vornd keeps while it decides the
   * statuses (`PATCHED_FIELDS`); null or undefined takes one away. Set here as
   * any change is, and while vornd decides also told to it as a patch, without
   * which its copy would keep what it had.
   */
  setRecordFields(id: string, fields: PatchedFields): void {
    const session = this.sessions.get(id)
    if (!session) return
    const keys = PATCHED_FIELDS.filter((key) => key in fields)
    for (const key of keys) setPatched(session, key, fields[key])
    this.recordChanged(id)
    if (vorndSessions.decidesStatus()) this.tellPatched(id, keys)
  }

  /**
   * Tell vornd the fields of a record it keeps (`PATCHED_FIELDS`), as they
   * are here: for a record whose fields were set in place, as a resume sets
   * those it carries over.
   */
  tellPatched(id: string, keys: readonly PatchedKey[] = PATCHED_FIELDS): void {
    const session = this.sessions.get(id)
    if (!session || !vorndSessions.decidesStatus()) return
    const fields: Partial<Record<PatchedKey, string | boolean | null>> = {}
    for (const key of keys) fields[key] = session[key] ?? null
    vorndSessions.patch(id, fields)
  }

  /** `byPerson` records who chose the name, which is what an extension may not overrule. */
  renameSession(id: string, displayName: string, byPerson = true): void {
    const session = this.sessions.get(id)
    if (!session) throw new Error(`Session not found: ${id}`)
    this.setRecordFields(id, { displayName, ...(byPerson ? { renamedByPerson: true } : {}) })
    this.emit('client-message', IPC.SESSION_UPDATED, session)
  }

  /** Files a session under a group, or takes it out of one with null. */
  setSessionGroup(id: string, groupId: string | null): void {
    const session = this.sessions.get(id)
    if (!session) throw new Error(`Session not found: ${id}`)
    this.setRecordFields(id, { groupId: groupId || null })
    this.emit('client-message', IPC.SESSION_UPDATED, session)
  }

  reorderSessions(ids: string[]): void {
    if (new Set(ids).size !== ids.length) throw new Error('Duplicate session IDs')
    for (const id of ids) {
      if (!this.sessions.has(id)) throw new Error(`Session not found: ${id}`)
    }
    this.sessionOrder = ids
    this.orderChanged()
    this.emit('client-message', IPC.SESSION_REORDERED, ids)
  }

  /** The last `lines` lines a session printed, from vornd's model of its screen. */
  async readOutput(id: string, lines?: number): Promise<string[]> {
    if (!this.sessions.has(id)) throw new Error(`Session not found: ${id}`)
    return vorndSessions.readOutput(id, lines)
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

  updateSessionsForWorktree(worktreePath: string, updates: WorktreeUpdates): void {
    for (const s of this.sessions.values()) {
      if (s.worktreePath === worktreePath) this.worktreeMoved(s, updates)
    }
  }

  private worktreeMoved(s: TerminalSession, updates: WorktreeUpdates): void {
    applyWorktreeUpdates(s, updates)
    this.heads.invalidate(s.id)
    this.recordChanged(s.id)
    this.emit('client-message', IPC.SESSION_UPDATED, s)
  }

  /**
   * The terminals matching cwd, most recently created first, that:
   * - are NOT already linked to a Claude session (no hookSessionId)
   * - are NOT in the excludeIds set (already claimed by another session_id)
   *
   * All of them rather than the newest, so a caller can tell a guess between
   * several from a single match.
   */
  findUnlinkedSessionsByCwd(cwd: string, excludeIds: Set<string>): TerminalSession[] {
    const normalizedCwd = normalizePath(cwd)
    const found: TerminalSession[] = []

    for (const session of this.sessions.values()) {
      if (session.hookSessionId) continue // already linked
      if (excludeIds.has(session.id)) continue
      const sessionPath =
        this.normalizedPaths.get(session.id) ??
        normalizePath(session.worktreePath || session.projectPath)
      if (sessionPath === normalizedCwd) found.push(session)
    }

    return found.sort((a, b) => b.createdAt - a.createdAt)
  }
}

export const ptyManager = new PtyManager()
