import { normalizePath } from './process-utils'

/**
 * Directories a session is being prepared in, before it is a session.
 *
 * Preparing awaits git, and with native git other requests run meanwhile. A
 * worktree action checks for sessions in a path before it deletes anything;
 * one still preparing is not a session yet, so it is held here until it is.
 */
const held = new Map<string, number>()

/** Holds `dir` until the returned release is called; releasing twice is harmless. */
export function holdWorkspace(dir: string): () => void {
  const key = normalizePath(dir)
  held.set(key, (held.get(key) ?? 0) + 1)
  let released = false
  return () => {
    if (released) return
    released = true
    const left = (held.get(key) ?? 1) - 1
    if (left > 0) held.set(key, left)
    else held.delete(key)
  }
}

export function isWorkspaceHeld(dir: string): boolean {
  return held.has(normalizePath(dir))
}
