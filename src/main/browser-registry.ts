import { webContents } from 'electron'
import type { WebContents } from 'electron'
import { normalizeUrl } from '../shared/browser-url'
import type {
  BrowserNode,
  BrowserSelection,
  BrowserAnnotation,
  BrowserStroke,
  BrowserPageRead,
  BrowserConsoleMessage,
  BrowserNetworkRequest,
  BrowserTarget
} from '../shared/types'
import log from './logger'

/**
 * The agent's handle on a session's browser pane.
 *
 * A `<webview>` guest is only addressable from main, and it carries no session
 * identity of its own — the element has a partition string and nothing else. So
 * the renderer tells us which `webContentsId` belongs to which session as soon
 * as the guest attaches, and everything downstream keys off that.
 *
 * Driving the page goes through CDP rather than executing JavaScript in it, for
 * three reasons: `Input.dispatchMouseEvent` produces events with `isTrusted`
 * set, which page-synthesised ones cannot fake and which some frameworks check;
 * `hardenWebviews` strips `preload` and forces `contextIsolation` on every
 * guest, so a script channel would mean walking that hardening back; and the
 * CDP domains already cover almost every tool we want.
 */

/** How many AX nodes one `read_page` returns before handing back a cursor. */
const PAGE_BUDGET = 200

/** Ring buffer depth for console and network capture, per session. */
const EVENT_BUFFER = 200

export interface Entry {
  webContentsId: number
  attached: boolean
  /**
   * Bumped on every navigation and document replacement. Refs are stamped with
   * the generation that produced them, so a ref held across a navigation is
   * refused rather than silently resolving to whatever now sits at that node id.
   */
  generation: number
  refs: Map<string, number>
  console: BrowserConsoleMessage[]
  network: BrowserNetworkRequest[]
}

const entries = new Map<string, Entry>()

function contentsFor(sessionId: string): { wc: WebContents; entry: Entry } {
  const entry = entries.get(sessionId)
  if (!entry) {
    throw new Error(
      'This session has no browser pane open. Open one from the session card, then retry.'
    )
  }
  const wc = webContents.fromId(entry.webContentsId)
  if (!wc || wc.isDestroyed()) {
    entries.delete(sessionId)
    throw new Error('This session’s browser pane is no longer available.')
  }
  return { wc, entry }
}

async function send<T = unknown>(
  wc: WebContents,
  method: string,
  params?: Record<string, unknown>
): Promise<T> {
  return (await wc.debugger.sendCommand(method, params)) as T
}

/**
 * Start driving a session's guest. Called by the renderer once the `<webview>`
 * has attached and can report its `webContentsId`.
 */
export function attach(sessionId: string, webContentsId: number): void {
  const existing = entries.get(sessionId)
  if (existing?.webContentsId === webContentsId && existing.attached) return

  // A pane that navigated to a new guest replaces the old entry outright; its
  // refs belong to a document that no longer exists.
  if (existing) detach(sessionId)

  const wc = webContents.fromId(webContentsId)
  if (!wc || wc.isDestroyed()) return

  const entry: Entry = {
    webContentsId,
    attached: false,
    generation: 1,
    refs: new Map(),
    console: [],
    network: []
  }
  entries.set(sessionId, entry)

  try {
    if (!wc.debugger.isAttached()) wc.debugger.attach('1.3')
    entry.attached = true
  } catch (err) {
    log.warn({ err }, `[browser] could not attach debugger for session ${sessionId}`)
    return
  }

  wc.debugger.on('message', (_e, method, params) => {
    const p = params as Record<string, unknown>
    if (method === 'Runtime.consoleAPICalled') {
      const args = (p.args as Array<{ value?: unknown; description?: string }>) ?? []
      push(entry.console, {
        level: String(p.type ?? 'log'),
        text: args
          .map((a) => (a.value !== undefined ? String(a.value) : (a.description ?? '')))
          .join(' '),
        timestamp: Date.now()
      })
    } else if (method === 'Network.requestWillBeSent') {
      const req = p.request as { method?: string; url?: string } | undefined
      push(entry.network, {
        method: String(req?.method ?? 'GET'),
        url: String(req?.url ?? ''),
        timestamp: Date.now()
      })
    } else if (method === 'Network.responseReceived') {
      const res = p.response as { url?: string; status?: number } | undefined
      const hit = entry.network.find((r) => r.url === res?.url && r.status === undefined)
      if (hit) hit.status = res?.status
    } else if (method === 'Page.frameNavigated' || method === 'Page.loadEventFired') {
      // The document the refs pointed into is gone. Bumping here is what makes
      // a stale ref fail loudly instead of resolving against a fresh tree.
      entry.generation++
      entry.refs.clear()
    }
  })

  for (const domain of ['DOM', 'Runtime', 'Page', 'Network', 'Accessibility']) {
    send(wc, `${domain}.enable`).catch((err) => {
      log.warn({ err }, `[browser] ${domain}.enable failed for session ${sessionId}`)
    })
  }
}

