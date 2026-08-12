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
import { IPC } from '../shared/types'
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
  /** Held so `detach` can remove it. An anonymous listener would accumulate one
   *  copy per re-attach, each closing over an orphaned entry. */
  onCdpMessage?: (e: unknown, method: string, params: unknown) => void
  /** Held for the same reason, for the debugger's own detach event. */
  onDebuggerDetach?: () => void
  console: BrowserConsoleMessage[]
  network: BrowserNetworkRequest[]
}

const entries = new Map<string, Entry>()

/**
 * How main asks the renderer to do something to a pane.
 *
 * Panes are renderer state — a `<webview>` element in a React tree — so main
 * can create a guest for a session only by asking. Injected rather than
 * imported from `index.ts` to keep the dependency pointing one way.
 */
type RendererSend = (channel: string, params: unknown) => void
let sendToRenderer: RendererSend = () => {}
export function setRendererSend(fn: RendererSend): void {
  sendToRenderer = fn
}

/** Wait for a session's guest to attach, or give up. */
function waitForAttach(sessionId: string, timeoutMs = 8000): Promise<void> {
  const start = Date.now()
  return new Promise((resolve, reject) => {
    const tick = (): void => {
      const entry = entries.get(sessionId)
      if (entry?.attached) return resolve()
      if (Date.now() - start > timeoutMs) {
        // Better a stated timeout than a tool that hangs until the agent's own
        // deadline fires with nothing to say about why.
        return reject(new Error('The browser pane did not finish opening in time.'))
      }
      setTimeout(tick, 100)
    }
    tick()
  })
}

/**
 * Open the session's browser pane, or point the existing one at `url`.
 *
 * This is what makes the agent self-sufficient: before it existed, every
 * browser tool depended on a person having clicked the pane open first, so an
 * agent told "go read this page" could only ask for help.
 */
export async function openPane(
  params: { sessionId: string; url?: string },
  timeoutMs?: number
): Promise<{ url: string }> {
  const normalized = params.url === undefined ? undefined : normalizeUrl(params.url)
  if (params.url !== undefined && !normalized) {
    throw new Error(`Refusing to open "${params.url}" — not an allowed web address.`)
  }
  sendToRenderer(IPC.BROWSER_OPEN_PANE, { sessionId: params.sessionId, url: normalized })
  await waitForAttach(params.sessionId, timeoutMs)
  return { url: normalized ?? '' }
}

/**
 * Add, close, or switch a tab.
 *
 * Tab bookkeeping lives entirely in the renderer store, so this forwards and
 * does not try to mirror it here — two copies of that state would drift the
 * first time a person clicked a tab themselves.
 */
