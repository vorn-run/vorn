import fs from 'node:fs'
import path from 'node:path'
import type { CoreStatus, ExperimentalConfig, RecordCursor } from '@vornrun/shared/types'

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
  /** The shape `Screen.feed` answers in; absent on a binary from before it was an object. */
  SCREEN_API?: number
  /**
   * Runs one git command off the event loop: answered in-process by gix when it
   * can be answered byte-for-byte as git would, otherwise by `git` on a core
   * thread. Resolves with stdout; rejects as `execFileSync` throws.
   */
  gitRun?(request: NativeGitRequest): Promise<string>
  /** A terminal's screen, scrollback and history framing on a thread of its own. */
  TerminalPipeline?: new (
    cols: number,
    rows: number,
    onEvent: (event: PipelineEvent) => void
  ) => NativePipeline
}

/** Where a record sits in a session's stream; the history log's `RecordHeader`. */
export interface PipelineRecord {
  rseq: number
  startOffset: number
}

/** What a terminal's thread noticed, delivered on the event loop. */
export interface PipelineEvent {
  kind: 'bell' | 'cwd' | 'screen-failed'
  cwd?: string | null
  error?: string | null
}

type NativeSnapshot = ReturnType<NativeScreen['serialize']>

/**
 * One terminal's output, handled on its own thread in the order it was given.
 * Writes return at once; a read waits for everything given before it.
 */
export interface NativePipeline {
  /** One flush: parsed, kept as scrollback, and framed for the history when `at` is given. */
  feed(data: string, at?: PipelineRecord | null): void
  /** The screen alone, for a replay: not kept as scrollback, not framed. */
  feedScreen(data: string): void
  resize(cols: number, rows: number, at?: PipelineRecord | null): void
  restoreLabels(title?: string | null, cwd?: string | null): void
  seedScrollback(data: string): void
  /** Scrollback alone, neither parsed nor recorded. */
  appendScrollback(data: string): void
  serialize(): NativeSnapshot
  scrollback(): string
  /**
   * A checkpoint file's body (absent once the screen model has failed) and the
   * history frames built before it, all at the point in the stream this is
   * called at. Resolves when the thread gets there; rejects if it stops first.
   */
  cut(meta: { generation: number; resume: RecordCursor; closedCleanly?: boolean }): Promise<{
    body?: Buffer | null
    frames: Buffer
  }>
  /** History frames built so far, without waiting for output still queued. */
  takeFrames(): Buffer
  /** Stops the thread and releases the terminal now. */
  free(): void
}

export interface NativeGitRequest {
  /** The git executable, resolved as the JS path resolves it. */
  bin: string
  args: string[]
  cwd: string
  env: Record<string, string>
  timeoutMs: number
  /** Stdout past this many bytes is an error, as `maxBuffer` is for `execFileSync`. */
  maxBuffer: number
}

export interface NativeAnalyzer {
  append(data: string, analyze: boolean): number
  output(lines?: number): string[]
  /** The line in progress, stripped: what the JS path keeps as the partial. */
  partial(): string
  /** Drops the line ring, which V8 does not see, now rather than at GC. */
  free(): void
}