function push<T>(buf: T[], item: T): void {
  buf.push(item)
  if (buf.length > EVENT_BUFFER) buf.shift()
}

/** Stop driving a session's guest. Safe to call for a session with no entry. */
export function detach(sessionId: string): void {
  const entry = entries.get(sessionId)
  if (!entry) return
  entries.delete(sessionId)
  const wc = webContents.fromId(entry.webContentsId)
  if (!wc || wc.isDestroyed()) return
  try {
    if (wc.debugger.isAttached()) wc.debugger.detach()
  } catch (err) {
    // A crashed or already-gone guest detaches itself; nothing left to release.
    log.debug({ err }, `[browser] detach for session ${sessionId} was already done`)
  }
}

/** Every attached session. Used to reconcile on window teardown. */
export function attachedSessions(): string[] {
  return Array.from(entries.keys())
}

// ─── Reads ──────────────────────────────────────────────────────

export interface AXNode {
  nodeId: string
  backendDOMNodeId?: number
  ignored?: boolean
  role?: { value?: string }
  name?: { value?: string }
  value?: { value?: string }
  properties?: Array<{ name: string; value?: { value?: unknown } }>
}

/** Roles worth handing the agent a ref for — the things you can act on. */
const INTERACTIVE = new Set([
  'button',
  'link',
  'textbox',
  'searchbox',
  'combobox',
  'checkbox',
  'radio',
  'switch',
  'slider',
  'menuitem',
  'menuitemcheckbox',
  'menuitemradio',
  'option',
  'tab',
  'spinbutton'
])

export function toNode(ax: AXNode, entry: Entry): BrowserNode | null {
  const role = ax.role?.value
  if (!role || ax.ignored) return null
  const name = ax.name?.value
  const interactive = INTERACTIVE.has(role)
  // A non-interactive node with nothing to read contributes only noise, and
  // the node budget is the scarce resource here.
  if (!interactive && !name) return null

  const node: BrowserNode = { role }
  if (name) node.name = name
  const value = ax.value?.value
  if (value) node.value = String(value)

  const disabled = ax.properties?.find((p) => p.name === 'disabled')?.value?.value
  if (disabled === true) node.disabled = true

  if (interactive && ax.backendDOMNodeId !== undefined) {
    const ref = `ref_${entry.refs.size + 1}`
    entry.refs.set(ref, ax.backendDOMNodeId)
    node.ref = ref
  }
  return node
}

/** A blank registry entry. Exported so tests can drive the pure helpers. */
export function newEntry(webContentsId = 0): Entry {
  return {
    webContentsId,
    attached: false,
    generation: 1,
    refs: new Map(),
    console: [],
    network: []
  }
}

export async function readPage(params: {
  sessionId: string
  filter?: 'interactive' | 'all'
  cursor?: string
  limit?: number
}): Promise<BrowserPageRead> {
  const { wc, entry } = contentsFor(params.sessionId)
  const { nodes: ax } = await send<{ nodes: AXNode[] }>(wc, 'Accessibility.getFullAXTree')

  const start = parseCursor(params.cursor, entry.generation)
  const budget = Math.min(params.limit ?? PAGE_BUDGET, PAGE_BUDGET)
  const wantAll = params.filter === 'all'

  const out: BrowserNode[] = []
  let i = start
  for (; i < ax.length && out.length < budget; i++) {
    const node = toNode(ax[i], entry)
    if (!node) continue
    if (!wantAll && !node.ref) continue
    out.push(node)
  }

  return {
    url: wc.getURL(),
    title: wc.getTitle(),
    nodes: out,
    generation: entry.generation,
    ...(i < ax.length ? { nextCursor: `${entry.generation}:${i}` } : {})
  }
}

