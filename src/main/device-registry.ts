import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { nativeImage } from 'electron'
import type {
  DeviceInfo,
  DeviceElement,
  DeviceScreenRead,
  DevicePoint,
  DeviceTarget,
  DeviceSelection,
  DeviceAnnotation
} from '../shared/types'
import { IPC } from '../shared/types'
import {
  startCompanion,
  stopCompanion,
  call,
  callStreaming,
  type CompanionHandle
} from './device-companion'
import log from './logger'

const exec = promisify(execFile)

/**
 * The agent's handle on a session's simulator.
 *
 * The browser registry's counterpart, and the differences are the interesting
 * part. A `<webview>` guest belongs to the app; a simulator is a machine-wide
 * resource that other sessions — and the person — can also reach. So this file
 * carries an ownership model the browser one does not need, and a weaker notion
 * of an element handle:
 *
 * A CDP ref names a DOM node, and survives reflow. There is no such identity on
 * an accessibility tree read over idb: a ref here is *a point that was correct
 * when the screen was read*. Nothing detects that the screen moved underneath
 * it. That is why the generation is bumped after every successful input rather
 * than only on navigation — mobile has no navigation event, and a tap landing
 * on whatever animated into that frame is both silent and destructive.
 */

/** How many elements one `read_screen` returns before handing back a cursor. */
const SCREEN_BUDGET = 200

/** Ring buffer depth for captured device logs, per session. */
const LOG_BUFFER = 500

/**
 * How close to a bezel a stroke may start before iOS claims it.
 *
 * A swipe beginning inside this band is swallowed as a system gesture — back,
 * home, app switcher — and reported as a perfectly successful swipe. The agent
 * sees "the scroll did nothing" and retries forever.
 */
export const EDGE_BAND_POINTS = 20

export interface Entry {
  sessionId: string
  udid: string
  /** Whether Vorn booted this simulator, which decides whether closing the
   *  session shuts it down. Shutting down a simulator the person booted
   *  themselves is destructive and surprising. */
  bootedByVorn: boolean
  companion: CompanionHandle | null
  /** Bumped after EVERY successful input, not only on screen change. */
  generation: number
  /** ref name → the point to tap, in **points**. */
  refs: Map<string, { x: number; y: number; label: string }>
  screenPoints: { width: number; height: number } | null
  /** pixels ÷ points — 3 on every modern iPhone, but read, never assumed. */
  scale: number
  logs: string[]
}

const entries = new Map<string, Entry>()

/** A blank entry. Exported so tests can drive the pure helpers directly, since
 *  `src/main/**` is outside the coverage include. */
export function newEntry(sessionId = 's', udid = 'udid-0'): Entry {
  return {
    sessionId,
    udid,
    bootedByVorn: false,
    companion: null,
    generation: 1,
    refs: new Map(),
    screenPoints: null,
    scale: 3,
    logs: []
  }
}

// ---------------------------------------------------------------------------
// Ownership
// ---------------------------------------------------------------------------

/**
 * Decides whether `sessionId` may claim `udid`.
 *
 * Sharing is never silent. Two agents tapping one screen produce a sequence of
 * interleaved actions that looks exactly like flaky app behaviour, and the
 * person debugging it has no way to see the second driver. So a contested claim
 * fails loudly and names both the holder and the way out.
 */
export function claimFor(
  udid: string,
  sessionId: string,
  claims: ReadonlyMap<string, { udid: string; sessionId: string }>,
  free: readonly string[] = []
): { ok: true; alreadyMine: boolean } | { ok: false; error: string } {
  for (const held of claims.values()) {
    if (held.udid !== udid) continue
    if (held.sessionId === sessionId) return { ok: true, alreadyMine: true }
    const alternatives = free.length
      ? ` Free devices: ${free.join(', ')}.`
      : ' No other device is free — release one or create a new simulator.'
    return {
      ok: false,
      error: `Device ${udid} is already claimed by session ${held.sessionId}.${alternatives}`
    }
  }
  return { ok: true, alreadyMine: false }
}

/** Every device currently claimed, keyed by session. */
function claims(): ReadonlyMap<string, { udid: string; sessionId: string }> {
  const m = new Map<string, { udid: string; sessionId: string }>()
  for (const [sid, e] of entries) m.set(sid, { udid: e.udid, sessionId: sid })
  return m
}

// ---------------------------------------------------------------------------
// Geometry — the 3× trap
// ---------------------------------------------------------------------------

