import fs from 'fs'
import crypto from 'node:crypto'
import { registerMethod, registerNotification } from './ws-handler'
import { ptyManager } from './pty-manager'
import { headlessManager } from './headless-manager'
import { configManager } from './config-manager'
import { sessionManager } from './session-persistence'
import { getRecentSessions } from './agent-history'
import { detectIDEs, openInIDE } from './ide-detector'
import { detectInstalledAgents } from './agent-detector'
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
import { hookServer } from './hook-server'
import { hookStatusMapper } from './hook-status-mapper'
import { installHooks } from './hook-installer'
import { installCopilotHooks } from './copilot-hook-installer'
import {
  IPC,
  PermissionRequestInfo,
  SessionEventType,
  RemoteHost,
  getProjectRemoteHostId
} from '@vornrun/shared/types'
import type { ProjectConfig, TerminalSession, WorktreeRetentionConfig } from '@vornrun/shared/types'
import { DEFAULT_ARTIFACT_DIRS } from '@vornrun/shared/types'
import * as gitUtils from './git-utils'
import {
  scanWorktreeInventory,
  reclaimArtifacts,
  removeWorktrees,
  pruneOrphanDirs,
  deleteStaleBranches,
  measureWorktree,
  invalidateSizeCache
} from './worktree-inventory'
import { fileStamp, listDir, readFileContent, writeFileContent } from './file-utils'
import { listShellExecutables } from './shell-integration'
import { listInstalledShells } from './shell-integration/installed'
import {
  dbSaveSSHKey,
  dbListSSHKeys,
  dbGetSSHKey,
  dbDeleteSSHKey,
  insertSessionEvent
} from './database'
import { getTailscaleStatus, clearBinaryCache } from './tailscale'
import { reachableUrls } from './reachable-urls'
import { listTokens, mintOwnerToken, revokeToken } from './token-manager'
import {
  approveRequest,
  cancelPairing,
  denyRequest,
  pendingRequests,
  startPairing
} from './pairing'
import { disconnectToken } from './ws-handler'
import { captureAgentSessionId } from './agent-session-capture'
import { listAgentModels } from './agent-model-catalog'
import { supportsExactSessionResume, supportsSessionIdPinning } from '@vornrun/shared/types'
import log from './logger'
import { vorndSessions } from './vornd-sessions'
import { wireVorndRestore } from './vornd-restore'
import { onePerKey } from './one-per-key'
import { isWorkspaceHeld } from './workspace-holds'

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
  // vornd answers both for every session it holds, which is every live one.
  // What reaches the server is a session it has no screen of: one from a
  // previous run, or one asked for by a client that is not behind vornd.
  registerMethod('terminal:readScrollback', () => ({ data: '' }))
  registerMethod('terminal:attach', ({ id }) => ({
    data: '',
    seq: 0,
    live: ptyManager.hasLivePty(id)
  }))
  registerMethod('terminal:readOutput', ({ id, lines }) => ptyManager.readOutput(id, lines))
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
  registerMethod('sessions:getRecent', (projectPath) => getRecentSessions(projectPath))

  // Resolve remote host by ID
  function resolveRemoteHostById(hostId: string): RemoteHost | undefined {
    const cfg = configManager.loadConfig()
    return cfg.remoteHosts?.find((h) => h.id === hostId)
  }

  // Git — resolve remote host for project or worktree paths
  function resolveRemoteHost(projectPath: string): RemoteHost | undefined {
    const cfg = configManager.loadConfig()
    const project = cfg.projects.find((p) => p.path === projectPath)
    if (!project) return undefined
    const remoteId = getProjectRemoteHostId(project)
    if (!remoteId) return undefined
    return cfg.remoteHosts?.find((h) => h.id === remoteId)
  }

  /** Resolve remote host from any path (project root or worktree subdirectory). */
  function resolveRemoteHostByPath(anyPath: string): RemoteHost | undefined {
    const cfg = configManager.loadConfig()
    for (const project of cfg.projects) {
      if (anyPath === project.path || anyPath.startsWith(project.path + '/')) {
        const remoteId = getProjectRemoteHostId(project)
        if (!remoteId) return undefined
        return cfg.remoteHosts?.find((h) => h.id === remoteId)
      }
      const parentDir = project.path.replace(/\/[^/]+$/, '')
      if (anyPath.startsWith(parentDir + '/.vorn-worktrees/')) {
        const remoteId = getProjectRemoteHostId(project)
        if (!remoteId) return undefined
        return cfg.remoteHosts?.find((h) => h.id === remoteId)
      }
    }
    return undefined
  }

  registerMethod('git:isGitRepo', (projectPath) => gitUtils.isGitRepo(projectPath))
  registerMethod('git:listBranches', async (projectPath) => {
    const remote = resolveRemoteHost(projectPath)
    const isRepo = remote || (await gitUtils.isGitRepo(projectPath))
    return {
      local: isRepo ? await gitUtils.listBranches(projectPath, remote) : [],
      current: isRepo ? await gitUtils.getGitBranch(projectPath, remote) : null,
      isGitRepo: !!isRepo
    }
  })
  registerMethod('git:listRemoteBranches', (projectPath) => {
    const remote = resolveRemoteHost(projectPath)
    return gitUtils.listRemoteBranches(projectPath, remote)
  })
  registerMethod('git:createWorktree', ({ projectPath, branch, worktreeName }) => {
    const remote = resolveRemoteHost(projectPath)
    return gitUtils.createWorktree(projectPath, branch, worktreeName, remote)
  })
  registerMethod('git:removeWorktree', ({ projectPath, worktreePath, force, deleteBranch }) => {
    const remote = resolveRemoteHost(projectPath)
    invalidateSizeCache(worktreePath)
    return gitUtils.removeWorktree(projectPath, worktreePath, force, remote, deleteBranch)
  })
  registerMethod('git:checkoutBranch', async ({ cwd, branch }) => {
    const remote = resolveRemoteHostByPath(cwd)
    const result = await gitUtils.checkoutBranch(cwd, branch, remote)
    if (result.ok) {
      ptyManager.updateSessionsForWorktree(cwd, { branch })
      headlessManager.updateSessionsForWorktree(cwd, { branch })
    }
    return result
  })
  registerMethod('git:getWorktreeBranch', (worktreePath) => {
    const remote = resolveRemoteHostByPath(worktreePath)
    return gitUtils.getGitBranch(worktreePath, remote)
  })
  registerMethod('git:renameWorktreeBranch', async ({ worktreePath, newBranch }) => {
    const remote = resolveRemoteHostByPath(worktreePath)
    const result = await gitUtils.renameWorktreeBranch(worktreePath, newBranch, remote)
    if (result) {
      ptyManager.updateSessionsForWorktree(worktreePath, { branch: newBranch })
      headlessManager.updateSessionsForWorktree(worktreePath, { branch: newBranch })
    }
    return result
  })
  registerMethod('git:renameWorktree', async ({ worktreePath, newName }) => {
    const remote = resolveRemoteHostByPath(worktreePath)
    const result = await gitUtils.renameWorktree(worktreePath, newName, remote)
    if (result) {
      ptyManager.updateSessionsForWorktree(worktreePath, {
        worktreePath: result.newPath,
        worktreeName: result.name
      })
      headlessManager.updateSessionsForWorktree(worktreePath, {
        worktreePath: result.newPath,
        worktreeName: result.name
      })
    }
    return result
  })
  registerMethod('git:worktreeDirty', (worktreePath) => {
    const remote = resolveRemoteHostByPath(worktreePath)
    return gitUtils.isWorktreeDirty(worktreePath, remote)
  })
  registerMethod('git:listWorktrees', (projectPath) => {
    const remote = resolveRemoteHost(projectPath)
    return gitUtils.listWorktrees(projectPath, remote)
  })

  registerMethod('worktree:activeSessions', (worktreePath: string) => {
    const pty = ptyManager.getActiveSessionsForWorktree(worktreePath)
    const headless = headlessManager.getActiveSessionsForWorktree(worktreePath)
    return {
      count: pty.count + headless.count,
      sessionIds: [...pty.sessionIds, ...headless.sessionIds]
    }
  })

  // ─── Worktree manager ──────────────────────────────────────────

  function activeSessionIds(worktreePath: string): string[] {
    return [
      ...ptyManager.getActiveSessionsForWorktree(worktreePath).sessionIds,
      ...headlessManager.getActiveSessionsForWorktree(worktreePath).sessionIds
    ]
  }

  function retentionConfig(): WorktreeRetentionConfig {
    return configManager.loadConfig().defaults.worktreeRetention ?? {}
  }

  function artifactDirNames(): string[] {
    const configured = retentionConfig().artifactDirs
    return configured?.length ? configured : DEFAULT_ARTIFACT_DIRS
  }

  /** Cached size for a path — measured during the scan that preceded the action. */
  function cachedSizeOf(worktreePath: string): number {
    const remote = resolveRemoteHostByPath(worktreePath)
    return measureWorktree(worktreePath, artifactDirNames(), remote).sizeBytes
  }

  /**
   * Refuse to act on a worktree that has a live session. Checked immediately
   * before the action rather than read off the scan, because a session can
   * start while the panel is open.
   */
  function assertNoActiveSessions(paths: string[]): void {
    for (const p of paths) assertIdle(p)
  }

  /**
   * Checked up front, and again by the action just before it deletes: the git
   * in between lets a session start, or finish preparing, in the same path.
   */
  function assertIdle(p: string): void {
    const count = activeSessionIds(p).length
    if (count > 0) {
      throw new Error(`${p} has ${count} active session${count > 1 ? 's' : ''} — close them first`)
    }
    if (isWorkspaceHeld(p)) throw new Error(`${p} has a session starting — close it first`)
  }

  /** Resolve a project to its remote host, or undefined when it is local. */
  function remoteForProject(project: ProjectConfig): RemoteHost | undefined {
    const remoteId = getProjectRemoteHostId(project)
    if (!remoteId) return undefined
    return configManager.loadConfig().remoteHosts?.find((h) => h.id === remoteId)
  }

  registerMethod('worktree:inventory', (params) => {
    const cfg = configManager.loadConfig()
    return scanWorktreeInventory({
      projects: cfg.projects,
      projectPaths: params?.projectPaths,
      refresh: params?.refresh,
      retention: cfg.defaults.worktreeRetention,
      resolveRemote: remoteForProject,
      getActiveSessions: activeSessionIds
    })
  })

  registerMethod('worktree:reclaimArtifacts', ({ paths }) => {
    assertNoActiveSessions(paths)
    const cfg = configManager.loadConfig()
    return reclaimArtifacts(paths, artifactDirNames(), cfg.projects, remoteForProject, assertIdle)
  })

  registerMethod('worktree:removeMany', ({ items }) => {
    assertNoActiveSessions(items.map((i) => i.worktreePath))
    const cfg = configManager.loadConfig()
    return removeWorktrees(items, cachedSizeOf, cfg.projects, remoteForProject, assertIdle)
  })

  registerMethod('worktree:pruneOrphans', ({ paths }) => {
    assertNoActiveSessions(paths)
    return pruneOrphanDirs(paths, cachedSizeOf, resolveRemoteHostByPath, assertIdle)
  })

  registerMethod('git:deleteBranches', ({ projectPath, branches, force }) => {
    const remote = resolveRemoteHost(projectPath)
    return deleteStaleBranches(projectPath, branches, force ?? false, remote)
  })
  registerMethod('git:getBranch', (cwd) => {
    const remote = resolveRemoteHostByPath(cwd)
    return gitUtils.getGitBranch(cwd, remote)
  })
  registerMethod('git:diffStat', (cwd) => {
    const remote = resolveRemoteHostByPath(cwd)
    return gitUtils.getGitDiffStat(cwd, remote)
  })
  registerMethod('git:diffFull', (req) => {
    const cwd = typeof req === 'string' ? req : req.cwd
    const remote = resolveRemoteHostByPath(cwd)
    const range = typeof req === 'string' ? undefined : { from: req.from, to: req.to }
    return gitUtils.getGitDiffFull(cwd, remote, range)
  })
  registerMethod('git:commit', ({ cwd, message, includeUnstaged }) => {
    const remote = resolveRemoteHostByPath(cwd)
    return gitUtils.gitCommit(cwd, message, includeUnstaged, remote)
  })
  registerMethod('git:push', (cwd) => {
    const remote = resolveRemoteHostByPath(cwd)
    return gitUtils.gitPush(cwd, remote)
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

  // Agent/IDE detection
  registerMethod('agent:detectInstalled', () => detectInstalledAgents())
  registerMethod('agent:listModels', (request) => listAgentModels(request))
  registerMethod('ide:detect', () => detectIDEs())
  registerMethod('ide:open', ({ ideId, projectPath }) => openInIDE(ideId, projectPath))

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

  // Tailscale network access. Informational only now: it supplies an address and
  // a QR code, and no longer decides whether the server binds wide.
  registerMethod('tailscale:status', async () => {
    clearBinaryCache() // Always re-detect in case user just installed
    // Deliberately does not rebind. Reading status used to have that side effect,
    // because Tailscale could start after boot and change the answer. Nothing
    // about the bind depends on it now, and rebinding drops every connection —
    // so it happens when the setting changes, and at no other time.
    return getTailscaleStatus(serverPort)
  })

  // Credential vault (storage — encryption handled by main process)
  registerMethod('credential:storeKey', (params) => {
    const id = crypto.randomUUID()
    dbSaveSSHKey({
      id,
      label: params.label,
      encryptedPrivateKey: params.encryptedPrivateKey,
      publicKey: params.publicKey,
      certificate: params.certificate,
      keyType: params.keyType,
      createdAt: new Date().toISOString()
    })
    return { id }
  })
  registerMethod('credential:listKeys', () => dbListSSHKeys())
  registerMethod('credential:deleteKey', (id) => dbDeleteSSHKey(id))
  registerMethod('credential:getEncryptedKey', (id) => dbGetSSHKey(id))

  // Device tokens. Until now these existed only behind `vorn-server token`, so
  // pairing a phone meant finding a terminal on the machine running the server.
  registerMethod('token:list', () => listTokens())
  registerMethod('token:create', ({ name }) => {
    // Coerced rather than trusted: a malformed param would otherwise fail inside
    // `.trim()` with a TypeError that reaches the client verbatim, saying nothing
    // about what was wrong.
    const label = typeof name === 'string' ? name.trim() : ''
    // The plaintext is returned exactly once and never stored — only its hash
    // reaches the database — so the caller has to show it and then drop it.
    const minted = mintOwnerToken(label || 'Device')
    return { token: minted.token, plaintext: minted.plaintext }
  })
  // Pairing a phone. These four are the desktop's half: it asks for a code,
  // sees who offered it, and decides. The phone's half is two HTTP routes in
  // `index.ts`, because a phone that has not paired yet has no credential and
  // the socket admits exactly one method before authenticating.
  registerMethod('pairing:start', () => startPairing())
  registerMethod('pairing:pending', () => pendingRequests())
  registerMethod('pairing:approve', ({ requestId }) => ({ ok: approveRequest(requestId) }))
  registerMethod('pairing:deny', ({ requestId }) => ({ ok: denyRequest(requestId) }))
  registerMethod('pairing:cancel', () => {
    cancelPairing()
    return { ok: true }
  })

  registerMethod('token:revoke', (id) => {
    const revoked = revokeToken(id)
    // Revoking has to reach a socket already holding the token, or a lost phone
    // keeps working until it happens to reconnect.
    if (revoked) disconnectToken(id)
    return { revoked }
  })

  // File explorer
  registerMethod('file:listDir', ({ dirPath, remoteHostId }) => {
    const remote = remoteHostId ? resolveRemoteHostById(remoteHostId) : undefined
    return listDir(dirPath, remote)
  })
  registerMethod('file:readContent', ({ filePath, maxBytes, remoteHostId }) => {
    const remote = remoteHostId ? resolveRemoteHostById(remoteHostId) : undefined
    return readFileContent(filePath, maxBytes, remote)
  })
  registerMethod('file:stamp', ({ filePath, remoteHostId }) => {
    const remote = remoteHostId ? resolveRemoteHostById(remoteHostId) : undefined
    return fileStamp(filePath, remote)
  })
  registerMethod('file:writeContent', ({ filePath, content, remoteHostId }) => {
    const remote = remoteHostId ? resolveRemoteHostById(remoteHostId) : undefined
    return writeFileContent(filePath, content, remote)
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

  // Permission resolution
  registerMethod('permission:resolve', ({ requestId, allow, updatedPermissions, updatedInput }) => {
    hookServer.resolvePermission(requestId, allow, { updatedPermissions, updatedInput })
  })

  // Resolve top pending permission (for global shortcuts)
  registerMethod('permission:resolve-top', ({ allow }) => {
    const pending = hookServer.getPendingPermissions()
    if (pending.length > 0) {
      hookServer.resolvePermission(pending[0].requestId, allow)
    }
  })

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

    if (payload.agentType === 'copilot' && hookServer.getPort() > 0) {
      const installation = installCopilotHooks(session.id)
      hookStatusMapper.forceLink(installation.sessionId, session.id)
      ptyManager.linkHookSession(session.id, installation.sessionId)
      // Don't set statusSource = 'hooks' eagerly — it disables the pattern-based
      // fallback. If hooks actually fire, the session is promoted on the
      // first event. This fixes status stuck on 'waiting' when hooks don't work
      // (e.g. the agent CLI doesn't support hooks.json).
    }

    // A local lookup for agents that cannot pin an id; a known id or a remote session is never replaced.
    if (
      supportsExactSessionResume(payload.agentType) &&
      !supportsSessionIdPinning(payload.agentType) &&
      !session.agentSessionId &&
      !session.remoteHostId
    ) {
      const captureSessionId = session.id
      // Asked more than once: an agent slow to write its own history used to be
      // read at five seconds, come up empty, and never be asked again -- leaving
      // the session holding a conversation it could not name, which a later
      // resume was then free to take.
      const attempt = (remaining: number[]): void => {
        const [delay, ...rest] = remaining
        if (delay === undefined) return
        setTimeout(() => {
          const s = ptyManager.getActiveSessions().find((t) => t.id === captureSessionId)
          if (!s) {
            releaseClaimsFor(captureSessionId)
            return
          }
          if (s.agentSessionId) return
          const cwd = s.worktreePath || s.projectPath
          const capturedId = captureAgentSessionId(s.agentType, cwd)
          if (!capturedId) return attempt(rest)
          ptyManager.setRecordFields(s.id, { agentSessionId: capturedId })
          // Its own record names the conversation now, so the spawn claim is spent.
          releaseClaimsFor(captureSessionId)
          sessionManager.scheduleSave()
          clientRegistry.broadcast(IPC.SESSION_UPDATED, s)
          log.info(`[session] captured ${s.agentType} session ID: ${capturedId}`)
        }, delay)
      }
      attempt([5000, 5000, 10_000, 20_000])
    }

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

  // Start hook server
  hookServer
    .start()
    .then((port) => {
      try {
        // Only the instance that claimed the shared hook files writes the
        // settings entry that points at them. A dev server beside the packaged
        // app used to redirect its hooks here and, killed before it could tidy
        // up, leave them pointing at a port with no server behind it.
        if (hookServer.ownsRegistration()) {
          installHooks(port, hookServer.getAuthToken())
        } else {
          log.info('[hooks] another Vorn owns the registration; leaving it alone')
          hookServer.once('claimed', () => installHooks(port, hookServer.getAuthToken()))
        }
      } catch (err) {
        log.error({ err }, '[hooks] failed to install hooks:')
      }

      hookServer.on('permission-cancelled', (requestId: string) => {
        clientRegistry.broadcast(IPC.WIDGET_PERMISSION_CANCELLED, requestId)
      })

      hookServer.on('hook-event', (event) => {
        log.info(
          `[hooks] ${event.hook_event_name}: session=${event.session_id} cwd=${event.cwd}` +
            (event.vorn_terminal_id ? ` terminal=${event.vorn_terminal_id}` : '')
        )
        const result = hookStatusMapper.mapEventToStatus(event)
        if (result) {
          ptyManager.hookStatus(result.terminalId, result.status, true)

          // Persist after hookSessionId is set (SessionStart links the session)
          if (event.hook_event_name === 'SessionStart') {
            sessionManager.scheduleSave()
            try {
              const config = configManager.loadConfig()
              const task = config.tasks?.find(
                (t) =>
                  t.assignedSessionId === result.terminalId &&
                  t.status === 'in_progress' &&
                  !t.agentSessionId
              )
              if (task) {
                task.agentSessionId = event.session_id
                task.updatedAt = new Date().toISOString()
                configManager.saveConfig(config)
                configManager.notifyChanged()
                log.info(
                  `[hooks] stored agentSessionId ${event.session_id} on task "${task.title}"`
                )
              }
            } catch (err) {
              log.error({ err }, '[hooks] failed to persist agentSessionId:')
            }
          }
        }

        const dismissEvents = ['PostToolUse', 'PostToolUseFailure', 'Stop', 'UserPromptSubmit']
        if (dismissEvents.includes(event.hook_event_name)) {
          hookServer.cancelSessionPermissions(event.session_id)
        }
      })

      hookServer.on('permission-request', ({ requestId, event }) => {
        const terminalId = hookStatusMapper.resolveTerminal(event)

        log.info(
          `[hooks] permission-request: session=${event.session_id} tool=${event.tool_name} → terminal=${terminalId ?? 'none (passthrough)'}`
        )

        if (!terminalId) {
          hookServer.passthroughPermission(requestId)
          return
        }

        ptyManager.hookStatus(terminalId, null, true)

        const session = ptyManager.getActiveSessions().find((s) => s.id === terminalId)

        const permReq: PermissionRequestInfo = {
          requestId,
          sessionId: event.session_id,
          terminalId,
          toolName: event.tool_name || 'unknown',
          toolInput: event.tool_input || {},
          description:
            typeof event.tool_input?.file_path === 'string'
              ? (event.tool_input.file_path as string)
              : typeof event.tool_input?.command === 'string'
                ? (event.tool_input.command as string)
                : typeof event.tool_input?.description === 'string'
                  ? (event.tool_input.description as string)
                  : undefined,
          agentType: session?.agentType,
          projectName: session?.projectName,
          permissionSuggestions: event.permission_suggestions,
          questions:
            event.tool_name === 'AskUserQuestion'
              ? (event.tool_input?.questions as PermissionRequestInfo['questions'] | undefined)
              : undefined
        }

        clientRegistry.broadcast(IPC.WIDGET_PERMISSION_REQUEST, permReq)
        ptyManager.hookStatus(terminalId, 'waiting', false)
      })
    })
    .catch((err) => {
      log.error('Failed to start hook server:', err)
    })
}