/**
 * A cursor is only meaningful against the tree that produced it. Rather than
 * silently resuming at an offset into a different document, a cursor from an
 * older generation restarts the read — the agent gets the page from the top,
 * which is recoverable, instead of a silently truncated middle.
 */
export function parseCursor(cursor: string | undefined, generation: number): number {
  if (!cursor) return 0
  const [gen, idx] = cursor.split(':')
  if (Number(gen) !== generation) return 0
  const n = Number(idx)
  return Number.isFinite(n) && n >= 0 ? n : 0
}

/** Character budget for one `get_page_text` page. */
const TEXT_BUDGET = 20_000

export async function getText(params: {
  sessionId: string
  cursor?: string
}): Promise<{ url: string; text: string; nextCursor?: string }> {
  const { wc, entry } = contentsFor(params.sessionId)
  const { result } = await send<{ result: { value?: string } }>(wc, 'Runtime.evaluate', {
    expression: 'document.body ? document.body.innerText : ""',
    returnByValue: true
  })
  const full = result.value ?? ''
  const start = parseCursor(params.cursor, entry.generation)
  const slice = full.slice(start, start + TEXT_BUDGET)
  const end = start + slice.length
  return {
    url: wc.getURL(),
    text: slice,
    ...(end < full.length ? { nextCursor: `${entry.generation}:${end}` } : {})
  }
}

export function consoleMessages(params: {
  sessionId: string
  limit?: number
}): BrowserConsoleMessage[] {
  const { entry } = contentsFor(params.sessionId)
  return entry.console.slice(-(params.limit ?? 50))
}

export function networkRequests(params: {
  sessionId: string
  limit?: number
}): BrowserNetworkRequest[] {
  const { entry } = contentsFor(params.sessionId)
  return entry.network.slice(-(params.limit ?? 50))
}

export async function screenshot(params: {
  sessionId: string
  fullPage?: boolean
}): Promise<{ data: string }> {
  const { wc } = contentsFor(params.sessionId)
  const { data } = await send<{ data: string }>(wc, 'Page.captureScreenshot', {
    format: 'png',
    captureBeyondViewport: params.fullPage === true
  })
  return { data }
}

export async function find(params: {
  sessionId: string
  text: string
  limit?: number
}): Promise<BrowserNode[]> {
  const page = await readPage({ sessionId: params.sessionId, filter: 'all' })
  const needle = params.text.toLowerCase()
  return page.nodes
    .filter((n) => (n.name ?? '').toLowerCase().includes(needle))
    .slice(0, params.limit ?? 20)
}

// ─── Interaction ────────────────────────────────────────────────

/** Viewport coordinates of a ref, or a hard failure if it no longer resolves. */
async function pointFor(
  wc: WebContents,
  entry: Entry,
  target: BrowserTarget
): Promise<{ x: number; y: number }> {
  if ('x' in target) return { x: target.x, y: target.y }

  const backendNodeId = entry.refs.get(target.ref)
  if (backendNodeId === undefined) {
    // Deliberately fatal. Falling back to a remembered coordinate would click
    // whatever now occupies that spot, which is the worst outcome available:
    // it looks like it worked and acts on the wrong thing.
    throw new Error(
      `Ref "${target.ref}" is stale — the page changed since it was read. Call read_page again for fresh refs.`
    )
  }

  const { model } = await send<{ model?: { content: number[] } }>(wc, 'DOM.getBoxModel', {
    backendNodeId
  })
  if (!model) {
    throw new Error(`Ref "${target.ref}" is not visible on the page.`)
  }
  const [x1, y1, x2, , , y3] = model.content
  return { x: (x1 + x2) / 2, y: (y1 + y3) / 2 }
}

