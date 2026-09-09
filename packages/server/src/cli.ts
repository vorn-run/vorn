import { startServer } from './index'
import { initDatabase, closeDatabase } from './database'
import { mintOwnerToken, listTokens, hasTokens, revokeToken } from './token-manager'
import { parseServerArgs, ServerArgsError, SERVER_OPTIONS, type ServerArgs } from './server-args'
import { parseClientArgs, ClientArgsError, CLIENT_OPTIONS } from './client-args'
import { useDataDir } from './rpc-client'
import { clientContext, type CliDeps } from './cli/deps'
import { EXIT_FAILURE, EXIT_OK, EXIT_USAGE } from './cli/exit'
import { runSessionCommand } from './cli/session'
import { runWorkflowCommand } from './cli/workflow'

/**
 * The `vorn` command.
 *
 * Two halves behind one name. `vorn server` runs a server on this machine,
 * which is what this file used to be on its own, as `vorn-server`. The rest
 * talks to a server that is already running — starting one first if there is
 * none — and every one of those verbs is an RPC the app has always had.
 *
 * `runCli` takes its argv and its output sinks as arguments and returns an exit
 * code instead of writing to `process` and calling `process.exit`, so every
 * command is assertable. Same shape as `packages/connector-sdk/src/cli.ts`.
 *
 * The two grammars live in `server-args.ts` and `client-args.ts`. The first is
 * shared with the entry point Electron forks, which is why the flags a person
 * types are kept out of it.
 */

export type { CliDeps }

// Replaced at build time (`tsup.config.ts`); absent when run from source.
declare const __CLI_VERSION__: string | undefined

function version(): string {
  return typeof __CLI_VERSION__ === 'undefined' ? 'dev' : __CLI_VERSION__
}

const USAGE = `vorn: Vorn from the command line

Usage
  vorn session start --agent <agent> [--prompt <text>]   Start an agent session
  vorn session list [--recent] [--json]                  What is running
  vorn session logs|send|kill <id>                       Read, steer, stop one
  vorn workflow list [--json]                            Workflows
  vorn workflow run <name> [--input k=v]                 Start one
  vorn workflow stop <run>                               Stop a run
  vorn workflow runs [--workflow <name>] [--json]        Their runs
  vorn server serve|token                                Run a server, or its tokens
  vorn --help | --version

Options common to the commands that talk to a server
  --json              Machine-readable output on stdout
  --data-dir <path>   Reach a server started with the same flag
  --timeout <ms>      Give up on a call after this long

Running \`vorn\` with no command opens the app. With no server running, a command
starts one and says so.
`

