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

/** What the catalog says right now, fetching it the first time anyone asks. */
export function useConnectorCatalog(): CatalogSnapshot {
  const [snapshot, setSnapshot] = useState<CatalogSnapshot>(() => cache ?? EMPTY)

  useEffect(() => {
    let live = true
    listeners.add(setSnapshot)
    void load().then((next) => {
      if (live) setSnapshot(next)
    })
    return () => {
      live = false
      listeners.delete(setSnapshot)
    }
  }, [])

  return snapshot
}

/** Read it again after something that changes what is published or installed. */
export async function refreshConnectorCatalog(): Promise<void> {
  cache = undefined
  await load()
}

/** Test seam: forget what this process has read. */
export function __resetCatalogCacheForTests(): void {
  cache = undefined
  inFlight = undefined
  listeners.clear()
}