export async function interact(params: {
  sessionId: string
  action: 'click' | 'hover' | 'type' | 'key' | 'scroll'
  target?: BrowserTarget
  text?: string
  deltaY?: number
}): Promise<{ ok: true }> {
  const { wc, entry } = contentsFor(params.sessionId)

  if (params.action === 'type') {
    if (params.target) {
      const pt = await pointFor(wc, entry, params.target)
      await click(wc, pt)
    }
    for (const ch of params.text ?? '') {
      await send(wc, 'Input.dispatchKeyEvent', { type: 'char', text: ch })
    }
    return { ok: true }
  }

  if (params.action === 'key') {
    const key = params.text ?? ''
    await send(wc, 'Input.dispatchKeyEvent', { type: 'keyDown', key, windowsVirtualKeyCode: 0 })
    await send(wc, 'Input.dispatchKeyEvent', { type: 'keyUp', key })
    return { ok: true }
  }

  if (params.action === 'scroll') {
    const pt = params.target ? await pointFor(wc, entry, params.target) : { x: 10, y: 10 }
    await send(wc, 'Input.dispatchMouseEvent', {
      type: 'mouseWheel',
      x: pt.x,
      y: pt.y,
      deltaX: 0,
      deltaY: params.deltaY ?? 400
    })
    return { ok: true }
  }

  if (!params.target) throw new Error(`"${params.action}" needs a target (a ref or x/y).`)
  const pt = await pointFor(wc, entry, params.target)

  if (params.action === 'hover') {
    await send(wc, 'Input.dispatchMouseEvent', {
      type: 'mouseMoved',
      x: pt.x,
      y: pt.y,
      button: 'none'
    })
    return { ok: true }
  }

  await click(wc, pt)
  return { ok: true }
}

async function click(wc: WebContents, pt: { x: number; y: number }): Promise<void> {
  const base = { x: pt.x, y: pt.y, button: 'left' as const, clickCount: 1 }
  await send(wc, 'Input.dispatchMouseEvent', { type: 'mousePressed', ...base })
  await send(wc, 'Input.dispatchMouseEvent', { type: 'mouseReleased', ...base })
}

/**
 * Navigate the pane. Runs the same `normalizeUrl` the address bar does, so the
 * schemes a person cannot type here (`file:`, `javascript:`, `data:`) are the
 * same ones an agent cannot reach.
 */
export async function navigate(params: {
  sessionId: string
  url: string
}): Promise<{ url: string }> {
  const { wc } = contentsFor(params.sessionId)
  const normalized = normalizeUrl(params.url)
  if (!normalized) {
    throw new Error(`Refusing to navigate to "${params.url}" — not an allowed web address.`)
  }
  await send(wc, 'Page.navigate', { url: normalized })
  return { url: normalized }
}

// ─── Element picker ─────────────────────────────────────────────

/**
 * "This one" — pointed at by the person, handed to the agent.
 *
 * Deliberately user-initiated only. There is no agent-callable variant: a tool
 * that lets a background agent seize the pointer and wait on a human is a
 * nuisance at best, and this direction (person marks, agent acts) is the one we
 * have actually seen people want.
 *
 * The highlight comes from CDP's `Overlay` domain rather than an injected
 * element, so it cannot be styled away by the page, never appears in the page's
 * own DOM, and needs no cleanup if the guest dies mid-pick.
 */

/** How much of an element's markup travels. Enough to identify, not to dump. */
const HTML_BUDGET = 2000

/** Session → how to disarm its in-progress pick. */
const pickers = new Map<string, () => void>()

/**
 * Read everything identifying about one node, in the page.
 *
 * This is the one place a script runs inside the guest, because the facts it
 * gathers — the React fiber, `elementsFromPoint`, a computed selector — have no
 * CDP equivalent. It reads only; it never writes to the page.
 */
const DESCRIBE_FN = `function () {
  const el = this
  const r = el.getBoundingClientRect()
  const path = (n) => {
    const parts = []
    for (; n && n.nodeType === 1 && parts.length < 8; n = n.parentElement) {
      let p = n.tagName.toLowerCase()
      if (n.id) { parts.unshift(p + '#' + n.id); break }
      if (n.classList.length) p += '.' + Array.from(n.classList).slice(0, 3).join('.')
      parts.unshift(p)
    }
    return parts.join(' > ')
  }
  // React hangs its fiber off a randomly-suffixed key, so it has to be found
  // by prefix. Absent in production builds, which is why nothing requires it.
  let componentName, source
  const key = Object.keys(el).find((k) => k.startsWith('__reactFiber$'))
  if (key) {
    let f = el[key]
    for (let i = 0; f && i < 10; i++, f = f.return) {
      if (typeof f.type === 'function') { componentName = f.type.displayName || f.type.name; break }
      if (f._debugSource && !source) source = f._debugSource.fileName + ':' + f._debugSource.lineNumber
    }
  }
  return JSON.stringify({
    url: location.href,
    rect: { x: r.x, y: r.y, width: r.width, height: r.height },
    text: (el.innerText || el.value || el.getAttribute('aria-label') || '').slice(0, 500).trim(),
    selector: path(el),
    outerHTML: el.outerHTML.slice(0, ${HTML_BUDGET}),
    tagName: el.tagName.toLowerCase(),
    id: el.id || undefined,
    classes: el.classList.length ? Array.from(el.classList) : undefined,
    componentName,
    source
  })
}`

