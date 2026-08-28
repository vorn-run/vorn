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
 * How far the parser may fall behind before chunks are skipped.
 *
 * A screenful at a generous size is a few tens of kilobytes, so this is room for
 * many of them -- enough that nothing normal is ever dropped -- while being far
 * below the fifty megabytes xterm would otherwise hold before refusing.
 */
const MAX_QUEUED_UNITS = 4 * 1024 * 1024

/**
 * How much of a title or a cwd is kept.
 *
 * Both are stored for the life of the terminal and both travel in the
 * checkpoint, and xterm will hand over an OSC payload of up to ten million
 * characters. One `\x1b]0;` followed by five megabytes -- which an agent
 * printing the contents of a file it was asked to read can produce without
 * anybody intending it -- would put every checkpoint for that session over its
 * size cap, permanently, for as long as the terminal lives. Nothing that is
 * genuinely a title or a path comes near this.
 */
const MAX_LABEL_UNITS = 512

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
  const held: Held = { term, serializer, cols, rows, queued: 0, behind: false, title: '', cwd: '' }

  // Two things the program says about itself that the serialized screen does
  // not carry, so they are caught as they go past. Neither is a reply -- these
  // are notifications from the program, and observing one sends nothing back.
  //
  // The title is stored and nothing more. A property write cannot throw, and an
  // earlier version wrapped it anyway, which made a no-op look load-bearing.
  term.onTitleChange((title) => {
    held.title = title.slice(0, MAX_LABEL_UNITS)
  })
  // The cwd handler is the one that can. It runs from xterm's own timer rather
  // than from `feedScreen`, so a throw here reaches the top of the process past
  // every `try` in this file -- and the server installs no `uncaughtException`
  // handler. `decodeURIComponent` throws on a lone percent, which is a path a
  // shell can genuinely be sitting in, so that would be the whole server and
  // every session killed by a directory name.
  term.parser.registerOscHandler(7, (payload) => {
    try {
      // `file://host/path`, per the convention every shell integration uses.
      const raw = payload.replace(/^file:\/\/[^/]*/, '')
      // `decodeURIComponent` throws on a stray `%` -- and a directory named
      // `100%` is a directory somebody has. Left encoded rather than lost.
      if (raw) held.cwd = tryDecode(raw).slice(0, MAX_LABEL_UNITS)
    } catch {
      /* an unreadable cwd is not worth anything at all */
    }
    // False: this is an observation, not a takeover. xterm goes on handling it.
    return false
  })
  // And the one Vorn's own shells actually emit.
  //
  // The handler above listens on OSC 7, which is the convention other terminals
  // use -- and Vorn's shell integration has always emitted `5522;cwd;<path>`
  // instead, namespaced so it cannot collide with a standard sequence. So the
  // cwd this model captured was empty for every Vorn shell, and the checkpoint
  // has been carrying an empty field since it was written. Both are registered:
  // OSC 7 for a shell configured by hand or by another tool, 5522 for ours.
  //
  // Same discipline as above, and for the same reason -- this runs from xterm's
  // timer, where a throw reaches the top of a process with no handler behind it.
  term.parser.registerOscHandler(5522, (payload) => {
    try {
      const [kind, ...rest] = payload.split(';')
      if (kind === 'cwd' && rest.length) {
        const path = rest.join(';')
        if (path) held.cwd = path.slice(0, MAX_LABEL_UNITS)
      }
    } catch {
      /* an unreadable cwd is not worth anything at all */
    }
    return false
  })

  return held
}

/** Percent-decoded where that is possible, left alone where it is not. */
function tryDecode(value: string): string {
  try {
    return decodeURIComponent(value)
  } catch {
    return value
  }
}

interface Held {
  term: Terminal
  serializer: SerializeAddon
  cols: number
  rows: number
  /** Written but not yet parsed, in UTF-16 units. */
  queued: number
  /** Whether the last chunk was skipped, so the log says it once. */
  behind: boolean
  /** Last OSC 0/2. The addon does not carry it. */
  title: string
  /** Last OSC 7, as a path. The addon does not carry this either. */
  cwd: string
}