export async function tabs(params: {
  sessionId: string
  action: 'add' | 'close' | 'select'
  url?: string
  index?: number
}): Promise<{ ok: true }> {
  // Throws if the session has no pane, which is the honest answer for a tab
  // command: there is nothing to add a tab to.
  contentsFor(params.sessionId)
  if (params.action === 'add' && params.url !== undefined && !normalizeUrl(params.url)) {
    throw new Error(`Refusing to open "${params.url}" — not an allowed web address.`)
  }
  if (params.action !== 'add' && typeof params.index !== 'number') {
    throw new Error(`A tab index is required to ${params.action} a tab.`)
  }
  sendToRenderer(IPC.BROWSER_TAB_COMMAND, params)
  return { ok: true }
}

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
  if (!entry.attached) {
    // The guest is alive but we are no longer driving it — it crashed and
    // reloaded, or DevTools took the debugger. Saying so beats every
    // subsequent command failing with a CDP transport error.
    throw new Error(
      'This session’s browser pane is not currently driveable (it may have crashed or have ' +
        'DevTools open). Reload the pane and retry.'
    )
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
    // Leaving the entry behind would make `contentsFor` hand every tool a
    // handle to a debugger that was never attached, and each command would
    // reject with something far less clear than "no pane".
    entries.delete(sessionId)
    return
  }

  // Chromium drops our CDP session when the guest crashes, and when the user
  // opens DevTools on it. Neither tells us through `message`, so without this
  // the entry keeps claiming to be attached: the re-attach on the next
  // `dom-ready` early-returns, and every command rejects forever.
  const onDetach = (): void => {
    entry.attached = false
    entry.refs.clear()
    entry.generation++
  }
  entry.onDebuggerDetach = onDetach
  wc.debugger.on('detach', onDetach)

  const onCdpMessage = (_e: unknown, method: string, params: unknown): void => {
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
    } else if (method === 'Page.frameNavigated' || method === 'Page.navigatedWithinDocument') {
      // Subframes navigate on their own schedule — a refreshing ad iframe must
      // not spend the main document's refs and send the agent round a re-read
      // loop for a page that never changed.
      const frame = p.frame as { parentId?: string } | undefined
      if (method === 'Page.frameNavigated' && frame?.parentId) return
      // The document the refs pointed into is gone. Bumping here is what makes
      // a stale ref fail loudly instead of resolving against a fresh tree.
      // `navigatedWithinDocument` counts: an SPA route change tears down the
      // DOM just as thoroughly while firing no frameNavigated.
      // Deliberately not on `Page.loadEventFired`: that fires for the document
      // the refs were *just* read from, which would spend them on a page that
      // never changed and send the agent round the re-read loop for nothing.
      entry.generation++
      entry.refs.clear()
    }
  }
  entry.onCdpMessage = onCdpMessage
  wc.debugger.on('message', onCdpMessage)

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
    if (entry.onCdpMessage) wc.debugger.off('message', entry.onCdpMessage)
    if (entry.onDebuggerDetach) wc.debugger.off('detach', entry.onDebuggerDetach)
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
    // The generation is part of the name, not just a field alongside it.
    // Navigation clears the map and restarts numbering at 1, so a bare
    // `ref_7` held across a navigation would land on the *new* document's
    // seventh element and resolve cleanly — acting on the wrong thing while
    // looking like it worked. Carrying the generation makes that a miss.
    const ref = `g${entry.generation}_ref_${entry.refs.size + 1}`
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
  // Snapshot before the await: if a navigation lands while the tree is in
  // flight, these nodes describe a document that is already gone, and minting
  // refs for them would stamp document A's nodes with document B's generation
  // — stale in a way no later check could detect.
  const readAt = entry.generation
  const { nodes: ax } = await send<{ nodes: AXNode[] }>(wc, 'Accessibility.getFullAXTree')
  if (entry.generation !== readAt) {
    throw new Error('The page navigated while it was being read. Call read_page again.')
  }

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

/**
 * The matches for `needle` across a whole accessibility tree.
 *
 * Separate from `find` so it can be tested without a live guest, and pure so
 * the budget question stays visible: `limit` bounds the matches returned, not
 * the nodes searched.
 */
export function matchNodes(
  ax: AXNode[],
  entry: Entry,
  needle: string,
  limit: number
): BrowserNode[] {
  const q = needle.toLowerCase()
  const out: BrowserNode[] = []
  for (const raw of ax) {
    if (out.length >= limit) break
    // Matched before minting a ref, so a search does not hand out handles to
    // every interactive node on the page as a side effect of looking.
    if (!(raw.name?.value ?? '').toLowerCase().includes(q)) continue
    const node = toNode(raw, entry)
    if (node) out.push(node)
  }
  return out
}

/**
 * Search the whole page for matching text.
 *
 * Deliberately not built on `readPage`: that paginates to a node budget, so a
 * search through it would only ever see the top of the document and report
 * "not found" for text sitting plainly further down. Since long pages are the
 * exact case an agent reaches for `find` to handle, the budget has to apply to
 * what comes *back* — the matches — rather than to what gets searched.
 */