/**
 * Arm the picker. Resolves when the person clicks an element, or rejects if
 * they press escape — a cancelled pick must not look like an empty one.
 */
export async function startPick(params: { sessionId: string }): Promise<BrowserSelection> {
  const { wc, entry } = contentsFor(params.sessionId)
  cancelPick(params.sessionId)

  await send(wc, 'Overlay.enable')
  await send(wc, 'Overlay.setInspectMode', {
    mode: 'searchForNode',
    highlightConfig: {
      contentColor: { r: 111, g: 168, b: 220, a: 0.35 },
      borderColor: { r: 111, g: 168, b: 220, a: 0.9 }
    }
  })

  return new Promise<BrowserSelection>((resolve, reject) => {
    const onMessage = (_e: unknown, method: string, p: unknown): void => {
      if (method !== 'Overlay.inspectNodeRequested') return
      wc.debugger.off('message', onMessage)
      pickers.delete(params.sessionId)
      const backendNodeId = (p as { backendNodeId: number }).backendNodeId
      void describe(wc, entry, backendNodeId).then(resolve, reject)
      void send(wc, 'Overlay.setInspectMode', { mode: 'none', highlightConfig: {} })
    }
    wc.debugger.on('message', onMessage)
    pickers.set(params.sessionId, () => {
      wc.debugger.off('message', onMessage)
      reject(new Error('Selection cancelled'))
    })
  })
}

/** Disarm a pick in progress. Safe when nothing is armed. */
export function cancelPick(sessionId: string): void {
  const cancel = pickers.get(sessionId)
  pickers.delete(sessionId)
  cancel?.()
  const entry = entries.get(sessionId)
  if (!entry) return
  const wc = webContents.fromId(entry.webContentsId)
  if (!wc || wc.isDestroyed()) return
  send(wc, 'Overlay.setInspectMode', { mode: 'none', highlightConfig: {} }).catch(() => {
    // The guest went away mid-pick; there is no overlay left to turn off.
  })
}

async function describe(
  wc: WebContents,
  entry: Entry,
  backendNodeId: number
): Promise<BrowserSelection> {
  const { object } = await send<{ object: { objectId: string } }>(wc, 'DOM.resolveNode', {
    backendNodeId
  })
  const { result } = await send<{ result: { value?: string } }>(wc, 'Runtime.callFunctionOn', {
    objectId: object.objectId,
    functionDeclaration: DESCRIBE_FN,
    returnByValue: true
  })
  const selection = JSON.parse(result.value ?? '{}') as BrowserSelection

  // A ref alongside the description, so the agent can act on what was picked
  // without a round trip through read_page.
  const ref = `ref_${entry.refs.size + 1}`
  entry.refs.set(ref, backendNodeId)
  ;(selection as BrowserSelection & { ref?: string }).ref = ref

  // A crop of just this element. Cheap — one node, not a page — and it settles
  // questions about rendering that markup alone cannot.
  try {
    const { data } = await send<{ data: string }>(wc, 'Page.captureScreenshot', {
      format: 'png',
      clip: { ...selection.rect, scale: 1 }
    })
    selection.screenshot = data
  } catch (err) {
    // A zero-area or off-screen element cannot be clipped; the description
    // alone is still worth returning.
    log.debug({ err }, '[browser] element screenshot skipped')
  }
  return selection
}

// ─── Annotation ─────────────────────────────────────────────────

/**
 * Resolve freehand ink to the elements it covers.
 *
 * Strokes arrive in **page** coordinates, not window ones. That distinction is
 * the whole trap here: on a layout with an inner scrolling container, window
 * coordinates resolve to whatever happens to be scrolled into that position at
 * lookup time, and it fails *silently* — you get plausible elements that are
 * simply the wrong ones. The renderer converts before sending.
 */
