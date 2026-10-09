/** The `vorn` binary, run as a person runs it, against `fixtures/cli-reference.json`. */
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

/** What one command left behind. */
export interface Ran {
  code: number
  out: string
  err: string
}

const EXE = process.platform === 'win32' ? '.exe' : ''

/** The binary, where `yarn build:core` or cargo put it; undefined when it was not built. */
export const vornBinary: string | undefined = [
  process.env.VORN_CLI_BINARY,
  path.resolve(__dirname, `../../packages/core/vorn${EXE}`),
  path.resolve(__dirname, `../../packages/core/target/release/vorn${EXE}`)
].find((p): p is string => !!p && fs.existsSync(p) && fs.statSync(p).isFile())

/** A home of the tests' own: no command a test runs reaches this machine's user. */
const testHome = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-cli-home-'))

/** Where every command runs, so a project named after it is named the same on every machine. */
const workDir = path.join(testHome, 'project')
fs.mkdirSync(workDir)

/** This process's environment with a home of its own, no data directory, no colour setting and no vornd. */
export function safeEnv(): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {
    ...process.env,
    HOME: testHome,
    USERPROFILE: testHome,
    VORN_VORND_PATH: path.join(testHome, 'no-vornd')
  }
  delete env.VORN_DATA_DIR
  delete env.NO_COLOR
  return env
}

/** Runs the binary without blocking the event loop, which a server in this process has to keep answering on. */
export function runBinary(args: string[]): Promise<Ran> {
  return new Promise((resolve, reject) => {
    const child = spawn(vornBinary!, args, {
      cwd: workDir,
      env: safeEnv(),
      stdio: ['ignore', 'pipe', 'pipe']
    })
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

const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g

/** `text` with each of `dirs` (and its real path) named by its placeholder, ports and tokens scrubbed. */
export function scrub(text: string, dirs: Record<string, string>): string {
  let scrubbed = text
  for (const [name, dir] of Object.entries({ ...dirs, home: testHome })) {
    for (const form of new Set([fs.realpathSync(dir), dir])) {
      scrubbed = scrubbed.split(form).join(`<${name}>`)
    }
  }
  return scrubbed
    .replace(/<([a-z-]+)>([^\s"]*)/g, (_, name: string, rest: string) => {
      // A path under a placeholder, written with POSIX separators wherever it was printed.
      return `<${name}>${rest.replace(/\\\\|\\/g, '/')}`
    })
    .replace(/127\.0\.0\.1:\d+/g, '127.0.0.1:<port>')
    .replace(/vorn_[0-9a-f-]{36}_[A-Za-z0-9_-]{43}/g, '<token>')
    .replace(/^\d+\.\d+\.\d+\S*\n$/, '<version>\n')
}

/** A token's id, which is random, in what a token command printed. */
export function scrubIds(text: string): string {
  return text.replace(UUID, '<id>')
}

/** The recorded answers; `VORN_RECORD_CLI_REFERENCE=1` records them from the live run instead. */
export class CliReference {
  readonly recording = process.env.VORN_RECORD_CLI_REFERENCE === '1'
  private readonly file = path.join(__dirname, '..', 'fixtures', 'cli-reference.json')
  private readonly answers: Record<string, unknown> =
    this.recording || !fs.existsSync(this.file)
      ? {}
      : (JSON.parse(fs.readFileSync(this.file, 'utf-8')) as Record<string, unknown>)

  want<T>(key: string, live: T): T {
    if (this.recording) {
      if (key in this.answers) throw new Error(`recorded twice: ${key}`)
      this.answers[key] = live
      return live
    }
    if (!(key in this.answers)) throw new Error(`no recorded answer for ${key} in ${this.file}`)
    return this.answers[key] as T
  }

  save(): void {
    if (!this.recording) return
    fs.writeFileSync(this.file, JSON.stringify(this.answers, null, 2) + '\n')
  }
}
