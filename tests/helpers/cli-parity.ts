/**
 * Where the Rust `vorn` is allowed to differ from the TypeScript `runCli`.
 *
 * Parity is stdout, stderr and the exit code of the same command line against
 * the same server and data directory. Every difference a test accepts is a
 * named normalizer here, applied to both sides, so the list of what differs is
 * one file long and a new difference has to be added on purpose.
 */
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'

/** What one command left behind. */
export interface Ran {
  code: number
  out: string
  err: string
}

const EXE = process.platform === 'win32' ? '.exe' : ''

/** The Rust binary, where `yarn build:core` or cargo put it; undefined when it was not built. */
export const vornBinary: string | undefined = [
  process.env.VORN_CLI_BINARY,
  path.resolve(__dirname, `../../packages/core/vorn${EXE}`),
  path.resolve(__dirname, `../../packages/core/target/release/vorn${EXE}`)
].find((p): p is string => !!p && fs.existsSync(p) && fs.statSync(p).isFile())

/**
 * Runs the binary without blocking the event loop, which a server in this
 * process has to keep answering on.
 */
export function runBinary(args: string[], env: NodeJS.ProcessEnv = process.env): Promise<Ran> {
  return runProgram(vornBinary!, args, env)
}

const CLI_SOURCE = path.resolve(__dirname, '../../packages/server/src/cli.ts')

/**
 * The TypeScript command as its own process, from source: what a person
 * running it gets, one process per command. The token commands run this way,
 * because a process that keeps reopening one database through libsql, as
 * `runCli` in this process would, holds on to SQLite's view of a WAL index the
 * Rust process removes when it closes the file last.
 */
export function runTypeScript(args: string[], env: NodeJS.ProcessEnv = process.env): Promise<Ran> {
  return runProgram(process.execPath, ['--import', 'tsx', CLI_SOURCE, ...args], env)
}

function runProgram(program: string, args: string[], env: NodeJS.ProcessEnv): Promise<Ran> {
  return new Promise((resolve, reject) => {
    const child = spawn(program, args, { env, stdio: ['ignore', 'pipe', 'pipe'] })
    const out: Buffer[] = []
    const err: Buffer[] = []
    child.stdout.on('data', (chunk: Buffer) => out.push(chunk))
    child.stderr.on('data', (chunk: Buffer) => err.push(chunk))
    child.once('error', reject)
    child.once('close', (code) =>
      resolve({
        code: code ?? -1,
        out: Buffer.concat(out).toString('utf-8'),
        err: Buffer.concat(err).toString('utf-8')
      })
    )
  })
}

/**
 * `--version` from a checkout: the TypeScript reads a constant tsup defines
 * at build time and says `dev` from source; the binary always knows the
 * version it was built as.
 */
export const VERSION = 'version-from-source-is-dev'

/**
 * The usage the binary prints names `vorn mcp`, a command only it has.
 */
export const MCP_USAGE_LINE = 'usage-names-vorn-mcp'

/**
 * A minted token is random: its id, its secret, and so its plaintext differ
 * on every mint, from either side.
 */
export const MINTED_TOKEN = 'minted-tokens-are-random'

/**
 * The TypeScript command logs what the server modules it shares do, as JSON
 * lines on stderr: the migrations it ran, the token it minted. The binary
 * keeps its stderr to the messages meant for a person.
 */
export const SERVER_LOG_LINES = 'typescript-logs-json-lines-on-stderr'

const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g

const normalizers: Record<string, (ran: Ran) => Ran> = {
  [VERSION]: (ran) => ({
    ...ran,
    out: ran.out.replace(/^(dev|\d+\.\d+\.\d+\S*)\n$/, '<version>\n')
  }),
  [MCP_USAGE_LINE]: (ran) => {
    const drop = (text: string): string => text.replace(/^ {2}vorn mcp .*\n/m, '')
    return { ...ran, out: drop(ran.out), err: drop(ran.err) }
  },
  [SERVER_LOG_LINES]: (ran) => ({
    ...ran,
    err: ran.err.replace(/^\{"level":\d+,.*\n/gm, '')
  }),
  [MINTED_TOKEN]: (ran) => {
    const scrub = (text: string): string =>
      text.replace(/vorn_[0-9a-f-]{36}_[A-Za-z0-9_-]{43}/g, '<token>').replace(UUID, '<id>')
    return { ...ran, out: scrub(ran.out), err: scrub(ran.err) }
  }
}

/** Both sides with the named differences taken out. */
export function normalized(ran: Ran, accepted: string[]): Ran {
  return accepted.reduce((acc, name) => {
    const normalize = normalizers[name]
    if (!normalize) throw new Error(`no cli-parity normalizer named ${name}`)
    return normalize(acc)
  }, ran)
}