/**
 * A screen, and the things the serialized string does not say.
 *
 * `serialize()` emits cells, SGR, the final cursor position and a set of modes.
 * Everything else a restorer needs has to be gathered beside it -- which is why
 * this is a record rather than a string.
 */
export interface ScreenSnapshot {
  /** Escape sequences that reproduce the screen. */
  screen: string
  /** The geometry it must be replayed at, or the wrap points do not match. */
  cols: number
  rows: number
  /** Last window title the program set, if it set one. */
  title: string
  /** Working directory the shell last reported, if it reports one. */
  cwd: string
}

const screens = new Map<string, Held>()

/**
 * Feed output to the screen model.
 *
 * Never throws. This sits on the path that broadcasts live output to every
 * attached client, and a screen model is worth nothing beside a terminal that
 * stopped updating — so a terminal that faults is dropped and the session
 * carries on without one.
 */
export function feedScreen(id: string, data: string): void {
  const held = screens.get(id)
  if (!held) return

  // Dropped rather than queued when the model is behind.
  //
  // xterm parses on a timer and holds everything not yet parsed. A PTY can
  // outrun that -- `cat` of a large file arrives far faster than a VT parse --
  // and xterm's own answer at fifty megabytes of backlog is to throw. That would
  // cost the session its model permanently, having first held fifty megabytes to
  // get there.
  //
  // So the backlog is bounded here instead, and the response to a full one is to
  // skip the chunk. A model missing a screenful of a flood is repaired by the
  // next repaint; a model that no longer exists is not repaired at all. The
  // client is unaffected either way -- it got these bytes before this line ran.
  if (held.queued > MAX_QUEUED_UNITS) {
    if (!held.behind) {
      held.behind = true
      log.warn({ id }, '[screen] output is outrunning the screen model; skipping ahead')
    }
    return
  }

  try {
    held.queued += data.length
    held.behind = false
    held.term.write(data, () => {
      held.queued = Math.max(0, held.queued - data.length)
    })
  } catch (err) {
    drop(id, err)
  }
}

/**
 * Start modelling a terminal, at the geometry it was spawned with.
 *
 * Explicit rather than created on the first byte, so the geometry comes from the
 * one place that knows it instead of being threaded through every flush. It also
 * means `feedScreen` is a map lookup and a write, on a path that runs for every
 * chunk of every session.
 */
export function createScreen(
  id: string,
  cols: number,
  rows: number,
  labels?: { title?: string; cwd?: string }
): void {
  clearScreen(id)
  try {
    const held = create(cols, rows)
    // A restored screen is rebuilt from escape sequences, and neither of these
    // is one -- they arrive as notifications the serializer does not round-trip.
    // The checkpoint has been storing both since it was written and recovery
    // never put them back, so a session came back with no title and no
    // directory however carefully the rest of it was replayed.
    if (labels?.title) held.title = labels.title.slice(0, MAX_LABEL_UNITS)
    if (labels?.cwd) held.cwd = labels.cwd.slice(0, MAX_LABEL_UNITS)
    screens.set(id, held)
  } catch (err) {
    drop(id, err)
  }
}

/**
 * Follow a resize.
 *
 * The values must be the ones node-pty was given, because the program is
 * rendering against those: a model one column wider wraps in a different place,
 * and every line after the first divergence is wrong.
 */
export async function resizeScreen(id: string, cols: number, rows: number): Promise<void> {
  const held = screens.get(id)
  if (!held) return
  try {
    // At the drain, and that ordering is the whole point. Writes are queued, so
    // resizing straight away reflows bytes that arrived *before* the resize at
    // the size that came *after* it -- and the client, which parsed those bytes
    // before it refitted, wraps them somewhere else. A model that wraps
    // differently from the terminal it models is the failure this exists to
    // avoid, and it would show up as a screen that is subtly wrong rather than
    // one that is obviously broken.
    //
    // At the marker rather than after it, for the reason `atDrain` gives: bytes
    // parsed while the loop unwinds would be laid out at the old width by a
    // program that had already been told the new one.
    await atDrain(held, () => {
      held.term.resize(cols, rows)
      held.cols = cols
      held.rows = rows
    })
  } catch (err) {
    drop(id, err)
  }
}

