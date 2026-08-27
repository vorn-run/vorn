import * as headless from '@xterm/headless'
import * as serializeAddon from '@xterm/addon-serialize'
import log from './logger'

/**
 * Reached through an interop dance rather than by name, and it earns its keep.
 *
 * Both packages are plain CommonJS -- no `exports` map, no `module` field -- and
 * build their exports in a way Node's named-export detection cannot see. That
 * makes `import { Terminal } from '@xterm/headless'` fail at import time under
 * `tsx`, which is how the server runs in development: a server that will not
 * start, caught by `server-port-stability` spawning the real thing.
 *
 * Nor is a plain default import enough. Compiled to CommonJS the namespace *is*
 * the exports object and there is no `default` at all; loaded as ESM the exports
 * arrive under `default`, because the same detection failure that breaks the
 * named form also leaves the namespace with nothing else on it. This file is
 * loaded both ways -- the bundle, `tsx` and vitest do not agree -- so it takes
 * whichever of the two actually holds the classes.
 */
function interop<T>(mod: unknown): T {
  const ns = mod as { default?: T }
  return ns?.default ?? (mod as T)
}

const { Terminal } = interop<typeof import('@xterm/headless')>(headless)
const { SerializeAddon } = interop<typeof import('@xterm/addon-serialize')>(serializeAddon)
type Terminal = InstanceType<typeof Terminal>
type SerializeAddon = InstanceType<typeof SerializeAddon>

/**
 * What the terminal currently looks like, as a screen rather than as bytes.
 *
 * `terminal-scrollback` keeps the bytes a terminal emitted, which is what a
 * client needs in order to draw. This keeps what those bytes *mean*: where the
 * cursor is, which modes are set, whether the alternate screen is active, what
 * colour each cell is. Neither can be derived from the other, so both are kept —
 * a checkpoint of a byte buffer is just a smaller byte buffer, while a
 * checkpoint of a screen is a state something can be restored to.
 *
 * Nothing consumes this yet. `terminal:readScrollback` has no caller outside the
 * server either; both are groundwork for attaching to a live session rather than
 * relaunching it, and for sending each client what changed on screen instead of
 * every byte.
 *
 * ## This emulator must never answer a query
 *
 * A terminal that is asked who it is answers. Feed one `\x1b[c` and it produces
 * `\x1b[?62;...c`; the same is true of cursor-position reports, mode queries and
 * half a dozen others. That is correct behaviour for the terminal a person is
 * looking at, and this is not that terminal.
 *
 * The client's xterm is already the thing that answers — its `onData` is wired
 * to `terminal:write`, which writes to the PTY. A second emulator answering the
 * same query would interleave a second reply into the PTY's *input* stream, and
 * the shell would take it as literal keystrokes: a stray `[?62;1;2c` typed at
 * the prompt that nobody typed.
 *
 * Every reply xterm produces goes through one place — `CoreService.triggerDataEvent`,
 * whose whole tail is `this._onData.fire(data)`. With no subscriber that is a
 * no-op. So the protection here is an absence rather than a flag: **nothing in
 * this file subscribes to `onData` or `onBinary`, and nothing may.** A flag
 * defaulting to off would be worse, because it would look like a thing that
 * could be turned on.
 *
 * Verified rather than assumed: feeding `\x1b[c` to a 5.5.0 headless terminal
 * with a listener attached does fire it. The hazard is real; the absence is what
 * closes it.
 */

/**
 * No scrollback at all.
 *
 * The client runs 2000 lines, and matching that would cost roughly five megabytes
 * per session at a realistic 200x50 — a quarter of a gigabyte across fifty of
 * them, to duplicate history that is already held twice: once as bytes in
 * `terminal-scrollback`, once as stripped lines in `pty-manager`. This models
 * the *screen*, which is the thing neither of those can answer for.
 *
 * It also makes a resize cheap. Shrinking reflows and discards what falls off
 * the top, so a phone briefly fitting a session to sixty columns would otherwise
 * permanently truncate what the desktop had.
 */
const SCROLLBACK = 0

