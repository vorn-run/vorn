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
// Bumped by a refresh, so a read that started before it cannot overwrite what the refresh brought back.
let generation = 0
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
  const started = generation
  inFlight = Promise.resolve(window.api?.listConnectorCatalog?.())
    // No answer is a build that cannot ask yet, not an empty catalog: nothing is kept.
    .then((raw) => (raw && started === generation ? publish(raw) : (cache ?? EMPTY)))
    .catch(() => EMPTY)
    .finally(() => {
      inFlight = undefined
    })
  return inFlight
}

// Subscribed once, so a catalog the server refreshed in the background reaches a list already on screen.
let unsubscribeCatalogChange: (() => void) | undefined
function watchForChanges(): void {
  if (unsubscribeCatalogChange || typeof window.api?.onConnectorCatalogChanged !== 'function')
    return
  unsubscribeCatalogChange = window.api.onConnectorCatalogChanged((next) => {
    // Newer than any read in flight, which must not overwrite it.
    generation += 1
    inFlight = undefined
    if (next) publish(next)
    else {
      cache = undefined
      void load()
    }
  })
}

// `enabled` is how a closed panel says nobody is looking yet.
export function useConnectorCatalog(enabled: boolean = true): CatalogSnapshot {
  const [snapshot, setSnapshot] = useState<CatalogSnapshot>(() => cache ?? EMPTY)

  useEffect(() => {
    if (!enabled) return
    let live = true
    watchForChanges()
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
  generation += 1
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
  unsubscribeCatalogChange?.()
  unsubscribeCatalogChange = undefined
}