/**
 * Points → pixels, and back.
 *
 * The accessibility tree speaks points (402×874 on an iPhone 17 Pro); a
 * screenshot is pixels (1206×2622). Reading a coordinate off an image and
 * tapping it directly lands at one third of the intended position — near the
 * top-left of the screen, on whatever happens to be there. It is the single
 * most likely silent mis-tap in this whole surface, which is why the conversion
 * is a named, tested function rather than an inline `* 3`.
 */
export function pointsToPixels(p: DevicePoint, scale: number): DevicePoint {
  return { x: p.x * scale, y: p.y * scale }
}

export function pixelsToPoints(p: DevicePoint, scale: number): DevicePoint {
  return { x: p.x / scale, y: p.y / scale }
}

/** Whether a stroke starting here would be eaten by iOS as a system gesture. */
export function inEdgeBand(
  p: DevicePoint,
  screen: { width: number; height: number },
  band = EDGE_BAND_POINTS
): boolean {
  return p.x < band || p.y < band || p.x > screen.width - band || p.y > screen.height - band
}

// ---------------------------------------------------------------------------
// Accessibility tree
// ---------------------------------------------------------------------------

/** One node of idb's NESTED accessibility JSON. */
export interface AXElement {
  role?: string | null
  role_description?: string | null
  AXLabel?: string | null
  AXValue?: string | null
  AXUniqueId?: string | null
  title?: string | null
  enabled?: boolean
  frame?: { x: number; y: number; width: number; height: number }
  children?: AXElement[]
}

/** Roles that accept input. A ref for anything else spends the budget without
 *  giving the agent anything to do. */
const INTERACTIVE = new Set([
  'AXButton',
  'AXCell',
  'AXLink',
  'AXTextField',
  'AXSecureTextField',
  'AXTextArea',
  'AXSearchField',
  'AXSwitch',
  'AXSlider',
  'AXStepper',
  'AXSegmentedControl',
  'AXTabBar',
  'AXTab',
  'AXPopUpButton',
  'AXCheckBox',
  'AXRadioButton',
  'AXMenuItem',
  'AXPicker',
  'AXPickerWheel'
])

/** Depth-first flatten. idb returns a tree; every consumer here wants a list,
 *  and `find` in particular must see all of it (see `matchElements`). */
export function flattenTree(root: AXElement | AXElement[]): AXElement[] {
  const out: AXElement[] = []
  const walk = (n: AXElement): void => {
    out.push(n)
    for (const c of n.children ?? []) walk(c)
  }
  for (const n of Array.isArray(root) ? root : [root]) walk(n)
  return out
}

/** The label a person would recognise, from whichever field carries it. */
function labelOf(ax: AXElement): string | undefined {
  const label = (ax.AXLabel ?? ax.title ?? '').trim()
  return label.length ? label : undefined
}

/**
 * An accessibility node → an element the agent can act on, minting a ref when
 * there is something to act on.
 *
 * The ref carries its generation in the name, as in `browser-registry.toNode`,
 * and for the same reason: a bare `el_7` from an earlier screen would resolve
 * cleanly against this screen's seventh element and tap the wrong thing while
 * looking like it worked.
 */
export function toElement(ax: AXElement, entry: Entry): DeviceElement | null {
  const role = ax.role ?? undefined
  if (!role) return null
  const label = labelOf(ax)
  const interactive = INTERACTIVE.has(role)
  if (!interactive && !label) return null

  const el: DeviceElement = { role }
  if (label) el.label = label
  const value = ax.AXValue
  if (value !== null && value !== undefined && value !== '') el.value = String(value)
  // The greppable link back to app source — `accessibilityIdentifier` in the
  // code, and unlike a React debug id it survives a release build. A
  // convention rather than a guarantee, so it is surfaced, never required.
  if (ax.AXUniqueId) el.uniqueId = ax.AXUniqueId
  if (ax.enabled === false) el.disabled = true
  if (ax.frame) el.frame = ax.frame

  if (interactive && ax.frame && ax.enabled !== false) {
    const ref = `g${entry.generation}_el_${entry.refs.size + 1}`
    entry.refs.set(ref, {
      x: ax.frame.x + ax.frame.width / 2,
      y: ax.frame.y + ax.frame.height / 2,
      label: label ?? role
    })
    el.ref = ref
  }
  return el
}

/**
 * Resolves a ref to the point it named, or explains why it cannot.
 *
 * The generation mismatch is the whole reason this is a function. Refusing here
 * is the only thing standing between a stale handle and a tap at a coordinate
 * that now belongs to something else.
 */
