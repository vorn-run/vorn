const VIEWS_KEY = 'vorn:canvasViews'
const OUTLINE_KEY = 'vorn:workflowOutline'
/** How many workflows keep a remembered view; the one looked at longest ago goes first. */
const KEPT_VIEWS = 50

/** Where a workflow's canvas was last looked at: the pan in screen pixels and the zoom. */
export interface CanvasView {
  x: number
  y: number
  zoom: number
}

interface StoredView extends CanvasView {
  at: number
}

function load(): Record<string, StoredView> {
  try {
    const raw = localStorage.getItem(VIEWS_KEY)
    if (!raw) return {}
    const parsed = JSON.parse(raw) as unknown
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {}
    const out: Record<string, StoredView> = {}
    for (const [id, value] of Object.entries(parsed as Record<string, unknown>)) {
      const view = value as Partial<StoredView>
      const numbers = [view?.x, view?.y, view?.zoom, view?.at]
      if (!numbers.every((n) => typeof n === 'number' && Number.isFinite(n))) continue
      if ((view.zoom as number) <= 0) continue
      out[id] = view as StoredView
    }
    return out
  } catch {
    return {}
  }
}

function save(views: Record<string, StoredView>): void {
  try {
    localStorage.setItem(VIEWS_KEY, JSON.stringify(views))
  } catch {
    /* a full or blocked store only costs the remembered view */
  }
}

/** Where a workflow was last looked at, or null the first time it opens here. */
export function readCanvasView(workflowId: string): CanvasView | null {
  const view = load()[workflowId]
  return view ? { x: view.x, y: view.y, zoom: view.zoom } : null
}

export function writeCanvasView(workflowId: string, view: CanvasView, now = Date.now()): void {
  const views = load()
  views[workflowId] = { x: view.x, y: view.y, zoom: view.zoom, at: now }
  const ids = Object.keys(views)
  if (ids.length > KEPT_VIEWS) {
    ids.sort((a, b) => views[a].at - views[b].at)
    for (const id of ids.slice(0, ids.length - KEPT_VIEWS)) delete views[id]
  }
  save(views)
}

/** Drop the views of workflows that no longer exist. */
export function pruneCanvasViews(liveWorkflowIds: Set<string>): void {
  const views = load()
  const dead = Object.keys(views).filter((id) => !liveWorkflowIds.has(id))
  if (dead.length === 0) return
  for (const id of dead) delete views[id]
  save(views)
}

/** Whether the step outline is showing beside the canvas; it is until someone closes it. */
export function readOutlineOpen(): boolean {
  try {
    return localStorage.getItem(OUTLINE_KEY) !== '0'
  } catch {
    return true
  }
}

export function writeOutlineOpen(open: boolean): void {
  try {
    localStorage.setItem(OUTLINE_KEY, open ? '1' : '0')
  } catch {
    /* the outline just opens again next time */
  }
}
