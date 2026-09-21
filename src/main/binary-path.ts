import fs from 'node:fs'
import path from 'node:path'
import log from './logger'

/**
 * Finding a binary the person installed, from a process that has no shell.
 *
 * An app launched from Finder or the Dock inherits
 * `/usr/bin:/bin:/usr/sbin:/sbin` and nothing else, so Homebrew's
 * `/opt/homebrew/bin` — where `idb_companion` lives — is invisible to
 * `spawn('name', …)`. The spawn fails with ENOENT, which reads as "not
 * installed" while the binary sits on disk. That is the misleading answer
 * this module exists to prevent.
 *
 * The server already asks a login shell for the real PATH
 * (`packages/server/src/process-utils.ts`), so the first question is put to
 * it, over the bridge. Nothing waits on that answer to begin with: the fixed
 * Homebrew directories below cover an ordinary install, and a claim that
 * would otherwise fail is worth a short wait only once they have missed.
 *
 * `findOnPath` is deliberately a twin of the server's
 * (`packages/server/src/resolve-executable.ts`) rather than a shared import.
 * Neither shared directory will take it: `packages/shared` is free of `node:*`
 * because the renderer loads it, and `src/shared` is in the renderer's own
 * build. Fifteen lines of `fs.accessSync` is the cheaper of the two prices.
 */

/** Where `name` lives on `pathEnv`, or null. On Windows `.exe` and `.cmd` count too. */
function findOnPath(name: string, pathEnv: string | undefined): string | null {
  if (!pathEnv) return null
  const sep = process.platform === 'win32' ? ';' : ':'
  const candidates = process.platform === 'win32' ? [`${name}.exe`, `${name}.cmd`, name] : [name]
  for (const rawDir of pathEnv.split(sep)) {
    const dir = rawDir.trim()
    if (!dir) continue
    for (const candidate of candidates) {
      const full = path.join(dir, candidate)
      try {
        fs.accessSync(full, fs.constants.X_OK)
        return full
      } catch {
        /* not here */
      }
    }
  }
  return null
}

/**
 * Where a package manager puts binaries, for a process whose PATH says
 * nothing about them: Homebrew on Apple Silicon, then on Intel. Empty on
 * Windows, where an installer writes its own PATH entry instead.
 */
const EXTRA_BIN_DIRS: readonly string[] =
  process.platform === 'win32' ? [] : ['/opt/homebrew/bin', '/usr/local/bin']

/** Whether this exact path is a file that can be run, for a hand-written override. */
function isExecutableFile(candidate: string): boolean {
  try {
    if (!fs.statSync(candidate).isFile()) return false
    fs.accessSync(candidate, fs.constants.X_OK)
    return true
  } catch {
    return false
  }
}

/** What this module needs of the server bridge, so a test can stand in for it. */
export interface PathSource {
  request<T>(method: string, params?: unknown, timeoutMs?: number): Promise<T>
}

let source: PathSource | null = null
/** `final` once the server's login shell has answered; until then the PATH may still improve. */
let host: { path: string | null; final: boolean } = { path: null, final: false }
let asking: Promise<void> | null = null
/** Binaries already located, cleared whenever the PATH they were found on changes. */
const found = new Map<string, { path: string; searched: string[] }>()

export function setPathSource(next: PathSource | null): void {
  source = next
}

/** Drop what a previous server said, for when a different one takes over. */
export function resetHostPath(): void {
  host = { path: null, final: false }
  asking = null
  found.clear()
}

/** The server's PATH as last heard; never waits, so a caller can try it for free. */
export function hostPath(): string | null {
  return host.path
}

/**
 * Ask the server for its PATH, at most once at a time.
 *
 * A refusal is kept quiet and changes nothing: the bridge may be down, or the
 * server may be restarting, and neither is a reason for a device claim to
 * fail. The fixed directories still answer for an ordinary install.
 */
export function primeHostPath(): Promise<void> {
  if (host.final) return Promise.resolve()
  if (asking) return asking
  const pending = (async () => {
    try {
      const answer = await source?.request<{ path: string | null; resolved: boolean }>(
        'env:path',
        undefined,
        10_000
      )
      if (answer?.path && answer.path !== host.path) found.clear()
      if (answer?.path) host = { path: answer.path, final: answer.resolved }
    } catch (err) {
      log.debug({ err }, '[binary-path] the server did not say what its PATH is')
    } finally {
      asking = null
    }
  })()
  asking = pending
  return pending
}

/** Wait for the server's answer, but never longer than the caller can spare. */
async function hostPathSettled(maxMs: number): Promise<void> {
  if (host.final || !asking) return
  await Promise.race([asking, new Promise((resolve) => setTimeout(resolve, maxMs))])
}

export interface BinaryResolution {
  /** The absolute path, or null when nothing was found. */
  path: string | null
  /** The directories actually looked in, in order, for an error worth reading. */
  searched: string[]
  /** Set when an override names something that cannot be run; the search stops there. */
  overrideMiss?: string
}

function dirsOf(pathEnv: string | null | undefined): string[] {
  const sep = process.platform === 'win32' ? ';' : ':'
  return (pathEnv ?? '')
    .split(sep)
    .map((dir) => dir.trim())
    .filter(Boolean)
}

/**
 * Where a binary is, without waiting for anything.
 *
 * An override that names something unrunnable ends the search: a person who
 * set it is owed that answer rather than a report that the binary is missing
 * entirely.
 */
export function resolveBinary(name: string, overrideEnvVar?: string): BinaryResolution {
  const override = overrideEnvVar ? process.env[overrideEnvVar]?.trim() : undefined
  if (override) {
    return isExecutableFile(override)
      ? { path: override, searched: [override] }
      : { path: null, searched: [], overrideMiss: override }
  }

  // A hit is remembered, the way the server's resolver does: a device claim
  // asks every time, and the answer cannot change while the PATH it was found
  // on stays the same. A miss is never cached, so a binary installed
  // mid-session is picked up without a restart.
  const remembered = found.get(name)
  if (remembered) return { path: remembered.path, searched: remembered.searched }

  const searched: string[] = []
  // The server's PATH first: it is the one the person actually has.
  for (const pathEnv of [hostPath(), process.env.PATH, EXTRA_BIN_DIRS.join(path.delimiter)]) {
    const dirs = dirsOf(pathEnv).filter((dir) => !searched.includes(dir))
    if (dirs.length === 0) continue
    const hit = findOnPath(name, dirs.join(path.delimiter))
    searched.push(...dirs)
    if (hit) {
      found.set(name, { path: hit, searched })
      return { path: hit, searched }
    }
  }
  return { path: null, searched }
}

/**
 * Where a binary is, waiting once for the server's PATH if the rest missed.
 *
 * The wait is paid only when a binary lives somewhere unusual — by which
 * point the alternative is failing — and the prime at startup usually means
 * there is nothing left to wait for.
 */
export async function resolveBinaryWaiting(
  name: string,
  overrideEnvVar?: string,
  maxMs = 3_000
): Promise<BinaryResolution> {
  const first = resolveBinary(name, overrideEnvVar)
  if (first.path || first.overrideMiss || host.final) return first
  void primeHostPath()
  await hostPathSettled(maxMs)
  return resolveBinary(name, overrideEnvVar)
}

/** A PATH for a child that must find tools of its own, however this app was started. */
export function augmentedPath(): string {
  const dirs = [
    ...dirsOf(hostPath()),
    ...dirsOf(process.env.PATH),
    ...EXTRA_BIN_DIRS.filter((dir) => fs.existsSync(dir))
  ]
  return [...new Set(dirs)].join(path.delimiter)
}
