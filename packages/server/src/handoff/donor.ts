import { spawn, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import type { TerminalSession } from '@vornrun/shared/types'
import {
  HANDOFF_PROTOCOL_VERSION,
  HANDOFF_MANIFEST_VERSION,
  type HandoffRequest,
  type HandoffResult
} from '@vornrun/shared/protocol'
import log from '../logger'
import { FIRST_PTY_SLOT, manifestPath, writeManifest, discardManifest } from './manifest'

/**
 * Giving a running machine away without stopping it.
 *
 * Descriptors travel in the replacement's stdio array, the only way Node hands a
 * live descriptor to another process. Seven rules hold it up:
 * one reader per pty; nothing captured until every reader has stopped; a failure
 * before the commit leaves this server owning everything; after it, nothing is
 * torn down; no path closes the last descriptor; a partial handoff is refused;
 * the commit is a point, not a period.
 */

/** How long the replacement has to prove it can take the panes. */
const IMPORT_DEADLINE_MS = 20_000

/** Longer than the import: this covers a whole server startup on a busy machine. */
const SERVING_DEADLINE_MS = 45_000

/** A pane as the donor sees it, before it becomes a slot in an argument list. */
export interface DonorPane {
  session: TerminalSession
  /** The pty master. Present for every forked pty on POSIX; see `describePanes`. */
  fd: number
  pid: number
  cols: number
  rows: number
}

/** Injected so the sequence can be tested: its failure mode is every terminal on the machine. */
export interface HandoffHost {
  dataDir: string
  /** Every live pane, or null if even one of them cannot be described. */
  describePanes(): DonorPane[] | null
  /** Stop every reader. Bytes already in the kernel buffer stay there. */
  pauseAll(): void
  /** Start reading again, for a handoff that did not happen. */
  resumeAll(): void
  /** Buffered output, session records and screens onto disk. */
  quiesce(): Promise<void>
  /** Where the replacement's stdout and stderr should go. */
  openLogFd(): number
  /** The commit: after this nothing reaches this server by name. */
  release(): Promise<void>
  /** Take the endpoint back after a replacement that died post-commit. */
  reclaim(): Promise<boolean>
  /** Leave, without killing a single pty. */
  exit(): void
}

/** The message a person sees if they start a terminal mid-handoff. */
export const HANDOVER_MESSAGE =
  'Vorn is moving your terminals to the updated server. Try again in a moment.'

let inFlight = false

/** Whether a handoff is running. Session creation is refused while one is. */
export function isHandingOver(): boolean {
  return inFlight
}

/** Test-only, mirroring the other module-level state in this package. */
export function resetHandoffForTests(): void {
  inFlight = false
}

/** Injected so the commit boundary and both rollbacks can be tested against a fake child. */
export type SpawnHeir = (
  request: HandoffRequest,
  args: string[],
  stdio: Array<'ignore' | 'ipc' | number>,
  logFd: number
) => ChildProcess

const spawnHeirProcess: SpawnHeir = (request, args, stdio) =>
  spawn(request.exec, args, {
    cwd: request.cwd,
    env: { ...process.env, ...request.env },
    detached: true,
    stdio
  })

export async function handOver(
  request: HandoffRequest,
  host: HandoffHost,
  spawnHeir: SpawnHeir = spawnHeirProcess
): Promise<HandoffResult> {
  const declined = (because: string): HandoffResult => {
    log.warn({ because }, '[handoff] declined')
    return { kind: 'declined', because }
  }

  if (process.platform === 'win32') {
    // No way to inherit a console pseudoterminal there, and no endpoint to ask over.
    return declined('this platform cannot pass a pty to another process')
  }
  if (request.handoffVersion !== HANDOFF_PROTOCOL_VERSION) {
    return declined(
      `the caller speaks handoff ${request.handoffVersion}, this server speaks ${HANDOFF_PROTOCOL_VERSION}`
    )
  }
  if (inFlight) return declined('a handoff is already running')
  if (!request.exec || !fs.existsSync(request.exec)) {
    return declined(`there is nothing to run at ${request.exec}`)
  }

  inFlight = true
  try {
    return await run(request, host, declined, spawnHeir)
  } catch (err) {
    // A throw between the pause and the resume would leave every pane frozen.
    log.error({ err }, '[handoff] failed unexpectedly; resuming')
    host.resumeAll()
    return declined(err instanceof Error ? err.message : String(err))
  } finally {
    inFlight = false
  }
}

async function run(
  request: HandoffRequest,
  host: HandoffHost,
  declined: (because: string) => HandoffResult,
  spawnHeir: SpawnHeir
): Promise<HandoffResult> {
  // Stop reading first, so the manifest describes a machine holding still.
  host.pauseAll()

  // All of it, before a single side effect: a partial handoff is refused.
  const panes = host.describePanes()
  if (panes === null) {
    host.resumeAll()
    return declined('at least one terminal could not be described')
  }

  await host.quiesce()

  const slots = panes.map((pane, index) => ({ ...pane, slot: FIRST_PTY_SLOT + index }))
  const target = manifestPath(host.dataDir, process.pid)
  writeManifest(target, {
    version: HANDOFF_MANIFEST_VERSION,
    donorPid: process.pid,
    createdAt: Date.now(),
    panes: slots.map((pane) => ({
      slot: pane.slot,
      session: pane.session,
      pid: pane.pid,
      cols: pane.cols,
      rows: pane.rows
    }))
  })

  const logFd = host.openLogFd()
  let heir: ChildProcess
  try {
    heir = spawnHeir(
      request,
      [...request.args, '--adopt-handoff', target],
      // Duplicated into the child, so both processes hold every pty across the window.
      ['ignore', logFd, logFd, 'ipc', ...slots.map((pane) => pane.fd)],
      logFd
    )
  } catch (err) {
    fs.closeSync(logFd)
    discardManifest(target)
    host.resumeAll()
    return declined(`could not start the replacement: ${(err as Error).message}`)
  }
  fs.closeSync(logFd)

  log.info(
    { pid: heir.pid, panes: slots.length, version: request.appVersion },
    '[handoff] replacement started; waiting for it to take the panes'
  )

  const imported = await waitFor(heir, 'imported', IMPORT_DEADLINE_MS)
  if (!imported.ok) {
    // Nothing was released, so this rollback is complete.
    try {
      heir.kill('SIGKILL')
    } catch {
      // Already gone, which is the usual reason we are here.
    }
    discardManifest(target)
    host.resumeAll()
    return declined(`the replacement never took the panes: ${imported.because}`)
  }

  // ─── The commit boundary: the name is free and the replacement should have it.
  await host.release()
  heir.send({ kind: 'commit' })

  const serving = await waitFor(heir, 'serving', SERVING_DEADLINE_MS)
  if (!serving.ok) {
    // Both processes still hold every pty, so there is something to go back to.
    log.error(
      { because: serving.because },
      '[handoff] the replacement did not come up after the commit; taking the endpoint back'
    )
    try {
      heir.kill('SIGKILL')
    } catch {
      // Expected when it died on its own, which is the likely cause.
    }
    discardManifest(target)
    const back = await host.reclaim()
    host.resumeAll()
    return declined(
      back
        ? 'the replacement failed to start; this server took its terminals back'
        : 'the replacement failed to start and the endpoint could not be reclaimed; ' +
            'the terminals are still running here, but Vorn must be reopened to reach them'
    )
  }

  log.info({ pid: heir.pid }, '[handoff] the replacement is serving; standing down')
  // Nothing is killed: leaving closes this copy and the replacement holds the other.
  heir.unref()
  heir.disconnect?.()
  const result: HandoffResult = {
    kind: 'handed-over',
    sessions: slots.length,
    pid: heir.pid ?? 0
  }
  // Late enough for the reply to reach the caller before the socket closes.
  setTimeout(() => host.exit(), 250)
  return result
}

/** An exit is an answer too, and a faster one than the deadline. */
function waitFor(
  child: ChildProcess,
  kind: string,
  timeoutMs: number
): Promise<{ ok: true } | { ok: false; because: string }> {
  return new Promise((resolve) => {
    let settled = false
    const done = (answer: { ok: true } | { ok: false; because: string }): void => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      child.off('message', onMessage)
      child.off('exit', onExit)
      child.off('error', onError)
      resolve(answer)
    }
    const onMessage = (msg: unknown): void => {
      if ((msg as { kind?: string } | null)?.kind === kind) done({ ok: true })
    }
    const onExit = (code: number | null, signal: string | null): void =>
      done({ ok: false, because: `it exited (code ${code}, signal ${signal})` })
    const onError = (err: Error): void => done({ ok: false, because: err.message })
    const timer = setTimeout(
      () => done({ ok: false, because: `it did not say "${kind}" in time` }),
      timeoutMs
    )

    child.on('message', onMessage)
    child.on('exit', onExit)
    child.on('error', onError)
  })
}
