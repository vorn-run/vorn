import type { Viewport } from '@xyflow/react'

const VIEWS_KEY = 'vorn:canvasViews'
/** How many workflows keep a remembered view; the one looked at longest ago goes first. */
const KEPT_VIEWS = 50

function load(): Record<string, Viewport> {
  try {
    const parsed = JSON.parse(localStorage.getItem(VIEWS_KEY) ?? '{}') as unknown
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {}
    const views: Record<string, Viewport> = {}
    for (const [id, value] of Object.entries(parsed as Record<string, Partial<Viewport>>)) {
      const { x, y, zoom } = value ?? {}
      if (typeof x !== 'number' || typeof y !== 'number' || typeof zoom !== 'number') continue
      if (Number.isFinite(x) && Number.isFinite(y) && Number.isFinite(zoom) && zoom > 0) {
        views[id] = { x, y, zoom }
      }
    }
    return views
  } catch {
    return {}
  }
}

/** Where a workflow was last looked at, or null the first time it opens here. */
export function readCanvasView(workflowId: string): Viewport | null {
  return load()[workflowId] ?? null
}

/** Remember a workflow's view; stored in the order they were looked at, oldest first. */
export function writeCanvasView(workflowId: string, view: Viewport): void {
  const views = load()
  delete views[workflowId]
  views[workflowId] = { x: view.x, y: view.y, zoom: view.zoom }
  const kept = Object.fromEntries(Object.entries(views).slice(-KEPT_VIEWS))
  try {
    localStorage.setItem(VIEWS_KEY, JSON.stringify(kept))
  } catch {
    /* a full or blocked store only costs the remembered view */
  }
}