export async function annotate(params: {
  sessionId: string
  strokes: BrowserStroke[]
}): Promise<BrowserAnnotation> {
  const { wc } = contentsFor(params.sessionId)
  const viewport = params.strokes.flatMap((s) => s.points)
  if (viewport.length === 0) throw new Error('No strokes to resolve.')

  // Ink is drawn over the pane, so it arrives in *viewport* coordinates. The
  // crop needs page coordinates, and the two differ by the scroll offset at the
  // moment of drawing — so read it once, here, rather than threading a scroll
  // offset through the renderer where it would drift out of date between the
  // last stroke and the send.
  const { result: sc } = await send<{ result: { value: { x: number; y: number } } }>(
    wc,
    'Runtime.evaluate',
    { expression: '({x: scrollX, y: scrollY})', returnByValue: true }
  )
  const points = viewport.map((p) => ({ x: p.x + sc.value.x, y: p.y + sc.value.y }))

  const xs = points.map((p) => p.x)
  const ys = points.map((p) => p.y)
  const bounds = {
    x: Math.min(...xs),
    y: Math.min(...ys),
    width: Math.max(1, Math.max(...xs) - Math.min(...xs)),
    height: Math.max(1, Math.max(...ys) - Math.min(...ys))
  }

  // Sampled rather than exhaustive: a stroke can carry hundreds of points and
  // consecutive ones almost always land on the same element.
  const sampled = samplePoints(points, 24)
  const { result } = await send<{ result: { value?: string } }>(wc, 'Runtime.evaluate', {
    expression: `(${HIT_TEST_FN})(${JSON.stringify(sampled)})`,
    returnByValue: true
  })
  const hits = JSON.parse(result.value ?? '[]') as Array<{
    role: string
    name?: string
    selector: string
  }>

  const annotation: BrowserAnnotation = {
    url: wc.getURL(),
    elements: hits.map((h) => ({ role: h.role, name: h.name ?? h.selector })),
    image: '',
    bounds
  }

  try {
    const { data } = await send<{ data: string }>(wc, 'Page.captureScreenshot', {
      format: 'png',
      captureBeyondViewport: true
    })
    annotation.image = data
  } catch (err) {
    log.debug({ err }, '[browser] annotation page capture skipped')
  }

  // A tight crop of the marked region. The full page shows where the ink is;
  // the crop shows what it says, at a size the model can actually resolve.
  try {
    const pad = 16
    const { data } = await send<{ data: string }>(wc, 'Page.captureScreenshot', {
      format: 'png',
      captureBeyondViewport: true,
      clip: {
        x: Math.max(0, bounds.x - pad),
        y: Math.max(0, bounds.y - pad),
        width: bounds.width + pad * 2,
        height: bounds.height + pad * 2,
        scale: 1
      }
    })
    annotation.crop = data
  } catch (err) {
    log.debug({ err }, '[browser] annotation crop skipped')
  }
  return annotation
}

/** Thin a stroke down to at most `max` evenly spaced points, ends included. */
export function samplePoints<T>(points: T[], max: number): T[] {
  if (points.length <= max) return points
  const step = (points.length - 1) / (max - 1)
  return Array.from({ length: max }, (_, i) => points[Math.round(i * step)])
}

/**
 * Identify the topmost element at each inked point, deduplicated.
 *
 * `elementsFromPoint` takes *viewport* coordinates, so page coordinates are
 * converted back here against the current scroll — one conversion, in one
 * place, instead of a scroll offset threaded through the whole path.
 */
const HIT_TEST_FN = `function (pts) {
  const seen = new Set()
  const out = []
  for (const p of pts) {
    const el = document.elementFromPoint(p.x - scrollX, p.y - scrollY)
    if (!el) continue
    let sel = el.tagName.toLowerCase()
    if (el.id) sel += '#' + el.id
    else if (el.classList.length) sel += '.' + Array.from(el.classList).slice(0, 2).join('.')
    if (seen.has(sel)) continue
    seen.add(sel)
    out.push({
      role: el.getAttribute('role') || el.tagName.toLowerCase(),
      name: (el.getAttribute('aria-label') || el.innerText || '').slice(0, 120).trim() || undefined,
      selector: sel
    })
  }
  return JSON.stringify(out)
}`