/**
 * Mirrors the client's terminal, and the mirroring is the point.
 *
 * `allowProposedApi` matches `terminal-registry.ts` because the serialize addon
 * needs it. Nothing about colours or fonts is copied: a headless terminal has no
 * renderer, and a theme would only be a second copy of a value to drift.
 *
 * The one difference is scrollback, above, and it is deliberate. Any *other*
 * divergence — a different core version, a width-table addon the client does not
 * load — moves where a line wraps, and a screen that wraps differently is not
 * the screen the user is looking at. That is why the version here is pinned to
 * the client's 5.5, and why `@xterm/addon-unicode11` is deliberately absent: the
 * client uses xterm's built-in v6 width tables, and a server on v11 would wrap
 * an emoji at a different column.
 */
function create(cols: number, rows: number): Held {
  const term = new Terminal({ cols, rows, scrollback: SCROLLBACK, allowProposedApi: true })
  const serializer = new SerializeAddon()
  term.loadAddon(serializer)
  return { term, serializer, pending: Promise.resolve() }
}

interface Held {
  term: Terminal
  serializer: SerializeAddon
  /** The last write, so a read can wait for it. See `serializeScreen`. */
  pending: Promise<void>
}

const screens = new Map<string, Held>()

/** What a PTY starts at, when a session has not said otherwise. */
const FALLBACK_COLS = 80
const FALLBACK_ROWS = 24

/**
 * Feed output to the screen model.
 *
 * Never throws. This sits on the path that broadcasts live output to every
 * attached client, and a screen model is worth nothing beside a terminal that
 * stopped updating — so a terminal that faults is dropped and the session
 * carries on without one.
 */
export function feedScreen(id: string, data: string, cols?: number, rows?: number): void {
  try {
    let held = screens.get(id)
    if (!held) {
      held = create(cols ?? FALLBACK_COLS, rows ?? FALLBACK_ROWS)
      screens.set(id, held)
    }
    const term = held.term
    held.pending = new Promise<void>((resolve) => term.write(data, resolve))
  } catch (err) {
    log.warn({ err, id }, '[screen] dropping the screen model for this session')
    clearScreen(id)
  }
}

/**
 * Follow a resize.
 *
 * The values must be the ones node-pty was given, because the program is
 * rendering against those: a model one column wider wraps in a different place,
 * and every line after the first divergence is wrong.
 */
export function resizeScreen(id: string, cols: number, rows: number): void {
  const held = screens.get(id)
  if (!held) return
  try {
    held.term.resize(cols, rows)
  } catch (err) {
    log.warn({ err, id }, '[screen] dropping the screen model for this session')
    clearScreen(id)
  }
}

/**
 * The screen as escape sequences that reproduce it.
 *
 * Async, and that is not a convenience. `term.write` is queued, not applied —
 * xterm defers the parse to a macrotask — so serializing straight after a write
 * returns whatever had been parsed by then, which is usually nothing and
 * occasionally most of it. That failure is invisible in a test that happens to
 * yield and reappears under load.
 */
export async function serializeScreen(id: string): Promise<string> {
  const held = screens.get(id)
  if (!held) return ''
  await held.pending
  try {
    return held.serializer.serialize()
  } catch (err) {
    log.warn({ err, id }, '[screen] could not serialize')
    return ''
  }
}

/**
 * Forget a terminal that has gone.
 *
 * `dispose()` rather than a map delete: a `Terminal` holds buffers and internal
 * services, and dropping the reference alone leaves every session ever closed
 * resident for the life of the server.
 */
export function clearScreen(id: string): void {
  const held = screens.get(id)
  if (!held) return
  screens.delete(id)
  try {
    held.term.dispose()
  } catch (err) {
    log.warn({ err, id }, '[screen] could not dispose a terminal')
  }
}

/** How many models are held. For the measurement that bounds this. */
export function screenCount(): number {
  return screens.size
}

/** Test-only, mirroring `resetScrollback`. */
export function resetScreens(): void {
  for (const id of [...screens.keys()]) clearScreen(id)
}