export function tapPointFor(ref: string, entry: Entry): DevicePoint {
  const hit = entry.refs.get(ref)
  if (hit) return { x: hit.x, y: hit.y }
  const gen = /^g(\d+)_el_/.exec(ref)?.[1]
  if (gen && Number(gen) !== entry.generation) {
    throw new Error(
      `That element handle is from an earlier screen (g${gen}, now g${entry.generation}). Call read_screen again.`
    )
  }
  throw new Error(`Unknown element handle "${ref}". Call read_screen to get current handles.`)
}

export function parseCursor(cursor: string | undefined, generation: number): number {
  if (!cursor) return 0
  const [gen, idx] = cursor.split(':')
  if (Number(gen) !== generation) return 0
  const n = Number(idx)
  return Number.isFinite(n) && n >= 0 ? n : 0
}

/**
 * Searches the **whole** tree, bounding only what comes back.
 *
 * PR #435 shipped `find` built on the paginated read, so a match past the
 * 200-node budget came back "not found" — on exactly the long screens that are
 * the reason to search instead of read. The lesson transfers directly: bound
 * the results, never the search.
 */
export function matchElements(
  tree: AXElement | AXElement[],
  entry: Entry,
  needle: string,
  limit: number
): DeviceElement[] {
  const want = needle.toLowerCase()
  const out: DeviceElement[] = []
  for (const ax of flattenTree(tree)) {
    if (out.length >= limit) break
    const hay = `${labelOf(ax) ?? ''} ${ax.AXValue ?? ''} ${ax.AXUniqueId ?? ''}`.toLowerCase()
    if (!hay.includes(want)) continue
    // Minted only for matches: if looking handed out a handle for every
    // control on screen, ref numbering would race ahead of anything the agent
    // was told about, and the map would grow on every repeated search.
    const el = toElement(ax, entry)
    if (el) out.push(el)
  }
  return out
}

/** Area of a node's frame, or Infinity when it has none — so a frameless node
 *  never wins "smallest containing element". */
function frameArea(ax: AXElement): number {
  return ax.frame ? ax.frame.width * ax.frame.height : Number.POSITIVE_INFINITY
}

/** True when `p` (points) falls inside the node's frame. */
export function frameContains(ax: AXElement, p: DevicePoint): boolean {
  const f = ax.frame
  if (!f) return false
  return p.x >= f.x && p.y >= f.y && p.x <= f.x + f.width && p.y <= f.y + f.height
}

/**
 * The element a person meant by pointing at a point.
 *
 * Smallest containing frame wins, not first or deepest: an iOS tree nests a
 * button inside a cell inside a table inside a window, all of which contain the
 * point, and only the smallest is the thing that was pointed at. Depth alone
 * gets this wrong whenever a decorative leaf spans the row.
 */
export function elementAtPoint(
  tree: AXElement | AXElement[],
  entry: Entry,
  p: DevicePoint
): DeviceElement | null {
  let best: AXElement | null = null
  for (const ax of flattenTree(tree)) {
    if (!frameContains(ax, p)) continue
    if (!best || frameArea(ax) < frameArea(best)) best = ax
  }
  return best ? toElement(best, entry) : null
}

/** Axis-aligned box round every inked point, in points. */
export function inkBounds(
  strokes: Array<{ points: DevicePoint[] }>
): { x: number; y: number; width: number; height: number } | null {
  const pts = strokes.flatMap((s) => s.points)
  if (pts.length === 0) return null
  const xs = pts.map((q) => q.x)
  const ys = pts.map((q) => q.y)
  const x = Math.min(...xs)
  const y = Math.min(...ys)
  return { x, y, width: Math.max(...xs) - x, height: Math.max(...ys) - y }
}

/** True when a node's frame overlaps the ink's box at all. */
export function frameIntersects(
  ax: AXElement,
  b: { x: number; y: number; width: number; height: number }
): boolean {
  const f = ax.frame
  if (!f) return false
  return (
    f.x <= b.x + b.width && f.x + f.width >= b.x && f.y <= b.y + b.height && f.y + f.height >= b.y
  )
}

/**
 * Elements under the ink, smallest first.
 *
 * Anything overlapping the box counts, rather than only what is fully inside:
 * a circle drawn round a button usually clips it, and demanding containment
 * would return nothing for the most natural gesture there is.
 */
