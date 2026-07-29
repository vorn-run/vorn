import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

/**
 * Writing the init files a shell will source.
 *
 * Every shell we spawn reads these, so a file another user could replace is
 * arbitrary code execution as whoever is running Vorn. On Linux os.tmpdir() is
 * the shared /tmp, where the sticky bit protects the entries but not the
 * contents of a directory somebody else created first.
 */

const ROOT = path.join(os.tmpdir(), 'vorn-shell-integration')

const written = new Set<string>()

/** Refuse a directory we do not exclusively own. */
function isSafe(dir: string): boolean {
  const stat = fs.lstatSync(dir)
  if (!stat.isDirectory()) return false
  if (typeof process.getuid === 'function' && stat.uid !== process.getuid()) return false
  // Writable by group or other.
  return (stat.mode & 0o022) === 0
}

/**
 * Write `files` into a per-shell subdirectory and return its path. Cached per
 * subdirectory, since the contents are fixed for the lifetime of the process.
 */
export function writeShimDir(name: string, files: Record<string, string>): string {
  const dir = path.join(ROOT, name)
  if (written.has(name)) return dir

  fs.mkdirSync(dir, { recursive: true, mode: 0o700 })
  if (!isSafe(dir)) {
    throw new Error(`refusing to use shim directory not exclusively owned by this user: ${dir}`)
  }
  // mkdir's mode is masked by umask, and ignored outright when the directory
  // already exists, so set it explicitly.
  fs.chmodSync(dir, 0o700)

  for (const [file, contents] of Object.entries(files)) {
    const target = path.join(dir, file)
    fs.mkdirSync(path.dirname(target), { recursive: true, mode: 0o700 })
    // Truncate through any pre-existing symlink rather than following it.
    fs.rmSync(target, { force: true })
    fs.writeFileSync(target, contents, { mode: 0o600 })
  }
  written.add(name)
  return dir
}

export function resetShimCache(): void {
  written.clear()
}

export { ROOT as SHIM_ROOT }
