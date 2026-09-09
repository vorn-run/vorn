import tty from 'node:tty'
import type { TerminalSession } from '@vornrun/shared/types'
import log from '../logger'
import { AdoptedPty } from './adopted-pty'
import { readManifest, discardManifest } from './manifest'

/**
 * Taking a running machine over from the server that had it.
 *
 * Split along the commit boundary: before `imported`, only what proves the panes
 * can be taken, because giving up is still free. After `commit`, an ordinary
 * startup that happens to begin with terminals in hand.
 */

export interface AdoptedPane {
  session: TerminalSession
  pty: AdoptedPty
  cols: number
  rows: number
}

const COMMIT_DEADLINE_MS = 30_000

/**
 * The handshake channel: `process` in production, a fake under test.
 *
 * Injected because the alternative is a test that replaces `process.send` and
 * removes `process` listeners -- in a runner that uses both to report its own
 * results.
 */
export interface HandoffChannel {
  send?: (message: unknown) => boolean
  on(event: string, listener: (...args: unknown[]) => void): unknown
  off(event: string, listener: (...args: unknown[]) => void): unknown
}

/** Null means this process must not serve; the outgoing server still has everything. */
export async function receiveHandoff(
  source: string,
  channel: HandoffChannel = process
): Promise<AdoptedPane[] | null> {
  if (typeof channel.send !== 'function') {
    log.error('[handoff] started with --adopt-handoff but no channel to answer on')
    return null
  }

  const manifest = readManifest(source)
  if (!manifest) {
    // Not "no panes": serving nothing while holding their descriptors is how work disappears.
    log.error({ source }, '[handoff] the manifest is missing or unreadable')
    return null
  }

  const panes: AdoptedPane[] = []
  for (const pane of manifest.panes) {
    // Checked, not trusted: a slot that is not a pty is a pane that would never run anything.
    if (!tty.isatty(pane.slot)) {
      log.error(
        { slot: pane.slot, session: pane.session.id },
        '[handoff] a slot named in the manifest is not a terminal'
      )
      return null
    }
    try {
      // Built paused: the bytes stay in the kernel's buffer until a session can hold them.
      const adopted = new AdoptedPty(pane.slot, pane.pid)
      adopted.pause()
      panes.push({ session: pane.session, pty: adopted, cols: pane.cols, rows: pane.rows })
    } catch (err) {
      log.error(
        { err, slot: pane.slot },
        '[handoff] could not build a reader over an inherited pty'
      )
      return null
    }
  }

  log.info(
    { panes: panes.length, donor: manifest.donorPid },
    '[handoff] panes taken; waiting for the commit'
  )
  channel.send({ kind: 'imported' })

  const committed = await waitForCommit(channel)
  if (!committed) {
    log.error('[handoff] the outgoing server never committed; standing down')
    return null
  }

  discardManifest(source)
  // The outgoing server is about to exit and close this channel.
  process.channel?.unref()
  return panes
}

function waitForCommit(channel: HandoffChannel): Promise<boolean> {
  return new Promise((resolve) => {
    let settled = false
    const done = (answer: boolean): void => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      channel.off('message', onMessage)
      channel.off('disconnect', onDisconnect)
      resolve(answer)
    }
    const onMessage = (msg: unknown): void => {
      if ((msg as { kind?: string } | null)?.kind === 'commit') done(true)
    }
    // The channel closing first means the donor died without releasing anything.
    const onDisconnect = (): void => done(false)
    const timer = setTimeout(() => done(false), COMMIT_DEADLINE_MS)

    channel.on('message', onMessage)
    channel.on('disconnect', onDisconnect)
  })
}

/** Tell the outgoing server it may leave. */
export function announceServing(channel: HandoffChannel = process): void {
  try {
    channel.send?.({ kind: 'serving' })
  } catch (err) {
    log.warn({ err }, '[handoff] could not tell the outgoing server we are serving')
  }
}