export function elementsUnderInk(
  tree: AXElement | AXElement[],
  entry: Entry,
  bounds: { x: number; y: number; width: number; height: number },
  limit: number
): DeviceElement[] {
  const hits = flattenTree(tree)
    .filter((ax) => frameIntersects(ax, bounds))
    .sort((a, b) => frameArea(a) - frameArea(b))
  const out: DeviceElement[] = []
  for (const ax of hits) {
    if (out.length >= limit) break
    const el = toElement(ax, entry)
    if (el) out.push(el)
  }
  return out
}

// ---------------------------------------------------------------------------
// simctl
// ---------------------------------------------------------------------------

interface SimctlDevice {
  udid: string
  name: string
  state: string
  isAvailable: boolean
}

/**
 * Every simulator on the machine, with who holds it.
 *
 * The one operation that works without a claim — it is how the agent and the
 * pane both discover what there is to claim in the first place, so the person
 * never has to leave Vorn to find a udid.
 */
export async function listDevices(): Promise<DeviceInfo[]> {
  const { stdout } = await exec('xcrun', ['simctl', 'list', 'devices', 'available', '-j'])
  const parsed = JSON.parse(stdout) as { devices: Record<string, SimctlDevice[]> }
  const held = new Map<string, string>()
  for (const [sid, e] of entries) held.set(e.udid, sid)

  const out: DeviceInfo[] = []
  for (const [runtime, devices] of Object.entries(parsed.devices)) {
    // watchOS and tvOS simulators exist on most machines and none of this
    // surface applies to them; listing them is noise the agent must filter.
    if (!runtime.includes('iOS')) continue
    for (const d of devices) {
      if (!d.isAvailable) continue
      out.push({
        udid: d.udid,
        name: d.name,
        runtime: runtime.replace('com.apple.CoreSimulator.SimRuntime.', '').replace(/-/g, ' '),
        booted: d.state === 'Booted',
        ...(held.has(d.udid) ? { claimedBy: held.get(d.udid) } : {})
      })
    }
  }
  return out
}

/** Turns simctl's failure modes into instructions rather than diagnostics. */
function explainSimctl(err: unknown): Error {
  const msg = err instanceof Error ? err.message : String(err)
  if (/xcode-select|Command Line Tools|DeveloperDirectory/i.test(msg)) {
    return new Error(
      'Xcode is not selected — the Command Line Tools are, and they carry no simulators.\n' +
        'This needs a password, so you have to run it yourself:\n' +
        '  sudo xcode-select -s /Applications/Xcode.app/Contents/Developer'
    )
  }
  if (/Unable to find a device|Invalid device/i.test(msg)) {
    return new Error(`${msg}\nCall device_list to see the devices that exist.`)
  }
  if (/no runtime|runtime profile/i.test(msg)) {
    return new Error(
      `${msg}\nInstall an iOS runtime: Xcode → Settings → Components → iOS Simulator.`
    )
  }
  return new Error(msg)
}

// ---------------------------------------------------------------------------
// Claim / release
// ---------------------------------------------------------------------------

export async function claim(params: {
  sessionId: string
  udid: string
}): Promise<{ udid: string; name: string; booted: boolean }> {
  const devices = await listDevices()
  const device = devices.find((d) => d.udid === params.udid)
  if (!device) {
    throw new Error(
      `No simulator with udid ${params.udid}. Call device_list to see what is available.`
    )
  }

  const free = devices.filter((d) => !d.claimedBy).map((d) => `${d.name} (${d.udid})`)
  const decision = claimFor(params.udid, params.sessionId, claims(), free)
  if (!decision.ok) throw new Error(decision.error)

  // Releasing first keeps a session from silently holding two devices, which
  // would leak the old claim for as long as the session lived.
  const previous = entries.get(params.sessionId)
  if (previous && previous.udid !== params.udid) await release({ sessionId: params.sessionId })

  const wasBooted = device.booted
  if (!wasBooted) {
    try {
      await exec('xcrun', ['simctl', 'boot', params.udid])
      await exec('xcrun', ['simctl', 'bootstatus', params.udid, '-b'])
    } catch (err) {
      throw explainSimctl(err)
    }
  }

  const entry = newEntry(params.sessionId, params.udid)
  entry.bootedByVorn = !wasBooted
  entries.set(params.sessionId, entry)

  try {
    entry.companion = await startCompanion(params.udid, onCompanionExit)
  } catch (err) {
    entries.delete(params.sessionId)
    if (!wasBooted) await exec('xcrun', ['simctl', 'shutdown', params.udid]).catch(() => {})
    throw err
  }

  return { udid: params.udid, name: device.name, booted: true }
}

