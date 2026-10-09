import { spawn, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { createInterface } from 'node:readline'

/**
 * vornd as the Vorn server, started as a child on a test's own data
 * directory and home, with no other process behind it. Nothing it does
 * reaches this machine's user: the home is the test's, and so is the data
 * directory.
 */

const EXE = process.platform === 'win32' ? '.exe' : ''

/** Where a built vornd is, when there is one. */
export const builtVornd = [
  path.resolve(__dirname, `../../packages/core/vornd${EXE}`),
  path.resolve(__dirname, `../../packages/core/target/release/vornd${EXE}`),
  path.resolve(__dirname, `../../packages/core/target/debug/vornd${EXE}`)
].find((p): p is string => !!p && fs.existsSync(p))

/** The session holder beside it, when it was built too. */
export const builtSessiond = builtVornd
  ? [path.join(path.dirname(builtVornd), `vorn-sessiond${EXE}`)].find((p) => fs.existsSync(p))
  : undefined

export interface Served {
  port: number
  child: ChildProcess
  dataDir: string
  home: string
  stop(): Promise<void>
}

/**
 * Starts vornd serving `dataDir` (a new one by default), with `home` as its
 * home and `credential` as the local credential; with `sessiond`, it keeps a
 * session holder too.
 */
export async function startServed(options: {
  dataDir?: string
  home?: string
  credential: string
  sessiond?: boolean
  env?: Record<string, string>
  args?: string[]
  /** The port to ask for; null asks for none, so vornd picks as an app's would. */
  port?: number | null
}): Promise<Served> {
  if (!builtVornd) throw new Error('vornd is not built')
  const made = (name: string): string =>
    fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), `vorn-served-${name}-`)))
  const dataDir = options.dataDir ?? made('data')
  const home = options.home ?? made('home')
  const port = options.port === undefined ? 0 : options.port
  const args = [
    '--data-dir',
    dataDir,
    ...(port === null ? [] : ['--port', String(port)]),
    ...(options.args ?? [])
  ]
  if (options.sessiond && builtSessiond) args.push('--sessiond', builtSessiond)
  const child = spawn(builtVornd, args, {
    stdio: ['ignore', 'pipe', 'inherit'],
    env: {
      ...process.env,
      HOME: home,
      USERPROFILE: home,
      SECRET_VORN_BOOTSTRAP_TOKEN: options.credential,
      VORND_KEYCHAIN: '0',
      VORND_LOG: process.env.VORND_LOG ?? 'warn',
      ...options.env
    }
  })
  const listening = await new Promise<number>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('vornd did not start')), 20_000)
    createInterface({ input: child.stdout! }).once('line', (line) => {
      clearTimeout(timer)
      resolve((JSON.parse(line) as { port: number }).port)
    })
    child.once('exit', (code) => {
      clearTimeout(timer)
      reject(new Error(`vornd exited with ${code} before listening`))
    })
  })
  const at = listening
  if (options.sessiond && builtSessiond) {
    const deadline = Date.now() + 20_000
    for (;;) {
      const report = await fetch(`http://127.0.0.1:${at}/vornd/sessions`)
        .then((r) => r.json() as Promise<{ connected?: boolean }>)
        .catch(() => null)
      if (report?.connected) break
      if (Date.now() > deadline) throw new Error('vornd never reached its session holder')
      await new Promise((r) => setTimeout(r, 50))
    }
  }
  return {
    port: at,
    child,
    dataDir,
    home,
    stop: async () => {
      // The session holder outlives vornd by design; a test's must not.
      const holder = await fetch(`http://127.0.0.1:${at}/vornd/health`)
        .then((r) => r.json() as Promise<{ sessiond?: { current?: { pid?: number } } }>)
        .then((h) => h.sessiond?.current?.pid ?? null)
        .catch(() => null)
      if (child.exitCode === null && child.signalCode === null) {
        const exited = new Promise((resolve) => child.once('exit', resolve))
        child.kill()
        await exited
      }
      if (holder) {
        try {
          process.kill(holder)
        } catch {
          // Already gone.
        }
      }
    }
  }
}