const SERVER_USAGE = `vorn server: run a Vorn server without the desktop app

Usage
  vorn server serve [options]              Start the server
  vorn server token create --name <name>   Mint a device token
  vorn server token list                   List device tokens
  vorn server token revoke <id>            Revoke a device token

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
        deps.writeErr('vorn: token create requires --name <name>\n')
        return EXIT_USAGE
      }
      const { token, plaintext } = withDatabase(args.dataDir, () => mintOwnerToken(name))
      printMintedToken(deps, `Created token "${token.name}" (${token.id})`, plaintext)
      return EXIT_OK
    }

    case 'list': {
      const tokens = withDatabase(args.dataDir, () => listTokens())
      if (tokens.length === 0) {
        deps.write('No device tokens.\n')
        return EXIT_OK
      }
      for (const t of tokens) {
        const state = t.revokedAt ? 'revoked' : 'active'
        const seen = t.lastSeenAt ?? 'never'
        deps.write(`${t.id}  ${state.padEnd(7)}  last seen ${seen}  ${t.name}\n`)
      }
      return EXIT_OK
    }

    case 'revoke': {
      const id = rest[0]
      if (!id) {
        deps.writeErr('vorn: token revoke requires a token id\n')
        return EXIT_USAGE
      }
      if (!withDatabase(args.dataDir, () => revokeToken(id))) {
        deps.writeErr(`vorn: no active token with id ${id}\n`)
        return EXIT_FAILURE
      }
      deps.write(`Revoked ${id}\n`)
      return EXIT_OK
    }

    default:
      deps.writeErr(`vorn: unknown token command "${sub ?? ''}". Try: create, list, revoke\n`)
      return EXIT_USAGE
  }
}

async function runServe(args: ServerArgs, deps: CliDeps): Promise<number> {
  const { port } = await startServer({
    host: args.host,
    port: args.port,
    dataDir: args.dataDir,
    // A server somebody ran on purpose does not get to decide it is done. There
    // is no app watching to restart it, and the next line hands out a token for
    // other machines to connect with -- so exiting would strand exactly the
    // clients this command exists to serve.
    idleShutdown: false
  })
  deps.write(`Vorn server listening on port ${port}\n`)

  // A fresh data directory has no way for anyone to authenticate later, so mint
  // one token now rather than making the operator find the token command first.
  if (!hasTokens()) {
    // Auto-start redirects this stream to a log file, and a secret does not go
    // in one. A person watching a terminal still gets the token.
    if (deps.isTty ?? Boolean(process.stdout.isTTY)) {
      const { plaintext } = mintOwnerToken('first-run')
      printMintedToken(
        deps,
        '\nNo device tokens existed, so one was created for this server:',
        plaintext,
        'Manage tokens with: vorn server token list\n\n'
      )
    } else {
      deps.write(
        '\nNo device tokens exist. Mint one with: vorn server token create --name <name>\n'
      )
    }
  }
  return EXIT_OK
}

/** `vorn server ...`, and the bare `serve`/`token` that `vorn-server` has always taken. */
async function runServerCommand(argv: string[], deps: CliDeps): Promise<number> {
  let args: ServerArgs
  try {
    args = parseServerArgs(argv)
  } catch (err) {
    if (err instanceof ServerArgsError) {
      deps.writeErr(`vorn: ${err.message}\n`)
      return EXIT_USAGE
    }
    throw err
  }

  const command = args.positionals[0]

  if (args.help || command === 'help') {
    deps.write(SERVER_USAGE)
    return EXIT_OK
  }
  if (!command) {
    deps.writeErr(SERVER_USAGE)
    return EXIT_USAGE
  }

  switch (command) {
    case 'serve':
      return runServe(args, deps)
    case 'token':
      return runTokenCommand(args, deps)
    default:
      deps.writeErr(`vorn: unknown server command "${command}". Try: serve, token, help\n`)
      return EXIT_USAGE
  }
}

/** The commands that need a server: one grammar, one context, then the noun. */
async function runClientCommand(argv: string[], deps: CliDeps): Promise<number> {
  let args
  try {
    args = parseClientArgs(argv)
  } catch (err) {
    if (err instanceof ClientArgsError) {
      deps.writeErr(`vorn: ${err.message}\n`)
      return EXIT_USAGE
    }
    throw err
  }

  // Discovery reads the port and credential files from here, so this has to be
  // set before the first call rather than passed down to it.
  useDataDir(args.dataDir)

  const ctx = clientContext(deps, args)
  return args.positionals[0] === 'session' ? runSessionCommand(ctx) : runWorkflowCommand(ctx)
}

/** Which option names carry a value, taken from the grammars rather than a list. */
const TAKES_VALUE = new Set(
  [...Object.entries(SERVER_OPTIONS), ...Object.entries(CLIENT_OPTIONS)]
    .filter(([, spec]) => spec.type === 'string')
    .map(([name]) => name)
)

/**
 * The command, wherever it sits.
 *
 * `vorn --data-dir /tmp session list` is how a person writes this, and taking
 * argv[0] alone read the flag as the command and the directory as nothing.
 * Skipping an option's value is what makes the directory not look like a noun.
 */
export function findCommand(argv: string[]): string | undefined {
  for (let i = 0; i < argv.length; i++) {
    const token = argv[i]
    if (!token.startsWith('-')) return token
    if (token.startsWith('--') && !token.includes('=') && TAKES_VALUE.has(token.slice(2))) i++
  }
  return undefined
}

const COMMANDS = ['session', 'workflow', 'server', 'serve', 'token', 'help']

/**
 * An option that swallowed the command, because its own value was missing.
 *
 * `vorn --data-dir session list` parses: `--data-dir` takes `session`, and what
 * is left is `list`, which is not a command. Reported as the missing value it
 * is, rather than as a mystery about `list`. Only consulted once the command has
 * failed to resolve, so `--prompt session` on a real command is left alone.
 */
function optionAteTheCommand(argv: string[]): { option: string; value: string } | undefined {
  for (let i = 0; i < argv.length - 1; i++) {
    const token = argv[i]
    if (!token.startsWith('--') || token.includes('=')) continue
    if (!TAKES_VALUE.has(token.slice(2))) continue
    if (COMMANDS.includes(argv[i + 1])) return { option: token, value: argv[i + 1] }
  }
  return undefined
}

/** The same argv with the command itself taken out, options left where they were. */
function withoutCommand(argv: string[], command: string): string[] {
  const at = argv.indexOf(command)
  return [...argv.slice(0, at), ...argv.slice(at + 1)]
}

/**
 * Run one command. Returns the process exit code; `serve` returns 0 while
 * leaving the server listening, so the caller must not exit on success.
 */
export async function runCli(argv: string[], deps: CliDeps): Promise<number> {
  const command = findCommand(argv)

  if (!command) {
    if (argv.includes('--version')) {
      deps.write(`${version()}\n`)
      return EXIT_OK
    }
    // Asking for help is a successful invocation; being run with nothing is not.
    if (argv.includes('--help') || argv.includes('-h')) {
      deps.write(USAGE)
      return EXIT_OK
    }
    const eaten = optionAteTheCommand(argv)
    if (eaten) {
      deps.writeErr(`vorn: ${eaten.option} needs a value; it took "${eaten.value}" as one\n`)
      return EXIT_USAGE
    }
    deps.writeErr(USAGE)
    return EXIT_USAGE
  }

  switch (command) {
    case 'help':
      deps.write(USAGE)
      return EXIT_OK
    case 'server':
      return runServerCommand(withoutCommand(argv, command), deps)
    // `vorn-server` was called this way for as long as it existed, and scripts
    // that still do keep working.
    case 'serve':
    case 'token':
      return runServerCommand(argv, deps)
    case 'session':
    case 'workflow':
      return runClientCommand(argv, deps)
    default: {
      const eaten = optionAteTheCommand(argv)
      if (eaten) {
        deps.writeErr(`vorn: ${eaten.option} needs a value; it took "${eaten.value}" as one\n`)
        return EXIT_USAGE
      }
      deps.writeErr(`vorn: unknown command "${command}". Try: session, workflow, server, help\n`)
      return EXIT_USAGE
    }
  }
}

// Only when run as the binary — guarded the same way as `index.ts`, so importing
// this module from a test does not start anything. The npm bins resolve through
// symlinks named `vorn` and `vorn-server`, which is why those names are checked.
const isDirectRun = ['cli.ts', 'cli.js', 'cli.cjs', 'vorn', 'vorn-server'].some((name) =>
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
      // and an explicit exit(0) would kill it; the other commands have nothing
      // pending, so the event loop drains and node exits 0 by itself.
      if (code !== 0) process.exit(code)
    })
    .catch((err) => {
      const message = err instanceof Error ? err.stack || err.message : String(err)
      process.stderr.write(`vorn: ${message}\n`)
      process.exit(EXIT_FAILURE)
    })
}
