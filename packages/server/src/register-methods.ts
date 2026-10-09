import fs from 'fs'
import crypto from 'node:crypto'
import { registerMethod, registerNotification } from './ws-handler'
import { ptyManager } from './pty-manager'
import { headlessManager } from './headless-manager'
import { sessionManager } from './session-persistence'
import { clientRegistry } from './broadcast'
import {
  restoredRecords,
  listRestored,
  consumeRestored,
  consumeAllRestored,
  restoreHeld
} from './restored-sessions'
import { buildRestorePayload } from '@vornrun/shared/session-restore'
import { resumeCwdFor } from './resume-cwd'
import {
  freeTranscriptFor,
  sessionToBindOnCreate,
  transcriptScope,
  transcriptHolder,
  transcriptNamedOnCreate
} from './agent-transcript'
import {
  holdClaimsWhilePreparing,
  releaseSpawningTranscript,
  releaseSpawningTranscriptsFor
} from './transcript-claims'
import { IPC, SessionEventType } from '@vornrun/shared/types'
import type { TerminalSession } from '@vornrun/shared/types'
import { listShellExecutables } from './shell-integration'
import { listInstalledShells } from './shell-integration/installed'
import { insertSessionEvent } from './database'
import { getTailscaleStatus } from './tailscale'
import { reachableUrls } from './reachable-urls'
import log from './logger'
import { vorndSessions } from './vornd-sessions'
import { wireVorndRestore } from './vornd-restore'
import { onePerKey } from './one-per-key'

function logSessionEvent(
  sessionId: string,
  eventType: SessionEventType,
  metadata?: Record<string, unknown>
): void {
  try {
    insertSessionEvent({
      sessionId,
      eventType,
      timestamp: new Date().toISOString(),
      ...(metadata ? { metadata } : {})
    })
  } catch (err) {
    log.error({ err }, '[session-events] failed to log event:')
  }
}

let serverPort = 0
export function setServerPort(port: number): void {
  serverPort = port
}

/** Say a session exists; vornd settles what the extensions show on it from the records. */
export function announceSession(session: TerminalSession): void {
  clientRegistry.broadcast(IPC.SESSION_CREATED, session)
}

/** Whether a path is a directory right now, answering false for every other case. */
function isDirectory(at: string): boolean {
  try {
    return fs.statSync(at).isDirectory()
  } catch {
    return false
  }
}

/** Named because a rolled-back handoff has to start saving again, with exactly this. */
export function sessionsToPersist(): TerminalSession[] {
  const active = ptyManager.getActiveSessions()
  ptyManager.heads.refresh(active)
  return [...active, ...restoredRecords()]
}

/** Creates that name a conversation, by its id, while they prepare. */
const createNamed = onePerKey<TerminalSession>()

/** Resumes between claiming their conversation and spawning, by session id. */
const resuming = new Map<string, Promise<TerminalSession | undefined>>()

/**
 * The conversations being started are claimed in vornd while it creates
 * terminals for the clients, so its creates and this server's own starts check
 * one set of claims; here otherwise. A claim made here is let go of in both.
 */
async function claimInVornd(transcriptId: string, id: string): Promise<string | undefined> {
  return vorndSessions.claim(transcriptId, id)
}

function releaseClaim(transcriptId: string, id: string): void {
  releaseSpawningTranscript(transcriptId, id)
  vorndSessions.unclaim(id, transcriptId)
}

function releaseClaimsFor(id: string): void {
  releaseSpawningTranscriptsFor(id)
  vorndSessions.unclaim(id)
}

/** `holdClaimsWhilePreparing`, in vornd too while it holds the claims. */
function holdClaims(id: string): () => void {
  const prepared = holdClaimsWhilePreparing(id)
  vorndSessions.preparing(id)
  let done = false
  return () => {
    if (done) return
    done = true
    prepared()
    vorndSessions.prepared(id)
  }
}

