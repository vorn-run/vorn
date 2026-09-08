import type {
  ExtensionFooterItem,
  ExtensionFooterReading,
  InstalledConnectorPack,
  TerminalSession
} from '@vornrun/shared/types'
import { IPC } from '@vornrun/shared/types'
import { clientRegistry } from '../broadcast'
import { activationFor, subjectOf } from './activation'
import { getOrStartHost, installedExtensions } from './hosts'
import log from '../logger'

/**
 * A footer band, recomputed on the interval its extension declared.
 *
 * One timer per session and footer, because each declares its own interval and
 * a slow one must not hold up a fast one. A run that fails keeps the last good
 * reading beside the error and slows down, so a broken extension costs one line
 * on the card rather than a poll loop nobody asked for.
 */

const TONES = new Set(['default', 'ok', 'danger'])
/** A failing footer is retried this many times slower, until it answers again. */
const FAILURE_BACKOFF = 5

interface Poller {
  timer: NodeJS.Timeout
  everyMs: number
  failing: boolean
  /** What the running timer was built from, so an upgraded pack replaces it. */
  declared: { everyMs: number; title: string; version: string }
}

const pollers = new Map<string, Poller>()
const readings = new Map<string, ExtensionFooterReading>()
// A run in flight, so a slow footer does not start a second of itself on the next tick.
const inFlight = new Set<string>()

const keyOf = (sessionId: string, extensionId: string, footerId: string): string =>
  `${sessionId} ${extensionId} ${footerId}`

/** What every footer says about one session, in a stable order. */
export function footerReadings(sessionId: string): ExtensionFooterReading[] {
  return [...readings.entries()]
    .filter(([key]) => key.startsWith(`${sessionId} `))
    .map(([, reading]) => reading)
    .sort((a, b) =>
      a.extensionId === b.extensionId
        ? a.footerId.localeCompare(b.footerId)
        : a.extensionId.localeCompare(b.extensionId)
    )
}

/** Items an extension returned, held to what a band can draw. */
function readItems(value: unknown): ExtensionFooterItem[] {
  const raw = (value as { items?: unknown })?.items
  if (!Array.isArray(raw)) throw new Error('a footer answers with items')
  return raw.slice(0, 12).map((entry) => {
    const item = entry as Record<string, unknown>
    if (typeof item?.label !== 'string' || typeof item?.value !== 'string') {
      throw new Error('every item carries a label and a value')
    }
    let href: string | undefined
    if (typeof item.href === 'string' && item.href !== '') {
      const parsed = new URL(item.href)
      if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
        throw new Error('an item links to http or https, or to nothing')
      }
      href = item.href
    }
    return {
      label: item.label.slice(0, 60),
      value: item.value.slice(0, 60),
      ...(typeof item.tone === 'string' && TONES.has(item.tone)
        ? { tone: item.tone as ExtensionFooterItem['tone'] }
        : {}),
      ...(href !== undefined && { href })
    }
  })
}

const same = (a: ExtensionFooterReading | undefined, b: ExtensionFooterReading): boolean =>
  a !== undefined && a.error === b.error && JSON.stringify(a.items) === JSON.stringify(b.items)

