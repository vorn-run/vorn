import { parseArgs } from 'node:util'

/**
 * The one place the server's command line grammar lives.
 *
 * Both entry points — `index.ts` when Electron forks it, and `cli.ts` when a
 * person runs `vorn-server` — accept the same three options, and each used to
 * parse them by hand. They had already drifted: one accepted `--port 3000` and
 * rejected a non-numeric port, the other accepted only `--port=3000` and let
 * `NaN` reach `listen()`.
 *
 * Built on `node:util`'s `parseArgs`, which is in the standard library at the
 * node22 target, so this costs no dependency.
 */
export interface ServerArgs {
  host?: string
  port?: number
  dataDir?: string
  /** Label for `token create`. */
  name?: string
  help: boolean
  /** Everything that is not an option: `serve`, `token`, `create`, an id. */
  positionals: string[]
}

export class ServerArgsError extends Error {}

/**
 * Parse server arguments, accepting both `--port=3000` and `--port 3000`.
 *
 * Throws `ServerArgsError` on a malformed value so a caller can decide between
 * printing usage and failing — rather than passing `NaN` down to `listen()`.
 */
export function parseServerArgs(argv: string[]): ServerArgs {
  let values: {
    host?: string
    port?: string
    'data-dir'?: string
    name?: string
    help?: boolean
  }
  let positionals: string[]

  try {
    const parsed = parseArgs({
      args: argv,
      options: {
        host: { type: 'string' },
        port: { type: 'string' },
        'data-dir': { type: 'string' },
        name: { type: 'string' },
        help: { type: 'boolean', short: 'h' }
      },
      allowPositionals: true,
      // Unknown options are reported by us, with the option name in the message,
      // rather than surfacing parseArgs' internal phrasing.
      strict: true
    })
    values = parsed.values
    positionals = parsed.positionals
  } catch (err) {
    throw new ServerArgsError(err instanceof Error ? err.message : String(err))
  }

  let port: number | undefined
  if (values.port !== undefined) {
    port = Number.parseInt(values.port, 10)
    if (!Number.isInteger(port)) {
      throw new ServerArgsError(`--port must be a number, got "${values.port}"`)
    }
  }

  return {
    host: values.host,
    port,
    dataDir: values['data-dir'],
    name: values.name,
    help: values.help ?? false,
    positionals
  }
}

/**
 * Which port to ask for, and whether the answer is worth remembering.
 *
 * Extracted from `startServer` for the reason `parseServerArgs` above it was:
 * the decision was inline in a function that boots a database, a scheduler and a
 * websocket server, so it could not be tested, and it was wrong for as long as
 * it existed. The direct-run entry point passed `port ?? 0`, which turned "no
 * flag given" into an explicit zero — and `0 ?? remembered` is `0`, because `??`
 * only falls through on null and undefined. The remembered port was written on
 * every launch and read on none.
 *
 * That mattered because a browser keys `localStorage` by origin. A port that
 * moves hands the web client a new origin and its stored token stays behind at
 * the old one, which a person experiences as Vorn forgetting them. A phone
 * paired to `ws://host:port/ws` loses it the same way.
 */
export function resolveServerPort(input: {
  /** `--port`, which a person typed and which therefore wins. */
  explicit?: number | null
  /**
   * `defaults.serverPort` — the port this install last settled on.
   *
   * Null is in the type because it is in the data: defaults are read back through
   * `JSON.parse(row.value)`, so a stored JSON null arrives as one. Declaring only
   * `number | undefined` would have made every caller that knows better reach for
   * a cast, which is a worse way to say the same thing.
   */
  remembered?: number | null
  /** The constant, so a first run is predictable rather than random. */
  fallback: number
}): number {
  // `!= null`, not `!== undefined`. Defaults are read back through
  // `JSON.parse(row.value)`, so a stored JSON null arrives as `null` and would
  // pass a strict undefined check — then reach `listen()` as a port, which is how
  // you get an ephemeral one from a value that was meant to say "nothing set".
  // The `??` chain this replaces tolerated null, and dropping that would have
  // been a quiet narrowing. An explicit `0` survives either way, which is the
  // distinction that matters.
  if (input.explicit != null) return input.explicit
  if (input.remembered != null) return input.remembered
  return input.fallback
}

/**
 * Whether the port that was actually bound is worth writing to the configuration.
 *
 * Three rules, and the third is the one with a story.
 *
 * An explicit `--port` is never remembered. It is an instruction for one launch,
 * not a new preference: writing it back would let a single test run quietly
 * repoint every later launch, and the dev override exists precisely so a dev
 * server can differ from the stored value rather than redefine it.
 *
 * A port bound as asked is always remembered. That is the whole mechanism.
 *
 * A *fallback* port — one taken because something else held the port we wanted —
 * is remembered only when nothing was remembered before. The two cases look
 * identical from inside one process and want opposite things:
 *
 * - Another Vorn holds the port, because a dev server and the packaged app share
 *   a data directory. Writing the fallback would overwrite a working remembered
 *   port with an accidental one, and move the *other* instance on its next
 *   launch. Since that config already names a port, this is the case where a
 *   remembered value exists, so nothing is written.
 * - Something unrelated squats the default on a first run. Here there is nothing
 *   to protect, and refusing to write would hand out a fresh random port every
 *   launch forever — the exact failure this all exists to stop. So it is written,
 *   and the install settles on it.
 */
export function shouldRememberPort(input: {
  explicit?: number | null
  remembered?: number | null
  fellBack: boolean
}): boolean {
  if (input.explicit != null) return false
  if (!input.fellBack) return true
  return input.remembered == null
}