/**
 * The companion died on its own — the simulator was shut down from
 * Simulator.app, the process was killed, the machine slept.
 *
 * The browser registry's debugger-detach analogue: clear the client, forget
 * every ref and bump the generation, so the next call says the connection
 * dropped instead of hanging on a dead socket or tapping a remembered point.
 */
function onCompanionExit(udid: string): void {
  for (const entry of entries.values()) {
    if (entry.udid !== udid) continue
    entry.companion = null
    entry.refs.clear()
    entry.generation++
    log.warn(`[device] companion for ${udid.slice(0, 8)} exited; session ${entry.sessionId} idle`)
  }
}

export async function release(params: { sessionId: string }): Promise<{ released: boolean }> {
  const entry = entries.get(params.sessionId)
  if (!entry) return { released: false }
  entries.delete(params.sessionId)
  stopCompanion(entry.udid)
  // Only ours to shut down. A simulator the person booted stays up.
  if (entry.bootedByVorn) {
    await exec('xcrun', ['simctl', 'shutdown', entry.udid]).catch(() => {})
  }
  return { released: true }
}

/** Called when a session closes, so a claim cannot outlive its owner. */
export function releaseForSession(sessionId: string): void {
  void release({ sessionId })
}

// ---------------------------------------------------------------------------
// Driving
// ---------------------------------------------------------------------------

/** The entry for a session, or the reason there isn't one. */
export function deviceFor(sessionId: string): Entry & { companion: CompanionHandle } {
  const entry = entries.get(sessionId)
  if (!entry) {
    throw new Error(
      'No device is claimed for this session. Call device_list to see what is available, then device_claim.'
    )
  }
  if (!entry.companion) {
    throw new Error(
      `The connection to device ${entry.udid} dropped (the simulator or its companion exited). Call device_claim again.`
    )
  }
  return entry as Entry & { companion: CompanionHandle }
}

/** Reads the tree, refreshing the screen size and scale we key geometry off. */
async function fetchTree(entry: Entry & { companion: CompanionHandle }): Promise<AXElement[]> {
  const res = await call<{ json: string }>(entry.companion.client, 'accessibility_info', {
    format: 'NESTED'
  })
  // Verified against companion 1.1.8: this is an array whose first entry is the
  // application root, not a bare object.
  const parsed = JSON.parse(res.json) as AXElement | AXElement[]
  const list = Array.isArray(parsed) ? parsed : [parsed]
  const rootFrame = list[0]?.frame
  if (rootFrame) entry.screenPoints = { width: rootFrame.width, height: rootFrame.height }
  return list
}

export async function readScreen(params: {
  sessionId: string
  filter?: 'interactive' | 'all'
  cursor?: string
  limit?: number
}): Promise<DeviceScreenRead> {
  const entry = deviceFor(params.sessionId)
  const readAt = entry.generation
  const tree = await fetchTree(entry)
  if (entry.generation !== readAt) {
    throw new Error('The screen changed while it was being read. Call read_screen again.')
  }

  const flat = flattenTree(tree)
  const start = parseCursor(params.cursor, entry.generation)
  const budget = Math.min(params.limit ?? SCREEN_BUDGET, SCREEN_BUDGET)
  const wantAll = params.filter === 'all'

  const out: DeviceElement[] = []
  let i = start
  for (; i < flat.length && out.length < budget; i++) {
    const el = toElement(flat[i], entry)
    if (!el) continue
    if (!wantAll && !el.ref) continue
    out.push(el)
  }

  return {
    udid: entry.udid,
    elements: out,
    generation: entry.generation,
    screen: entry.screenPoints ?? { width: 0, height: 0 },
    scale: entry.scale,
    ...(i < flat.length ? { nextCursor: `${entry.generation}:${i}` } : {})
  }
}

export async function findElements(params: {
  sessionId: string
  query: string
  limit?: number
}): Promise<{ elements: DeviceElement[]; generation: number }> {
  const entry = deviceFor(params.sessionId)
  const tree = await fetchTree(entry)
  return {
    elements: matchElements(tree, entry, params.query, params.limit ?? 20),
    generation: entry.generation
  }
}