export function registerAllMethods(): void {
  // Wire headless worktree counter into pty-manager for cleanup gating
  ptyManager.setHeadlessWorktreeCounter((worktreePath, excludeId) =>
    headlessManager.getActiveSessionsForWorktree(worktreePath, excludeId)
  )

  // Terminal
  registerMethod('terminal:create', async (payload) => {
    const named = transcriptNamedOnCreate(payload.agentType, payload.resumeSessionId)
    // Naming a conversation that is already running: show what is writing it
    // rather than starting a second agent on it, as a resume does.
    const running = sessionToBindOnCreate(named, ptyManager.getLiveSessions())
    if (running) return running
    if (!named) return ptyManager.createPty(payload)
    // Preparing awaits git, and may create a worktree or check out a branch. A
    // second create for the same conversation in that window gets the first
    // one's session, rather than preparing a workspace of its own to discard.
    return createNamed(named, async () => {
      // Claimed before preparing, under the id the session will have, so a
      // resume of the same conversation sees it in flight and chooses another.
      const id = crypto.randomUUID()
      const holder = await claimInVornd(named, id)
      if (holder !== undefined) {
        // A resume got there first: wait for it, then show what it started.
        await resuming.get(holder)
        const live = ptyManager.getLiveSessions()
        const bound =
          sessionToBindOnCreate(named, live) ?? live.find((session) => session.id === holder)
        if (bound) return bound
        const again = await claimInVornd(named, id)
        if (again !== undefined) {
          throw new Error('This conversation is already starting in another pane')
        }
      }
      const prepared = holdClaims(id)
      try {
        const session = ptyManager.spawnPty(payload, await ptyManager.prepareSession(payload), id)
        prepared()
        // An agent that was told the id names the conversation itself; one that
        // cannot be keeps the claim until it reports, seconds later.
        if (session.agentSessionId) releaseClaim(named, id)
        return session
      } catch (err) {
        prepared()
        releaseClaim(named, id)
        throw err
      }
    })
  })
  /**
   * Let go of what was kept for a session from the last run.
   *
   * The screen the server rebuilt and the files it rebuilt it from. Both go
   * together, whether the record was claimed or declined -- a live session opens
   * its own history rather than appending to a record of the one it replaced,
   * and a declined one is not coming back.
   */

  registerMethod('terminal:kill', (id) => {
    // A pane showing a session from the last run has no PTY to kill. Closing it
    // is a decision about the record and the files, and it is the same decision
    // resume makes -- so it goes through the same door, and a second client
    // closing the same pane finds nothing rather than an error.
    if (consumeRestored(id)) {
      // The row is only removed by a save, and saves are event-driven. Without
      // this, closing a restored pane and quitting leaves the record behind --
      // and the next start offers a session whose files have gone, as an empty
      // pane the person already closed.
      sessionManager.scheduleSave()
      return
    }
    ptyManager.killPty(id)
  })
  registerMethod('terminal:listActive', () => ptyManager.getActiveSessions())
  registerMethod('terminal:rename', ({ id, displayName }) => {
    // Asked for by a person, so an extension may not overrule it afterwards.
    ptyManager.renameSession(id, displayName, true)
    logSessionEvent(id, 'renamed', { displayName })
    sessionManager.scheduleSave()
  })
  registerMethod('terminal:setGroup', ({ id, groupId }) => {
    ptyManager.setSessionGroup(id, groupId)
    sessionManager.scheduleSave()
  })
  registerMethod('terminal:reorder', (ids) => {
    ptyManager.reorderSessions(ids)
    sessionManager.scheduleSave()
  })
  registerMethod('shell:create', (cwd) => {
    const session = ptyManager.createShellPty(cwd)
    announceSession(session)
    logSessionEvent(session.id, 'created', {
      agentType: session.agentType,
      projectName: session.projectName,
      projectPath: session.projectPath
    })
    sessionManager.scheduleSave()
    return session
  })

  // Sessions
  registerMethod('sessions:clear', () => {
    // The offer is being declined for all of them at once.
    consumeAllRestored()
  })

  registerMethod('sessions:restored', () => listRestored())

  registerMethod('sessions:resume', async ({ id }) => {
    // Claimed before anything is started, and that ordering is the point. Two
    // clients can be looking at the same cold pane; the second must be told it
    // is gone rather than launching a second agent against one transcript.
    // Two kinds of ended session, and only one of them is held here.
    //
    // A session carried over from a previous run is in `held`. One that exited
    // during *this* run is not: its record outlives its process in the pty
    // manager, which is what `hasLivePty` exists to tell apart. That second kind
    // is the common one -- an agent finishing its turn -- and it was answered
    // `gone`, which the pane reported as "resumed somewhere else" before
    // deleting itself and its scrollback.
    const restored = consumeRestored(id)
    const dead = restored
      ? undefined
      : ptyManager.getActiveSessions().find((s) => s.id === id && !ptyManager.hasLivePty(id))
    const previous = restored?.session ?? dead
    if (!previous) return { ok: false as const, reason: 'gone' as const }

    const live = ptyManager.getLiveSessions()
    let transcriptId: string | undefined
    let settleResume: ((session: TerminalSession | undefined) => void) | undefined
    let claimsPrepared: (() => void) | undefined
    const pinned = previous.agentSessionId
    const holder = pinned ? transcriptHolder(pinned, live) : undefined
    if (holder) {
      // Its conversation is already running; hand back what is writing it.
      if (dead) ptyManager.releaseForResume(id)
      sessionManager.scheduleSave()
      return { ok: true as const, session: holder, boundTo: holder.id }
    }

    try {
      // Claimed, for the second kind. Not `killPty`: that announces an exit for
      // a session which is coming straight back, offers to delete the worktree
      // this is about to resume into, and removes the history directory
      // `startHistory` resets moments later.
      if (dead) ptyManager.releaseForResume(id)

      if (previous.agentType === 'shell') {
        // The remembered directory only if it is still a directory. It was
        // reported by the shell over the tty, so anything that could write to
        // that pane could have written it -- and a resume is the one moment it
        // turns into where a process starts. A stale or fabricated path falls
        // back to the project rather than being spawned into.
        const landing = resumeCwdFor(previous, isDirectory)
        if (!landing)
          return {
            ok: false as const,
            reason: 'workspace-gone' as const,
            message: `${previous.projectPath} is gone`
          }
        const cwd = landing.cwd
        // The same id, which is what makes this the same pane rather than a new
        // one beside it: the client keys its terminal by this, so a fresh id
        // would hand back a blank shell and drop the screen being resumed. The
        // previous run's history is not deleted here either -- `startHistory`
        // resets it under this name, on the queue that owns it, and only once
        // something is actually running.
        const session = ptyManager.createShellPty(cwd, id)
        // Carried across on the server rather than grafted on by whichever
        // client asked. `createShellPty` names a session after its directory, so
        // without this a restored shell loses the project it belonged to -- which
        // it does today, for exactly this reason.
        Object.assign(session, {
          projectName: previous.projectName,
          projectPath: previous.projectPath,
          ...(previous.worktreePath !== undefined && { worktreePath: previous.worktreePath }),
          ...(previous.worktreeName !== undefined && { worktreeName: previous.worktreeName }),
          ...(previous.branch !== undefined && { branch: previous.branch }),
          ...(previous.isWorktree !== undefined && { isWorktree: previous.isWorktree }),
          ...(previous.displayName !== undefined && { displayName: previous.displayName }),
          // Same reason as the rest: rebuilt from a whitelist, so anything left
          // out is written back as null by the save below and gone for good.
          ...(previous.groupId !== undefined && { groupId: previous.groupId })
        })
        ptyManager.recordChanged(session.id)
        // vornd keeps a name and a group while it decides, whatever an upsert says.
        ptyManager.tellPatched(session.id)
        announceSession(session)
        sessionManager.scheduleSave()
        return { ok: true as const, session }
      }

      // Checked before anything is claimed: a worktree removed while the machine
      // was off used to be spawned into as though it were there.
      const landing = resumeCwdFor(previous, isDirectory)
      if (!landing)
        return {
          ok: false as const,
          reason: 'workspace-gone' as const,
          message: `${previous.projectPath} is gone`
        }
      // buildRestorePayload reads worktreePath for the cwd, so a gone worktree is cleared from the record.
      const worktreeGone =
        previous.worktreePath !== undefined && landing.cwd === previous.projectPath
      const grounded = worktreeGone
        ? { ...previous, worktreePath: undefined, isWorktree: false }
        : previous

      // Read before the claim, so the claim and what it is checked against are
      // one synchronous step: with native git this await lets other calls run.
      const scope = await transcriptScope(grounded)
      // Not lapsing while the workspace below is prepared, however long git takes.
      claimsPrepared = holdClaims(id)
      // Claimed in vornd, which its creates check: taken meanwhile, the agent chooses.
      const free = freeTranscriptFor(
        grounded,
        ptyManager.getLiveSessions(),
        headlessManager.getActiveSessions(),
        scope
      )
      transcriptId = free && (await claimInVornd(free, id)) === undefined ? free : undefined
      // A create naming this conversation while it prepares waits for this spawn.
      const spawned = new Promise<TerminalSession | undefined>(
        (resolve) => (settleResume = resolve)
      )
      resuming.set(id, spawned)
      void spawned.then(() => {
        if (resuming.get(id) === spawned) resuming.delete(id)
      })

      // Same id, same reasons as the shell branch above. The claim stands in for
      // the session while its workspace is prepared, as it does for any spawn.
      const payload = buildRestorePayload(grounded, transcriptId)
      const session = ptyManager.spawnPty(payload, await ptyManager.prepareSession(payload), id)
      claimsPrepared()
      settleResume?.(session)
      // Carried on the server rather than through the payload, so membership is
      // never something a client can set on a spawn.
      if (grounded.groupId !== undefined) {
        ptyManager.setRecordFields(session.id, { groupId: grounded.groupId })
      }
      // The record names the conversation now, so the claim standing in for it is
      // spent; leaving it would hold an id the session already reports.
      if (session.agentSessionId) releaseClaimsFor(id)
      sessionManager.scheduleSave()
      return { ok: true as const, session }
    } catch (err) {
      log.warn({ err, id }, '[restored] could not resume this session')
      // Put it back. A claim is destructive on purpose, but a claim whose spawn
      // failed must not be the end of the session. Both kinds, because both were
      // taken: the carried-over record goes back to `restored-sessions`, and the
      // one that ended during this run goes back to the pty manager it came from.
      if (restored) restoreHeld(restored)
      else if (dead) ptyManager.restoreReleased(dead)
      claimsPrepared?.()
      if (transcriptId) releaseClaim(transcriptId, id)
      settleResume?.(undefined)
      return {
        ok: false as const,
        reason: 'failed' as const,
        message: err instanceof Error ? err.message : String(err)
      }
    }
  })

  // Headless
  registerMethod('headless:create', async (payload) => {
    const session = await headlessManager.createHeadless(payload)
    logSessionEvent(session.id, 'created', {
      agentType: payload.agentType,
      projectName: payload.projectName,
      projectPath: payload.projectPath,
      headless: true
    })
    return session
  })
  registerMethod('headless:kill', (id) => headlessManager.killHeadless(id))
  registerMethod('headless:list', () => headlessManager.getActiveSessions())

  // Where a browser can reach this server. Asked separately from Tailscale status
  // because it has to answer even when Tailscale is absent — that is the case the
  // old UI could not express at all.
  registerMethod('server:reachableUrls', async () => {
    let tailscaleIps: string[] = []
    try {
      const status = await getTailscaleStatus()
      if (status.running && status.selfIP) tailscaleIps = [status.selfIP]
    } catch {
      // Not installed or not answering; LAN addresses still stand.
    }
    return reachableUrls(serverPort, tailscaleIps)
  })

  // Intent bar completions
  registerMethod('shell:listExecutables', () => listShellExecutables())
  registerMethod('shell:listInstalled', () => listInstalledShells())

  // SSH

  // Fire-and-forget notifications
  registerNotification('terminal:write', ({ id, data }) => ptyManager.writeToPty(id, data))
  registerNotification('terminal:resize', ({ id, cols, rows }) =>
    ptyManager.resizePty(id, cols, rows)
  )

  // Widget status update request

  /**
   * Which instance a manager notification is about, or nothing.
   *
   * `terminal:data` and its siblings all carry the terminal's id, and a client
   * that only wants one terminal's output subscribes to `terminal:data#<id>`.
   * Payloads without an `id` simply have no instance, and match by name alone.
   */
  const terminalScope = (payload: unknown): string | undefined => {
    const id = (payload as { id?: unknown } | null)?.id
    return typeof id === 'string' ? id : undefined
  }

  // Wire manager events → broadcast to WS clients
  vorndSessions.on('notify', (id: string, title: string, body: string, effectId: string) => {
    // The effect's id lets a client that hears it twice show it once.
    clientRegistry.broadcast(IPC.TERMINAL_NOTIFY, { id, title, body, effectId }, id)
  })
  wireVorndRestore(announceSession)

  ptyManager.on('client-message', (channel: string, payload: unknown) => {
    // A payload's `id` is the instance this notification is about, which lets a
    // client subscribe to one terminal rather than to all of them. Read
    // generically rather than per channel: every id-bearing payload here means
    // the same thing by it.
    clientRegistry.broadcast(channel, payload, terminalScope(payload))
  })
  headlessManager.on('client-message', (channel: string, payload: unknown) => {
    clientRegistry.broadcast(channel, payload, terminalScope(payload))
  })

  // ─── Persistent session auto-save ──────────────────────────────
  // Combined with explicit saves on key lifecycle events (session-created,
  // session-exit, SessionStart hook), this reduces reliance on the shutdown
  // path (which has a race with bridge.close and doesn't cover
  // force-quit / crash).
  //
  // Live sessions *and* the ones a previous run left unclaimed. A save is a
  // whole-table replace, so persisting only the live set is what erased every
  // record from the last run the moment a single pane was opened -- and with the
  // record gone, the next start judged that session's history unreachable and
  // deleted it. Holding them here is what makes a terminal survive more than one
  // restart.
  sessionManager.startAutoSave(sessionsToPersist)

  // ─── Hook server integration ──────────────────────────────────

  // Handle new terminal sessions: broadcast to UI + Copilot hook setup
  ptyManager.on('session-created', (session, payload, madeByVornd?: boolean) => {
    announceSession(session)
    // vornd logs the sessions it makes itself.
    if (!madeByVornd)
      logSessionEvent(session.id, 'created', {
        agentType: session.agentType,
        projectName: session.projectName,
        projectPath: session.projectPath,
        ...(session.branch && { branch: session.branch })
      })

    sessionManager.scheduleSave()
  })

  // What vornd changed for a client, told and saved as the methods here that make the same changes.
  ptyManager.on('records-changed', () => {
    sessionManager.scheduleSave()
  })

  // A shell moved. Saved on a debounce, so a script running `cd` in a loop costs
  // one write rather than hundreds -- and what is being kept is where the shell
  // ended up, not every step it took to get there.
  ptyManager.on('session-cwd', () => {
    sessionManager.scheduleSave()
  })

  ptyManager.on('session-exit', (session) => {
    // A session that died early holds nothing; without this its conversation
    // stays unreachable for the rest of the spawn window.
    releaseClaimsFor(session.id)

    sessionManager.scheduleSave()
  })
}
