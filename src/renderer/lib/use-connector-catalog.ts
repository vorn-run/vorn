import { useEffect, useState } from 'react'
import type {
  ConnectorCatalogItem,
  McpServerCatalogEntry,
  WorkflowTemplate
} from '../../shared/types'

// The published catalog, read once and shared by every panel that wants it.
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

// Keep what came back and tell everyone; the one place a snapshot is shaped.
function publish(raw: Partial<CatalogSnapshot> | undefined): CatalogSnapshot {
  const next: CatalogSnapshot = {
    items: raw?.items ?? [],
    templates: raw?.templates ?? [],
    mcpServers: raw?.mcpServers ?? [],
    ...(raw?.fetchedAt !== undefined && { fetchedAt: raw.fetchedAt })
  }
  cache = next
  for (const listener of listeners) listener(next)
  return next
}

// Ask once, only where wanted; a failure is not cached, so the next panel to open asks again.
async function load(): Promise<CatalogSnapshot> {
  if (cache) return cache
  if (inFlight) return inFlight
  inFlight = Promise.resolve(window.api?.listConnectorCatalog?.())
    .then(publish)
    .catch(() => EMPTY)
    .finally(() => {
      inFlight = undefined
    })
  return inFlight
}

// `enabled` is how a closed panel says nobody is looking yet.
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

// "Check now": ask the publisher again rather than re-read the copy this process holds.
export async function refreshConnectorCatalog(): Promise<CatalogSnapshot> {
  cache = undefined
  const fetched = await Promise.resolve(window.api?.refreshConnectorCatalog?.()).catch(
    () => undefined
  )
  return fetched ? publish(fetched) : load()
}

/** Test seam: forget what this process has read. */
export function __resetCatalogCacheForTests(): void {
  cache = undefined
  inFlight = undefined
  listeners.clear()
}