export async function logsFor(params: {
  sessionId: string
  limit?: number
}): Promise<{ lines: string[] }> {
  const entry = deviceFor(params.sessionId)
  return { lines: entry.logs.slice(-(params.limit ?? 100)) }
}

/** Appends to the bounded log buffer. */
export function pushLog(sessionId: string, line: string): void {
  const entry = entries.get(sessionId)
  if (!entry) return
  entry.logs.push(line)
  if (entry.logs.length > LOG_BUFFER) entry.logs.splice(0, entry.logs.length - LOG_BUFFER)
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/** Resolves a target to a point, refusing a stale ref rather than tapping. */
function resolveTarget(target: DeviceTarget | undefined, entry: Entry): DevicePoint {
  if (!target) throw new Error('An interaction needs a target: either a ref or x/y (in points).')
  if ('ref' in target) return tapPointFor(target.ref, entry)
  return { x: target.x, y: target.y }
}

/** Keycodes for `type`, ASCII → HID usage. Enough for printable text. */
function keycodesFor(text: string): number[] {
  const codes: number[] = []
  for (const ch of text) {
    const lower = ch.toLowerCase()
    if (lower >= 'a' && lower <= 'z') codes.push(4 + (lower.charCodeAt(0) - 97))
    else if (ch >= '1' && ch <= '9') codes.push(30 + (ch.charCodeAt(0) - 49))
    else if (ch === '0') codes.push(39)
    else if (ch === ' ') codes.push(44)
    else if (ch === '\n') codes.push(40)
    else if (ch === '.') codes.push(55)
    else if (ch === '-') codes.push(45)
    else if (ch === '@') codes.push(31)
    else codes.push(44)
  }
  return codes
}

const BUTTONS = new Set(['APPLE_PAY', 'HOME', 'LOCK', 'SIDE_BUTTON', 'SIRI'])

/**
 * Tap, swipe, type, or press a hardware button.
 *
 * Every path here ends by bumping the generation. On the web a ref is refused
 * because the *document* changed; here it must be refused because anything at
 * all happened — a sheet animating in moves every frame on screen while the
 * tree we last read still describes where things used to be, and a tap at that
 * remembered point is both wrong and completely silent.
 */
export async function interact(params: {
  sessionId: string
  action: 'tap' | 'swipe' | 'type' | 'button' | 'press'
  target?: DeviceTarget
  to?: DevicePoint
  text?: string
  duration?: number
  systemGesture?: boolean
}): Promise<{ ok: true; generation: number }> {
  const entry = deviceFor(params.sessionId)
  // The tree also refreshes screen size, which the edge-band check needs.
  if (!entry.screenPoints) await fetchTree(entry)
  const screen = entry.screenPoints ?? { width: 0, height: 0 }

  const events: unknown[] = []
  switch (params.action) {
    case 'tap':
    case 'press': {
      const point = resolveTarget(params.target, entry)
      const touch = { action: { touch: { point } } }
      events.push({ press: { action: touch.action, direction: 'DOWN' } })
      if (params.action === 'press') {
        events.push({ delay: { duration: params.duration ?? 1 } })
      }
      events.push({ press: { action: touch.action, direction: 'UP' } })
      break
    }
    case 'swipe': {
      const start = resolveTarget(params.target, entry)
      if (!params.to) throw new Error('A swipe needs `to` — the point to swipe towards.')
      if (!params.systemGesture) {
        // Fail closed. A read taken while the screen is mid-transition comes
        // back with no root frame, which leaves the screen size unknown — and
        // an edge-band check that opts out when it cannot measure is exactly
        // the silent pass this guard exists to prevent. Verified live: the
        // first swipe after a navigating tap slipped through this way.
        if (!(screen.width > 0 && screen.height > 0)) {
          throw new Error(
            'The screen size could not be read (the device was probably mid-transition), so ' +
              'whether this swipe starts inside the bezel band cannot be checked — and a swipe ' +
              'that iOS claims as a system gesture is reported as a perfectly successful swipe. ' +
              'Call read_screen and try again, or pass systemGesture: true if that is what you meant.'
          )
        }
        if (inEdgeBand(start, screen)) {
          throw new Error(
            `A swipe starting within ${EDGE_BAND_POINTS}pt of the bezel is taken by iOS as a system ` +
              `gesture (back, home, app switcher) and never reaches the app — it looks like a swipe ` +
              `that did nothing. Start at least ${EDGE_BAND_POINTS}pt inside the screen ` +
              `(${screen.width}×${screen.height} points), or pass systemGesture: true if that is what you meant.`
          )
        }
      }
      events.push({ swipe: { start, end: params.to, duration: params.duration ?? 0.3 } })
      break
    }
    case 'type': {
      if (!params.text) throw new Error('`type` needs `text`.')
      for (const code of keycodesFor(params.text)) {
        events.push({ press: { action: { key: { keycode: code } }, direction: 'DOWN' } })
        events.push({ press: { action: { key: { keycode: code } }, direction: 'UP' } })
      }
      break
    }
    case 'button': {
      const name = (params.text ?? '').toUpperCase()
      if (!BUTTONS.has(name)) {
        throw new Error(`Unknown button "${params.text}". One of: ${[...BUTTONS].join(', ')}.`)
      }
      events.push({ press: { action: { button: { button: name } }, direction: 'DOWN' } })
      events.push({ press: { action: { button: { button: name } }, direction: 'UP' } })
      break
    }
  }

  await callStreaming(entry.companion.client, 'hid', events)
  // Refs describe a screen that this input may have just replaced.
  entry.generation++
  entry.refs.clear()
  return { ok: true, generation: entry.generation }
}

// ---------------------------------------------------------------------------
// Screenshot
// ---------------------------------------------------------------------------

/** Long edge, in pixels, of a screenshot handed to an agent. */
const AGENT_MAX_EDGE = 1000

/**
 * A downscaled still.
 *
 * `ScreenshotRequest` is an empty message — the companion offers no scale,
 * quality, or format knob, so every capture is a full ~2.9 MB PNG and the
 * downscaling is unavoidably ours. Sending that raw at even a slow poll rate
 * would dominate everything else this pane does.
 *
 * `scale` ships alongside because a coordinate read off this image is in image
 * pixels, and every tap is in points. Returning one without the other is what
 * makes a mis-tap at 3× look like a targeting mistake instead of a unit error.
 */
export async function screenshot(params: {
  sessionId: string
  maxEdge?: number
}): Promise<{ data: string; scale: number; screen: { width: number; height: number } }> {
  const entry = deviceFor(params.sessionId)
  if (!entry.screenPoints) await fetchTree(entry)
  const screen = entry.screenPoints ?? { width: 0, height: 0 }

  const res = await call<{ image_data: Buffer }>(entry.companion.client, 'screenshot', {})
  const image = nativeImage.createFromBuffer(Buffer.from(res.image_data))
  const size = image.getSize()
  // Read, never assumed: the tree's points against the image's pixels is the
  // only honest source for this ratio.
  if (screen.width > 0) entry.scale = size.width / screen.width

  const maxEdge = params.maxEdge ?? AGENT_MAX_EDGE
  const longest = Math.max(size.width, size.height)
  const out =
    longest > maxEdge
      ? image.resize({
          width: Math.round(size.width * (maxEdge / longest)),
          quality: 'good'
        })
      : image

  const shown = out.getSize()
  return {
    data: out.toPNG().toString('base64'),
    // The scale of the image *as returned*, not of the raw capture — this is
    // the number that converts a coordinate on the delivered image to points.
    scale: screen.width > 0 ? shown.width / screen.width : entry.scale,
    screen
  }
}

// ---------------------------------------------------------------------------
// Apps
// ---------------------------------------------------------------------------

export async function launch(params: {
  sessionId: string
  bundleId: string
}): Promise<{ ok: true }> {
  const entry = deviceFor(params.sessionId)
  try {
    await callStreaming(entry.companion.client, 'launch', [
      { start: { bundle_id: params.bundleId, foreground_if_running: true } }
    ])
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err)
    if (/not installed|no such|cannot find/i.test(msg)) {
      // Naming the bundle id is the actionable half; guessing a build command
      // for an unknown project would be worse than saying nothing.
      throw new Error(`${params.bundleId} is not installed on this device. Install it first.`, {
        cause: err
      })
    }
    throw err
  }
  return { ok: true }
}