export async function find(params: {
  sessionId: string
  text: string
  limit?: number
}): Promise<BrowserNode[]> {
  const { wc, entry } = contentsFor(params.sessionId)
  const readAt = entry.generation
  const { nodes: ax } = await send<{ nodes: AXNode[] }>(wc, 'Accessibility.getFullAXTree')
  if (entry.generation !== readAt) {
    throw new Error('The page navigated while it was being searched. Call find again.')
  }
  return matchNodes(ax, entry, params.text, params.limit ?? 20)
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
  const normalized = normalizeUrl(params.url)
  if (!normalized) {
    throw new Error(`Refusing to navigate to "${params.url}" — not an allowed web address.`)
  }
  // No pane yet is not a failure — it is the first navigation. Opening it with
  // the url in hand also avoids a visible blank-page flash.
  if (!entries.get(params.sessionId)?.attached) {
    await openPane({ sessionId: params.sessionId, url: normalized })
    return { url: normalized }
  }
  const { wc } = contentsFor(params.sessionId)
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
    // A guest that dies or navigates while the picker is armed will never send
    // `inspectNodeRequested`. Without this the promise never settles: the
    // renderer's invoke hangs forever and the button stays lit with no error.
    const abandon = (why: string) => (): void => {
      wc.debugger.off('message', onMessage)
      wc.off('destroyed', onGone)
      wc.off('did-start-navigation', onGone)
      pickers.delete(params.sessionId)
      reject(new Error(why))
    }
    const onGone = abandon('The page changed before anything was picked.')
    wc.once('destroyed', onGone)
    wc.once('did-start-navigation', onGone)

    wc.debugger.on('message', onMessage)
    pickers.set(params.sessionId, abandon('Selection cancelled'))
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

/** The guest's current scroll offset, for turning viewport coords into page ones. */
async function pageScroll(wc: WebContents): Promise<{ x: number; y: number }> {
  const { result } = await send<{ result: { value: { x: number; y: number } } }>(
    wc,
    'Runtime.evaluate',
    { expression: '({x: scrollX, y: scrollY})', returnByValue: true }
  )
  return result.value
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
  const ref = `g${entry.generation}_ref_${entry.refs.size + 1}`
  entry.refs.set(ref, backendNodeId)
  ;(selection as BrowserSelection & { ref?: string }).ref = ref

  // A crop of just this element. Cheap — one node, not a page — and it settles
  // questions about rendering that markup alone cannot.
  try {
    // `rect` came from getBoundingClientRect — viewport coordinates — while
    // `clip` is page-relative. On any scrolled page the difference is a crop of
    // whatever sits scrollY pixels above the picked element: plausible, and
    // silently the wrong thing.
    const scroll = await pageScroll(wc)
    const { data } = await send<{ data: string }>(wc, 'Page.captureScreenshot', {
      format: 'png',
      captureBeyondViewport: true,
      clip: {
        x: selection.rect.x + scroll.x,
        y: selection.rect.y + scroll.y,
        width: Math.max(1, selection.rect.width),
        height: Math.max(1, selection.rect.height),
        scale: 1
      }
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
 * Strokes arrive in **viewport** coordinates — ink is drawn on a canvas laid
 * over the pane, so that is the only space the renderer knows. Main converts
 * to page coordinates once, here, by reading the scroll offset itself.
 *
 * The distinction is the whole trap: on a layout with an inner scrolling
 * container, resolving a stroke against the wrong space returns whatever
 * happens to be scrolled into that position at lookup time, and it fails
 * *silently* — plausible elements that are simply the wrong ones. Reading the
 * offset here rather than threading it through the renderer keeps it from
 * drifting out of date between the last stroke and the send.
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
  const sc = await pageScroll(wc)
  const points = viewport.map((p) => ({ x: p.x + sc.x, y: p.y + sc.y }))

  // Reduced rather than spread: a freehand stroke can carry thousands of
  // points, and `Math.min(...pts)` passes every one as an argument.
  let x0 = Infinity
  let y0 = Infinity
  let x1 = -Infinity
  let y1 = -Infinity
  for (const p of points) {
    if (p.x < x0) x0 = p.x
    if (p.x > x1) x1 = p.x
    if (p.y < y0) y0 = p.y
    if (p.y > y1) y1 = p.y
  }
  const bounds = { x: x0, y: y0, width: Math.max(1, x1 - x0), height: Math.max(1, y1 - y0) }

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
