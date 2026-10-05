import log from '../logger'
import { readManifest, discardManifest } from './manifest'

/**
 * Taking over from the server this one replaces.
 *
 * Split along the commit boundary: before `imported`, only what proves the
 * takeover can happen, because giving up is still free. After `commit`, an
 * ordinary startup. Every terminal is in vornd's session holder, which this
 * server's vornd takes over, so nothing is carried across but the endpoint; a
 * server that still holds terminals of its own keeps them.
 */

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

/** False means this process must not serve; the outgoing server still has everything. */
export async function receiveHandoff(
  source: string,
  channel: HandoffChannel = process
): Promise<boolean> {
  if (typeof channel.send !== 'function') {
    log.error('[handoff] started with --adopt-handoff but no channel to answer on')
    return false
  }

  const manifest = readManifest(source)
  if (!manifest) {
    log.error({ source }, '[handoff] the manifest is missing or unreadable')
    return false
  }
  if (manifest.panes.length) {
    // Its terminals run on it, not in vornd, and nothing here can read them.
    log.error(
      { panes: manifest.panes.length },
      '[handoff] the outgoing server holds terminals of its own; it keeps them'
    )
    return false
  }

  channel.send({ kind: 'imported' })

  const committed = await waitForCommit(channel)
  if (!committed) {
    log.error('[handoff] the outgoing server never committed; standing down')
    return false
  }

  discardManifest(source)
  // The outgoing server is about to exit and close this channel.
  process.channel?.unref()
  return true
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