async function runFooter(
  pack: InstalledConnectorPack,
  footerId: string,
  title: string,
  session: TerminalSession
): Promise<void> {
  const key = keyOf(session.id, pack.id, footerId)
  if (inFlight.has(key)) return
  inFlight.add(key)
  const poller = pollers.get(key)
  try {
    const client = await getOrStartHost(pack.id, session.projectPath)
    const answered = await client.callTool({
      name: `vorn_footer_${footerId}`,
      arguments: {
        sessionId: session.id,
        worktreePath: session.worktreePath ?? session.projectPath,
        agent: session.agentType
      }
    })
    if (answered.isError) throw new Error(String(answered.content ?? 'the footer failed'))
    const reading: ExtensionFooterReading = {
      extensionId: pack.id,
      extensionName: pack.name,
      footerId,
      title,
      items: readItems(answered.structuredContent),
      computedAt: new Date().toISOString()
    }
    if (poller?.failing) restart(pack, footerId, title, session, false)
    publish(session.id, key, reading)
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err)
    const previous = readings.get(key)
    publish(session.id, key, {
      extensionId: pack.id,
      extensionName: pack.name,
      footerId,
      title,
      items: previous?.items ?? [],
      error: message,
      computedAt: new Date().toISOString()
    })
    // Slowed rather than stopped: an extension that is fixed should recover on its own.
    if (poller && !poller.failing) restart(pack, footerId, title, session, true)
    log.warn(`[extensions] ${pack.id} ${footerId} failed: ${message}`)
  } finally {
    inFlight.delete(key)
  }
}

function publish(sessionId: string, key: string, reading: ExtensionFooterReading): void {
  // A run that was in flight when its session ended has nothing to say about it,
  // and storing what it computed would leave a reading nothing will ever clear.
  if (!pollers.has(key)) return
  if (same(readings.get(key), reading)) return
  readings.set(key, reading)
  clientRegistry.broadcast(
    IPC.EXTENSION_FOOTER_ITEMS,
    { sessionId, readings: footerReadings(sessionId) },
    sessionId
  )
}

function restart(
  pack: InstalledConnectorPack,
  footerId: string,
  title: string,
  session: TerminalSession,
  failing: boolean
): void {
  const key = keyOf(session.id, pack.id, footerId)
  const existing = pollers.get(key)
  if (!existing) return
  clearInterval(existing.timer)
  const everyMs = failing ? existing.everyMs * FAILURE_BACKOFF : existing.everyMs / FAILURE_BACKOFF
  const timer = setInterval(() => void runFooter(pack, footerId, title, session), everyMs)
  timer.unref?.()
  pollers.set(key, { timer, everyMs, failing, declared: existing.declared })
}

/** Start what this session's activation says shows, and stop what no longer does. */
export function syncFooters(session: TerminalSession): void {
  const subject = subjectOf(session)
  const wanted = new Set<string>()

  for (const pack of installedExtensions()) {
    const activation = activationFor(pack, subject)
    if (!activation.active) continue
    for (const footer of pack.contributes?.footers ?? []) {
      if (!activation.footers.includes(footer.id)) continue
      const key = keyOf(session.id, pack.id, footer.id)
      wanted.add(key)
      const everyMs = Math.max(footer.every, 5) * 1000
      const declared = { everyMs, title: footer.title, version: pack.version }
      const running = pollers.get(key)
      // An upgraded pack is a different footer, so what is running is replaced
      // rather than left declaring the version it started at.
      if (running) {
        if (
          running.declared.everyMs === declared.everyMs &&
          running.declared.title === declared.title &&
          running.declared.version === declared.version
        ) {
          continue
        }
        clearInterval(running.timer)
      }
      const timer = setInterval(
        () => void runFooter(pack, footer.id, footer.title, session),
        everyMs
      )
      timer.unref?.()
      pollers.set(key, { timer, everyMs, failing: false, declared })
      // Once now, so a card shows a reading rather than an empty band for the first interval.
      void runFooter(pack, footer.id, footer.title, session)
    }
  }

  for (const [key, poller] of [...pollers]) {
    if (key.startsWith(`${session.id} `) && !wanted.has(key)) {
      clearInterval(poller.timer)
      pollers.delete(key)
      readings.delete(key)
    }
  }
}

export function stopFooters(sessionId: string): void {
  for (const [key, poller] of [...pollers]) {
    if (!key.startsWith(`${sessionId} `)) continue
    clearInterval(poller.timer)
    pollers.delete(key)
    readings.delete(key)
  }
}

export function stopAllFooters(): void {
  for (const poller of pollers.values()) clearInterval(poller.timer)
  pollers.clear()
  readings.clear()
}
