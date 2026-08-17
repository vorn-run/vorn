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