/**
 * Read the buffer at the point everything written so far has been parsed, and
 * nothing written later has.
 *
 * The callback of an empty write is the only such point, and it has to be *used*
 * rather than merely awaited. xterm invokes it from inside its own parse loop
 * and then carries on consuming the rest of the queue for up to twelve
 * milliseconds before returning -- so a `then` or an `await` on it runs as a
 * microtask, which cannot execute until that loop unwinds, by which time later
 * writes have been parsed too.
 *
 * That is not theoretical. A checkpoint asks for the screen, the flush timer
 * fires before the parser reaches the marker, and the snapshot comes back
 * holding output whose frames were also written to the log after it -- so a
 * restore applies those bytes twice. It reproduces on the first attempt.
 *
 * `read` therefore runs at the marker, synchronously, and its answer is what
 * resolves.
 */
function atDrain<T>(held: Held, read: () => T): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    held.term.write('', () => {
      try {
        resolve(read())
      } catch (err) {
        reject(err instanceof Error ? err : new Error(String(err)))
      }
    })
  })
}

/** Give up on a session's model. The session itself carries on without one. */
function drop(id: string, err: unknown): void {
  log.warn({ err, id }, '[screen] dropping the screen model for this session')
  clearScreen(id)
}

/**
 * The screen, and what has to travel beside it.
 *
 * ## What this does not carry, and why
 *
 * The task asks for scrollback, dimensions, modes, the saved-cursor register,
 * OSC 8 link ranges, the last title and the cwd. Dimensions, title and cwd are
 * gathered here; modes and the alternate-screen flag come from the addon. Three
 * do not, and it is better to say so than to let a caller assume:
 *
 * - **Scrollback** is deliberate: this model runs with none, for the reasons
 *   beside `SCROLLBACK`. The bytes are already kept twice over elsewhere.
 * - **The saved-cursor register** (DECSC) is not emitted by
 *   `@xterm/addon-serialize` at 0.13. A program that saved a position and had
 *   not yet restored it loses that, which shows up as a restore-cursor landing
 *   at the origin.
 * - **OSC 8 link ranges** are stored as cell attributes and the addon does not
 *   round-trip them, so a restored screen has the link text without the link.
 *
 * Reaching those means either an addon that emits them or walking the buffer
 * here. Neither belongs in the task that introduces the model, and both are
 * cheaper once something actually reads a snapshot -- which nothing does yet.
 *
 * Async, and that is not a convenience. `term.write` is queued, not applied —
 * xterm defers the parse to a macrotask — so serializing straight after a write
 * returns whatever had been parsed by then, which is usually nothing and
 * occasionally most of it. That failure is invisible in a test that happens to
 * yield and reappears under load.
 */
export async function serializeScreen(id: string): Promise<ScreenSnapshot | null> {
  const held = screens.get(id)
  if (!held) return null
  try {
    return await atDrain(held, () => ({
      screen: held.serializer.serialize(),
      cols: held.cols,
      rows: held.rows,
      title: held.title,
      cwd: held.cwd
    }))
  } catch (err) {
    log.warn({ err, id }, '[screen] could not serialize')
    return null
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

/**
 * Where this terminal's shell last said it was.
 *
 * Empty until a shell with Vorn's integration reports one. Read by
 * `pty-manager`, which writes it onto the session record so a restored shell can
 * be offered the directory somebody was actually in rather than the one it was
 * launched in.
 */
export function screenCwd(id: string): string {
  return screens.get(id)?.cwd ?? ''
}