export async function terminate(params: {
  sessionId: string
  bundleId: string
}): Promise<{ ok: true }> {
  const entry = deviceFor(params.sessionId)
  await call(entry.companion.client, 'terminate', { bundle_id: params.bundleId })
  return { ok: true }
}

export async function openUrl(params: { sessionId: string; url: string }): Promise<{ ok: true }> {
  const entry = deviceFor(params.sessionId)
  await call(entry.companion.client, 'open_url', { url: params.url })
  entry.generation++
  entry.refs.clear()
  return { ok: true }
}

export async function install(params: { sessionId: string; path: string }): Promise<{ ok: true }> {
  const entry = deviceFor(params.sessionId)
  // simctl rather than the companion's streaming install: it takes a path
  // directly, where the gRPC call wants the payload chunked over the wire.
  try {
    await exec('xcrun', ['simctl', 'install', entry.udid, params.path])
  } catch (err) {
    throw explainSimctl(err)
  }
  return { ok: true }
}

// ---------------------------------------------------------------------------
// Pane
// ---------------------------------------------------------------------------

/**
 * How main asks the renderer to show a pane, injected rather than imported so
 * the dependency keeps pointing one way. Same seam as the browser registry.
 */
type RendererSend = (channel: string, params: unknown) => void
/**
 * Fails loudly rather than dropping the send.
 *
 * A no-op default is how `device:openPane` came to return `{ udid }` while the
 * renderer was never told: main had wired the browser registry and not this
 * one, so every pane request succeeded and no pane ever appeared. Nothing in
 * the reply could have shown that. Throwing makes the missing wiring a startup
 * bug someone can read, not a pane that silently never opens.
 */
