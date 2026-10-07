import fs from 'node:fs'
import { syncBuiltinESMExports } from 'node:module'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { afterAll } from 'vitest'

declare module 'vitest' {
  export interface ProvidedContext {
    realHome: string
    sandboxRoot: string
    repoRoot: string
  }
}

interface SandboxState {
  realHome: string
  allowedWrites: string[]
  protectedReads: string[]
  optedIn: boolean
}

const STATE = Symbol.for('vorn.tests.sandbox')
const GUARDED = Symbol.for('vorn.tests.sandbox.guarded')
type Global = typeof globalThis & { [STATE]?: SandboxState; [GUARDED]?: true }

export function isInside(target: string, dir: string): boolean {
  const rel = path.relative(dir, target)
  return rel === '' || (rel !== '..' && !rel.startsWith(`..${path.sep}`) && !path.isAbsolute(rel))
}

function toPath(target: unknown): string | null {
  if (typeof target === 'string') return path.resolve(target)
  if (Buffer.isBuffer(target)) return path.resolve(target.toString())
  if (target instanceof URL && target.protocol === 'file:') return fileURLToPath(target)
  return null
}

/** Throws when the running test file would touch the developer's real home. */
export function assertSandboxed(target: unknown, access: 'read' | 'write'): void {
  const current = (globalThis as Global)[STATE]
  if (!current || current.optedIn) return
  const resolved = toPath(target)
  if (resolved === null) return
  const forbidden =
    access === 'write'
      ? isInside(resolved, current.realHome) &&
        !current.allowedWrites.some((dir) => isInside(resolved, dir))
      : current.protectedReads.some((dir) => isInside(resolved, dir))
  if (forbidden) {
    throw new Error(
      `Test sandbox: refused to ${access} ${resolved}. Build paths from os.homedir() or os.tmpdir(), or opt in with useRealHome().`
    )
  }
}

/** fs functions by base name, and which arguments are paths they touch. */
const WRITES: Record<string, number[]> = {
  writeFile: [0],
  appendFile: [0],
  mkdir: [0],
  mkdtemp: [0],
  rm: [0],
  rmdir: [0],
  unlink: [0],
  rename: [0, 1],
  copyFile: [1],
  cp: [1],
  symlink: [1],
  link: [1],
  truncate: [0],
  chmod: [0],
  utimes: [0]
}
const READS: Record<string, number[]> = {
  readFile: [0],
  readdir: [0],
  stat: [0],
  lstat: [0],
  access: [0],
  exists: [0]
}

function opensForWrite(flags: unknown): boolean {
  if (typeof flags === 'string') return /[wa+]/.test(flags)
  if (typeof flags !== 'number') return false
  const { O_WRONLY, O_RDWR, O_CREAT, O_APPEND, O_TRUNC } = fs.constants
  return (flags & (O_WRONLY | O_RDWR | O_CREAT | O_APPEND | O_TRUNC)) !== 0
}

type Table = Record<string, unknown>

function wrap(table: Table, name: string, check: (args: unknown[]) => void): void {
  const original = table[name]
  if (typeof original !== 'function') return
  const wrapped = function (this: unknown, ...args: unknown[]): unknown {
    check(args)
    return Reflect.apply(original, this, args)
  }
  // Keeps util.promisify.custom, so promisify(fs.exists) still works.
  Object.defineProperties(wrapped, Object.getOwnPropertyDescriptors(original))
  table[name] = wrapped
}

function guardFs(): void {
  const sync = fs as unknown as Table
  const promises = fs.promises as unknown as Table
  const variants = (base: string): Array<[Table, string]> => [
    [sync, base],
    [sync, `${base}Sync`],
    [promises, base]
  ]
  for (const [table, access] of [
    [WRITES, 'write'],
    [READS, 'read']
  ] as const) {
    for (const [base, positions] of Object.entries(table)) {
      for (const [target, name] of variants(base)) {
        wrap(target, name, (args) => positions.forEach((i) => assertSandboxed(args[i], access)))
      }
    }
  }
  for (const [target, name] of variants('open')) {
    wrap(target, name, (args) =>
      assertSandboxed(args[0], opensForWrite(args[1]) ? 'write' : 'read')
    )
  }
  wrap(sync, 'createWriteStream', (args) => assertSandboxed(args[0], 'write'))
  wrap(sync, 'createReadStream', (args) => assertSandboxed(args[0], 'read'))
  syncBuiltinESMExports()
}

/** Gives the current test file a fresh home under the run's temporary root. */
export function installSandbox(options: {
  realHome: string
  sandboxRoot: string
  repoRoot: string
}): string {
  const g = globalThis as Global
  const tmp = os.tmpdir()
  const dataDir = process.env.VORN_DATA_DIR
  g[STATE] = {
    realHome: options.realHome,
    allowedWrites: [tmp, fs.realpathSync(tmp), options.sandboxRoot, options.repoRoot],
    protectedReads: [path.join(options.realHome, '.vorn'), ...(dataDir ? [dataDir] : [])],
    optedIn: false
  }
  if (!g[GUARDED]) {
    guardFs()
    g[GUARDED] = true
  }

  const home = fs.mkdtempSync(path.join(options.sandboxRoot, 'home-'))
  process.env.HOME = home
  process.env.USERPROFILE = home
  if (process.platform === 'win32') {
    process.env.APPDATA = path.join(home, 'AppData', 'Roaming')
    process.env.LOCALAPPDATA = path.join(home, 'AppData', 'Local')
  }
  for (const name of ['XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME', 'XDG_CACHE_HOME']) {
    delete process.env[name]
  }
  // Both are inherited when the suite runs in a Vorn terminal and name the live app's data.
  delete process.env.VORN_DATA_DIR
  delete process.env.VORN_SESSION_ID
  return home
}

/** Opts this test file into the real home; call at top level, before imports that read `os.homedir()`. */
export function useRealHome(): string {
  const current = (globalThis as Global)[STATE]
  if (!current) throw new Error('Test sandbox is not installed')
  const before = { HOME: process.env.HOME, USERPROFILE: process.env.USERPROFILE }
  current.optedIn = true
  process.env.HOME = current.realHome
  process.env.USERPROFILE = current.realHome
  afterAll(() => {
    current.optedIn = false
    for (const [name, value] of Object.entries(before)) {
      if (value === undefined) delete process.env[name]
      else process.env[name] = value
    }
  })
  return current.realHome
}
