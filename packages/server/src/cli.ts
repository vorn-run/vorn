import { startServer } from './index'
import { initDatabase, closeDatabase } from './database'
import { mintOwnerToken, listTokens, hasTokens, revokeToken } from './token-manager'
import { parseServerArgs, ServerArgsError, type ServerArgs } from './server-args'

/**
 * Command line entry point for running Vorn's server on its own, without the
 * desktop app.
 *
 * `runCli` takes its argv and its output sink as arguments and returns an exit
 * code instead of writing to `process` and calling `process.exit`, so every
 * command is assertable. Same shape as `packages/connector-sdk/src/cli.ts`.
 *
 * The argument grammar lives in `server-args.ts`, shared with the entry point
 * Electron forks, so the two cannot drift.
 */

export interface CliDeps {
  /** Normal output. Log lines go to stderr (see `logger.ts`), so this is safe to pipe. */
  write(text: string): void
  /** Errors and usage shown on failure. */
  writeErr(text: string): void
}

const USAGE = `vorn-server — run a Vorn server without the desktop app

Usage
  vorn-server serve [options]              Start the server
  vorn-server token create --name <name>   Mint a device token
  vorn-server token list                   List device tokens
  vorn-server token revoke <id>            Revoke a device token

Options
  --port <port>       Port to listen on (default: chosen by the OS)
  --data-dir <path>   Where the database lives (default ~/.vorn)
  -h, --help          Show this message

A server sharing ~/.vorn with a desktop app on the same machine shares one
database. Pass --data-dir to keep them apart.
`

/** The one place a token's plaintext is ever printed. */
function printMintedToken(deps: CliDeps, lead: string, plaintext: string, trailer = ''): void {
  deps.write(
    `${lead}\n\n  ${plaintext}\n\nThis is the only time it is shown. Store it now.\n${trailer}`
  )
}

/** Token commands need the database but not a running server. */
function withDatabase<T>(dataDir: string | undefined, fn: () => T): T {
  initDatabase(dataDir)
  try {
    return fn()
  } finally {
    closeDatabase()
  }
}

function runTokenCommand(args: ServerArgs, deps: CliDeps): number {
  const [, sub, ...rest] = args.positionals

  switch (sub) {
    case 'create': {
      const name = args.name
      if (!name) {
        deps.writeErr('vorn-server: token create requires --name <name>\n')
        return 2
      }
      const { token, plaintext } = withDatabase(args.dataDir, () => mintOwnerToken(name))
      printMintedToken(deps, `Created token "${token.name}" (${token.id})`, plaintext)
      return 0
    }

    case 'list': {
      const tokens = withDatabase(args.dataDir, () => listTokens())
      if (tokens.length === 0) {
        deps.write('No device tokens.\n')
        return 0
      }
      for (const t of tokens) {
        const state = t.revokedAt ? 'revoked' : 'active'
        const seen = t.lastSeenAt ?? 'never'
        deps.write(`${t.id}  ${state.padEnd(7)}  last seen ${seen}  ${t.name}\n`)
      }
      return 0
    }

    case 'revoke': {
      const id = rest[0]
      if (!id) {
        deps.writeErr('vorn-server: token revoke requires a token id\n')
        return 2
      }
      if (!withDatabase(args.dataDir, () => revokeToken(id))) {
        deps.writeErr(`vorn-server: no active token with id ${id}\n`)
        return 1
      }
      deps.write(`Revoked ${id}\n`)
      return 0
    }

    default:
      deps.writeErr(
        `vorn-server: unknown token command "${sub ?? ''}". Try: create, list, revoke\n`
      )
      return 2
  }
}

async function runServe(args: ServerArgs, deps: CliDeps): Promise<number> {
  const { port } = await startServer({
    host: args.host,
    port: args.port,
    dataDir: args.dataDir
  })
  deps.write(`Vorn server listening on port ${port}\n`)

  // A fresh data directory has no way for anyone to authenticate later, so mint
  // one token now rather than making the operator find the token command first.
  if (!hasTokens()) {
    const { plaintext } = mintOwnerToken('first-run')
    printMintedToken(
      deps,
      '\nNo device tokens existed, so one was created for this server:',
      plaintext,
      'Manage tokens with: vorn-server token list\n\n'
    )
  }
  return 0
}

/**
 * Run one command. Returns the process exit code; `serve` returns 0 while
 * leaving the server listening, so the caller must not exit on success.
 */
export async function runCli(argv: string[], deps: CliDeps): Promise<number> {
  let args: ServerArgs
  try {
    args = parseServerArgs(argv)
  } catch (err) {
    if (err instanceof ServerArgsError) {
      deps.writeErr(`vorn-server: ${err.message}\n`)
      return 2
    }
    throw err
  }

  const command = args.positionals[0]

  // Asking for help is a successful invocation; being run with nothing is not.
  if (args.help || command === 'help') {
    deps.write(USAGE)
    return 0
  }
  if (!command) {
    deps.writeErr(USAGE)
    return 2
  }

  switch (command) {
    case 'serve':
      return runServe(args, deps)
    case 'token':
      return runTokenCommand(args, deps)
    default:
      deps.writeErr(`vorn-server: unknown command "${command}". Try: serve, token, help\n`)
      return 2
  }
}

// Only when run as the binary — guarded the same way as `index.ts`, so importing
// this module from a test does not start anything. The npm bin resolves through a
// symlink named `vorn-server`, which is why that name is checked too.
const isDirectRun = ['cli.ts', 'cli.js', 'cli.cjs', 'vorn-server'].some((name) =>
  process.argv[1]?.endsWith(name)
)

if (isDirectRun) {
  const deps: CliDeps = {
    write: (text) => process.stdout.write(text),
    writeErr: (text) => process.stderr.write(text)
  }

  runCli(process.argv.slice(2), deps)
    .then((code) => {
      // Exit only on failure. `serve` returns 0 with the server still listening,
      // and an explicit exit(0) would kill it; the token commands have nothing
      // pending, so the event loop drains and node exits 0 by itself.
      if (code !== 0) process.exit(code)
    })
    .catch((err) => {
      const message = err instanceof Error ? err.stack || err.message : String(err)
      process.stderr.write(`vorn-server: ${message}\n`)
      process.exit(1)
    })
}