const unwiredSend: RendererSend = () => {
  throw new Error(
    'The device registry was never wired to the renderer, so no pane can be shown. ' +
      'This is a Vorn startup bug (setRendererSend was not called), not something the ' +
      'agent or the device did.'
  )
}
let sendToRenderer: RendererSend = unwiredSend
export function setRendererSend(fn: RendererSend): void {
  sendToRenderer = fn
}

/**
 * Show the session's device pane.
 *
 * Unlike the browser, the pane is not required for anything: the claim and the
 * companion are what the tools run on, and both work headless. This exists so
 * the agent can *show its work* to the person — and so it claims first, then
 * asks for the pane, never the reverse.
 */
export async function openPane(params: {
  sessionId: string
  udid?: string
}): Promise<{ udid: string }> {
  if (params.udid) await claim({ sessionId: params.sessionId, udid: params.udid })
  const entry = entries.get(params.sessionId)
  if (!entry) {
    throw new Error(
      'No device is claimed for this session. Call device_claim first, or pass a udid.'
    )
  }
  // The pane titles itself with the display name, so it ships with the event.
  // Falling back to the udid keeps the pane openable when the listing fails —
  // an ugly title beats no pane at all.
  const name =
    (await listDevices().catch(() => [])).find((d) => d.udid === entry.udid)?.name ?? entry.udid
  sendToRenderer(IPC.DEVICE_OPEN_PANE, { sessionId: params.sessionId, udid: entry.udid, name })
  return { udid: entry.udid }
}

/**
 * Resolve a point the person pointed at to the element there.
 *
 * Deliberately read-only: pointing must never move the screen, or the person
 * would be describing an element the agent then finds gone. The generation is
 * reported so the caller can say which screen this described.
 */
export async function pickAt(params: {
  sessionId: string
  point: DevicePoint
}): Promise<DeviceSelection> {
  const entry = deviceFor(params.sessionId)
  const tree = await fetchTree(entry)
  const element = elementAtPoint(tree, entry, params.point)
  return {
    udid: entry.udid,
    point: params.point,
    generation: entry.generation,
    ...(element ? { element } : {})
  }
}

/**
 * Resolve freehand ink to the elements it covers.
 *
 * Also read-only, and for the same reason as `pickAt`.
 */
export async function annotate(params: {
  sessionId: string
  strokes: Array<{ points: DevicePoint[] }>
  limit?: number
}): Promise<DeviceAnnotation> {
  const entry = deviceFor(params.sessionId)
  const bounds = inkBounds(params.strokes)
  if (!bounds) throw new Error('The annotation had no strokes.')
  const tree = await fetchTree(entry)
  return {
    udid: entry.udid,
    bounds,
    elements: elementsUnderInk(tree, entry, bounds, params.limit ?? 10),
    generation: entry.generation
  }
}

/** Test seam: the registry is module state, so tests need a way back to zero. */
export function resetForTests(): void {
  entries.clear()
  // Also the renderer wiring, or a test that installs a send leaks it into
  // every later test — including the one asserting an unwired registry
  // refuses, which would then pass or fail on declaration order alone.
  sendToRenderer = unwiredSend
}

export function entryForTests(sessionId: string): Entry | undefined {
  return entries.get(sessionId)
}

export function setEntryForTests(entry: Entry): void {
  entries.set(entry.sessionId, entry)
}