export interface NativeScreen {
  /**
   * Null when the flush moved no cwd and rang no bell. `cwd` is where an OSC
   * 5522 moved to, for the session record; `bell` is a real BEL, not one that
   * ends an OSC.
   */
  feed(data: string): { cwd: string | null; bell: boolean } | null
  restoreLabels(title?: string | null, cwd?: string | null): void
  /** Releases the terminal, whose memory V8 does not see, now rather than at GC. */
  free(): void
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
    return { mode: 'js', native: null, fallback: err instanceof Error ? err.message : String(err) }
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
 * The core this process runs, resolved once from `VORN_CORE` and cached, so
 * the per-chunk lookup in the output path is a field read.
 */
export function activeCore(): CoreSelection {
  active ??= selectCore({ load: loadOverride })
  return active
}

/**
 * A piece of the terminal pipeline that can run on the core, each behind its
 * own switch in Settings › Experimental.
 */
export type NativeFeature = 'screen' | 'analysis' | 'git' | 'pipeline'

const FEATURE_FLAGS: Record<NativeFeature, keyof ExperimentalConfig> = {
  screen: 'nativeScreen',
  analysis: 'nativeAnalysis',
  git: 'nativeGit',
  pipeline: 'nativePipeline'
}

/**
 * The binary's screen model, when it answers `feed` in the shape this server
 * reads. An older binary answers with a cwd string, which would read as a model
 * that never hears a bell, so it counts as having no model rather than guess.
 */
export function screenOf(core: NativeCore | null | undefined): NativeCore['Screen'] {
  return core?.SCREEN_API === 2 ? core.Screen : undefined
}

/** Whether a loaded binary carries a feature; a build without libghostty-vt has no `Screen`. */
const FEATURE_EXPORTS: Record<NativeFeature, (core: NativeCore) => boolean> = {
  screen: (core) => screenOf(core) !== undefined,
  analysis: (core) => typeof core.Analyzer === 'function',
  git: (core) => typeof core.gitRun === 'function',
  pipeline: (core) => typeof core.TerminalPipeline === 'function'
}

type FlagSource = () => ExperimentalConfig | undefined
let readFlags: FlagSource = () => undefined

/**
 * Where the switches are read from. The server points this at its config; with
 * nothing set, as in tests and the bench, only `VORN_CORE` turns a feature on.
 */
export function setExperimentalSource(source: FlagSource | null): void {
  readFlags = source ?? (() => undefined)
}

/**
 * `VORN_CORE` when it was set at all: `native` turns every feature on and `js`
 * every feature off, whatever the switches say. Anything unrecognized counts as
 * `js`, as `selectCore` treats it.
 */
export function forcedCoreMode(value: string | undefined): CoreMode | null {
  if (!value?.trim()) return null
  return requestedCoreMode(value) ?? 'js'
}

let flagged: CoreSelection | null = null

/**
 * The binary, loaded the first time a switch asks for it, and only tried once.
 * Exported for `VORN_GIT=native`, which asks for it without the switch.
 */
export function flaggedCore(): CoreSelection {
  flagged ??= selectCore({ env: { ...process.env, VORN_CORE: 'native' }, load: loadOverride })
  return flagged
}

/**
 * The core a terminal opening now should use for `feature`, or null for the JS
 * path. Read once per terminal rather than per chunk, so a switch flipped while
 * a session runs leaves that session on what it started with.
 *
 * Never throws: a switch that is on with a binary that will not load is the JS
 * path, and `coreStatus` says why.
 */
export function coreFor(feature: NativeFeature): NativeCore | null {
  const forced = forcedCoreMode(process.env.VORN_CORE)
  if (forced === 'js') return null
  if (forced === 'native') return activeCore().native
  return flagsNow()?.[FEATURE_FLAGS[feature]] === true ? flaggedCore().native : null
}

function flagsNow(): ExperimentalConfig | undefined {
  try {
    return readFlags()
  } catch {
    // A config that cannot be read is no switch at all.
    return undefined
  }
}

/**
 * What Settings › Experimental shows beside the switches. Tries the binary if
 * nothing has yet, so the page can say whether turning a switch on would work.
 */
export function coreStatus(): CoreStatus {
  const forced = forcedCoreMode(process.env.VORN_CORE)
  if (forced === 'js') {
    const raw = process.env.VORN_CORE?.trim()
    // Forced to JS all the same, but the page should name the typo, not claim `js`.
    const error = requestedCoreMode(raw) === null ? `VORN_CORE=${raw} is not recognized` : null
    return { loaded: null, version: null, error, forced, missing: [] }
  }
  const selection = forced === 'native' ? activeCore() : flaggedCore()
  const core = selection.native
  const missing = core
    ? (Object.keys(FEATURE_FLAGS) as NativeFeature[])
        .filter((feature) => !FEATURE_EXPORTS[feature](core))
        .map((feature) => FEATURE_FLAGS[feature])
    : []
  return {
    loaded: core !== null,
    version: selection.info?.version ?? null,
    error: core ? null : (selection.fallback ?? null),
    forced,
    missing
  }
}

/**
 * Loads the binary now if any switch is on, so a missing build is in the log at
 * startup rather than when the first terminal opens. Returns what it found.
 */
export function preloadFlaggedCore(): CoreSelection | null {
  if (forcedCoreMode(process.env.VORN_CORE) !== null) return null
  const flags = flagsNow()
  const anyOn = Object.values(FEATURE_FLAGS).some((key) => flags?.[key] === true)
  return anyOn ? flaggedCore() : null
}

/** Test-only: forget every cached selection, and load through `load` from now on. */
export function resetCoreSelection(load?: (candidates: string[]) => NativeCore): void {
  active = null
  flagged = null
  loadOverride = load
}
