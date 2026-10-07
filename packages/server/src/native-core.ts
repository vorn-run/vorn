import fs from 'node:fs'
import path from 'node:path'
import type { CoreStatus } from '@vornrun/shared/types'

/**
 * What `packages/core/src/lib.rs` exports: `vorn_core.node`, the Rust crate in
 * `packages/core`, which runs git and the native store. Terminals run in vornd.
 */
export interface NativeCore {
  info(): { version: string }
  hello(name: string): string
  /**
   * Runs one git command off the event loop: answered in-process by gix when it
   * can be answered byte-for-byte as git would, otherwise by `git` on a core
   * thread. Resolves with stdout; rejects as `execFileSync` throws.
   */
  gitRun?(request: NativeGitRequest): Promise<string>
  /** The store on rusqlite, which keeps the server's database (`database.ts`). */
  NativeStore?: NativeStoreClass
}

/** Opens a database in the Rust store. `options` is JSON: what the store needs from the server. */
export interface NativeStoreClass {
  open(path: string, options: string): NativeStore
  openInMemory(options: string): NativeStore
}

/** One open database in the Rust store. */
export interface NativeStore {
  /** Calls the `database.ts` function named `call` with a JSON array of its arguments; returns its result as JSON. */
  call(call: string, args: string): string
  close(): void
  /** Where a corrupt file was copied before the store replaced it, or null. */
  readonly recovered: string | null
}

export interface NativeGitRequest {
  /** The git executable, resolved by `gitBin`. */
  bin: string
  args: string[]
  cwd: string
  env: Record<string, string>
  timeoutMs: number
  /** Stdout past this many bytes is an error, as `maxBuffer` is for `execFileSync`. */
  maxBuffer: number
}

export interface CoreSelection {
  /** Null when the binary could not be loaded. */
  native: NativeCore | null
  /** What the loaded binary reported about itself. */
  info?: ReturnType<NativeCore['info']>
  /** Why there is no core. */
  error?: string
}

export const NATIVE_CORE_FILE = 'vorn_core.node'

/**
 * Where `vorn_core.node` may be, in the order they are tried.
 *
 * `dir` is the directory of the running server code: `packages/server/src` under
 * tsx, `packages/server/dist` for a built checkout, and `resources/server` in a
 * packaged app, where electron-builder puts the binary at `resources/core`.
 */
export function nativeCoreCandidates(dir: string, override?: string): string[] {
  const candidates = [
    path.join(dir, '..', 'core', NATIVE_CORE_FILE),
    path.join(dir, '..', '..', 'core', NATIVE_CORE_FILE)
  ]
  return override ? [path.resolve(override), ...candidates] : candidates
}

type Dlopen = (file: string) => unknown

/**
 * `process.dlopen` rather than `require`, because this file runs both as ESM
 * under tsx, where there is no `require`, and inside the CJS bundle, where
 * `import.meta` must not appear.
 */
function dlopen(file: string): unknown {
  const mod = { exports: {} }
  process.dlopen(mod, file)
  return mod.exports
}

export function loadNativeCore(
  candidates: string[],
  deps: { exists?: (file: string) => boolean; open?: Dlopen } = {}
): NativeCore {
  const exists = deps.exists ?? fs.existsSync
  const open = deps.open ?? dlopen
  const file = candidates.find((candidate) => exists(candidate))
  if (!file) {
    throw new Error(
      `${NATIVE_CORE_FILE} not found; build it with \`yarn build:core\` (looked in ${candidates.join(', ')})`
    )
  }
  const core = open(file) as Partial<NativeCore>
  if (typeof core.info !== 'function' || typeof core.hello !== 'function') {
    throw new Error(`${file} is not a vorn core: it exports no info() or hello()`)
  }
  return core as NativeCore
}

/**
 * Loads the core this process will use.
 *
 * Never throws. Every build ships the binary, so a server without one is a
 * checkout that has not run `yarn build:core`, or a broken install. Without it
 * git runs on a child process and the database cannot open (`initDatabase`
 * throws, saying why).
 */
export function selectCore(
  options: {
    env?: NodeJS.ProcessEnv
    dir?: string
    load?: (candidates: string[]) => NativeCore
  } = {}
): CoreSelection {
  const env = options.env ?? process.env
  const dir = options.dir ?? serverDir()
  const load = options.load ?? ((candidates) => loadNativeCore(candidates))
  try {
    const native = load(nativeCoreCandidates(dir, env.VORN_CORE_PATH))
    // Call into the binary here, inside the try: a stale or mismatched build can
    // export info() and still throw from it, and that must not throw either.
    const info = native.info()
    if (!info || typeof info.version !== 'string') {
      throw new Error('vorn core info() returned no version')
    }
    return { native, info }
  } catch (err) {
    return { native: null, error: err instanceof Error ? err.message : String(err) }
  }
}

// Same resolution as index.ts: __dirname in the CJS bundle, the entry script's
// directory under tsx.
function serverDir(): string {
  return typeof __dirname !== 'undefined' ? __dirname : path.dirname(process.argv[1])
}

let active: CoreSelection | null = null
/** Test-only, through `resetCoreSelection`. */
let loadOverride: ((candidates: string[]) => NativeCore) | undefined

/**
 * The core this process runs, loaded once and cached, so the lookup when a
 * terminal opens or git runs is a field read.
 */
export function activeCore(): CoreSelection {
  active ??= selectCore({ load: loadOverride })
  return active
}

/** The loaded core, or null when there is none. */
export function nativeCore(): NativeCore | null {
  return activeCore().native
}

/**
 * The exports the server uses beyond `info` and `hello`, and what is missing
 * without them: absent only from a binary older than the server.
 */
const OPTIONAL_EXPORTS: Array<[keyof NativeCore, string]> = [
  ['gitRun', 'git off the main thread'],
  ['NativeStore', 'the native store']
]

/** What Settings › About shows about the core. */
export function coreStatus(): CoreStatus {
  const selection = activeCore()
  const core = selection.native
  return {
    loaded: core !== null,
    version: selection.info?.version ?? null,
    error: core ? null : (selection.error ?? null),
    missing: core
      ? OPTIONAL_EXPORTS.filter(([key]) => typeof core[key] !== 'function').map(([, what]) => what)
      : []
  }
}

/** Test-only: forget the cached core, and load through `load` from now on. */
export function resetCoreSelection(load?: (candidates: string[]) => NativeCore): void {
  active = null
  loadOverride = load
}
