import fs from 'node:fs'
import path from 'node:path'

/**
 * Which implementation of the terminal pipeline this server runs.
 *
 * `js` is everything that ships today. `native` loads `vorn_core.node`, the Rust
 * crate in `packages/core`, and is opt-in through `VORN_CORE=native` while the
 * crate takes over one piece at a time. Anything else, including unset, is `js`:
 * the native path has to be asked for by name.
 */
export type CoreMode = 'js' | 'native'

/** What `packages/core/src/lib.rs` exports. Grows as the crate takes over work. */
export interface NativeCore {
  info(): { version: string; ghostty?: string | null }
  hello(name: string): string
  /** Only present when the crate was built with libghostty-vt. */
  parseTitle?(bytes: Buffer): string
  /** `appendOutput`'s per-chunk analysis. Returns a `NATIVE_STATUS` code. */
  Analyzer?: new () => NativeAnalyzer
  /** The screen model, on libghostty-vt. Only present when built with it. */
  Screen?: new (cols: number, rows: number) => NativeScreen
}

export interface NativeAnalyzer {
  append(data: string, analyze: boolean): number
  output(lines?: number): string[]
}

export interface NativeScreen {
  feed(data: string): void
  resize(cols: number, rows: number): void
  serialize(): { screen: string; cols: number; rows: number; title: string; cwd: string }
  readonly title: string
  readonly cwd: string
}

/** What `NativeAnalyzer.append` returns, in order. */
export const NATIVE_STATUS = [null, 'running', 'waiting', 'error'] as const

export interface CoreSelection {
  mode: CoreMode
  /** Loaded only when `mode` is `native`. */
  native: NativeCore | null
  /** What the loaded binary reported about itself, when `mode` is `native`. */
  info?: ReturnType<NativeCore['info']>
  /** Why the server is on `js` although something else was asked for. */
  fallback?: string
}

export const NATIVE_CORE_FILE = 'vorn_core.node'

export function requestedCoreMode(value: string | undefined): CoreMode | null {
  const normalized = value?.trim().toLowerCase()
  if (!normalized || normalized === 'js') return 'js'
  if (normalized === 'native') return 'native'
  return null
}

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
 * Resolves `VORN_CORE` into the core this process will use.
 *
 * Never throws. A server that was asked for the native core and cannot load it
 * keeps running on the JS path and says why, because the flag is an experiment
 * and a missing binary must not cost anyone their terminals.
 */
export function selectCore(
  options: {
    env?: NodeJS.ProcessEnv
    dir?: string
    load?: (candidates: string[]) => NativeCore
  } = {}
): CoreSelection {
  const env = options.env ?? process.env
  const requested = requestedCoreMode(env.VORN_CORE)
  if (requested === null) {
    return { mode: 'js', native: null, fallback: `unrecognized VORN_CORE=${env.VORN_CORE}` }
  }
  if (requested === 'js') return { mode: 'js', native: null }

  const dir = options.dir ?? serverDir()
  const load = options.load ?? ((candidates) => loadNativeCore(candidates))
  try {
    const native = load(nativeCoreCandidates(dir, env.VORN_CORE_PATH))
    // Call into the binary here, inside the try: a stale or mismatched build can
    // export info() and still throw from it, and that must fall back too.
    const info = native.info()
    if (!info || typeof info.version !== 'string') {
      throw new Error('vorn core info() returned no version')
    }
    return { mode: 'native', native, info }
  } catch (err) {
    return { mode: 'js', native: null, fallback: (err as Error).message }
  }
}

// Same resolution as index.ts: __dirname in the CJS bundle, the entry script's
// directory under tsx.
function serverDir(): string {
  return typeof __dirname !== 'undefined' ? __dirname : path.dirname(process.argv[1])
}

let active: CoreSelection | null = null

/**
 * The core this process runs, resolved once from `VORN_CORE`. The output path
 * reads it per session rather than per chunk.
 */
export function activeCore(): CoreSelection {
  active ??= selectCore()
  return active
}
