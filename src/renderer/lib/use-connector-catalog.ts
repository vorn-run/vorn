import { useEffect, useState } from 'react'
import type {
  ConnectorCatalogItem,
  McpServerCatalogEntry,
  WorkflowTemplate
} from '../../shared/types'

/**
 * The published catalog, read once and shared.
 *
 * Two surfaces want it — the connector list and the step library, which offers
 * a connector's actions before anything is installed — and both mount and
 * unmount as panels. A module-level cache is what keeps that from being a fetch
 * per open, and keeps the second surface from waiting on a request the first
 * one already made.
 */
export interface CatalogSnapshot {
  items: ConnectorCatalogItem[]
  templates: WorkflowTemplate[]
  mcpServers: McpServerCatalogEntry[]
  fetchedAt?: number
}

const EMPTY: CatalogSnapshot = { items: [], templates: [], mcpServers: [] }

let cache: CatalogSnapshot | undefined
let inFlight: Promise<CatalogSnapshot> | undefined
const listeners = new Set<(snapshot: CatalogSnapshot) => void>()

/**
 * Ask once, and only where the answer is wanted.
 *
 * A failure is not cached: the server may simply not have been listening yet,
 * and the next panel that opens should ask again rather than inherit a silence.
 */
async function load(): Promise<CatalogSnapshot> {
  if (cache) return cache
  if (inFlight) return inFlight
  inFlight = Promise.resolve(window.api?.listConnectorCatalog?.())
    .then((snapshot) => {
      const next: CatalogSnapshot = {
        items: snapshot?.items ?? [],
        templates: snapshot?.templates ?? [],
        mcpServers: snapshot?.mcpServers ?? [],
        ...(snapshot?.fetchedAt !== undefined && { fetchedAt: snapshot.fetchedAt })
      }
      cache = next
      for (const listener of listeners) listener(next)
      return next
    })
    .catch(() => EMPTY)
    .finally(() => {
      inFlight = undefined
    })
  return inFlight
}

/**
 * What the catalog says right now, fetching it the first time anyone asks.
 *
 * `enabled` is how a surface says nobody is looking yet — a panel that is
 * closed should not send the request, and one that opens after a failed
 * attempt should send it again rather than inherit the silence.
 */
export function useConnectorCatalog(enabled: boolean = true): CatalogSnapshot {
  const [snapshot, setSnapshot] = useState<CatalogSnapshot>(() => cache ?? EMPTY)

  useEffect(() => {
    if (!enabled) return
    let live = true
    listeners.add(setSnapshot)
    void load().then((next) => {
      if (live) setSnapshot(next)
    })
    return () => {
      live = false
      listeners.delete(setSnapshot)
    }
  }, [enabled])

  return snapshot
}

/**
 * Go and ask the publisher again, and tell everyone what came back.
 *
 * This is what "Check now" means: not re-reading the copy this process already
 * has, which is what makes the button feel like it did nothing.
 */
export async function refreshConnectorCatalog(): Promise<CatalogSnapshot> {
  cache = undefined
  const fetched = await Promise.resolve(window.api?.refreshConnectorCatalog?.()).catch(
    () => undefined
  )
  if (!fetched) return load()
  const next: CatalogSnapshot = {
    items: fetched.items ?? [],
    templates: fetched.templates ?? [],
    mcpServers: fetched.mcpServers ?? [],
    ...(fetched.fetchedAt !== undefined && { fetchedAt: fetched.fetchedAt })
  }
  cache = next
  for (const listener of listeners) listener(next)
  return next
}

/** Test seam: forget what this process has read. */
export function __resetCatalogCacheForTests(): void {
  cache = undefined
  inFlight = undefined
  listeners.clear()
}
