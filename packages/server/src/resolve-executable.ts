import fs from 'node:fs'
import path from 'node:path'
import { getSafeEnv } from './process-utils'

// Packaged Electron apps on macOS don't inherit the login-shell PATH, so
// user-installed binaries (Homebrew at /opt/homebrew/bin, /usr/local/bin)
// aren't visible to `spawn('name', …)`. We resolve the absolute path once
// from the user's resolved shell env and reuse it.

// Cache only successful lookups — if the user installs `gh` or `git` mid-
// session, we want the next probe to discover it rather than reporting
// "not found" until app restart.
const cache = new Map<string, string>()

/** Where `name` lives on `pathEnv`, or null. On Windows `.exe` and `.cmd` count too. */
export function findOnPath(name: string, pathEnv: string | undefined): string | null {
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

function find(name: string): string | null {
  const env = getSafeEnv()
  return findOnPath(name, env.PATH || env.Path || process.env.PATH)
}

/**
 * Look up an executable by name on the user's PATH (login-shell PATH via
 * `getSafeEnv()`). On Windows also tries `.exe` and `.cmd` suffixes.
 * Successful lookups are cached; misses are re-probed so a fresh install
 * is picked up without an app restart.
 */
export function resolveExecutable(name: string): string | null {
  const hit = cache.get(name)
  if (hit) return hit
  const found = find(name)
  if (found) cache.set(name, found)
  return found
}

export function resetResolveCache(name?: string): void {
  if (name) cache.delete(name)
  else cache.clear()
}
