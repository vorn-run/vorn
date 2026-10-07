/**
 * What differs between two runs of the worktree manager in vornd, and nothing
 * else.
 *
 * {@link normalizeWorktrees} applies each difference below, so a run can be
 * compared with the answers it must give.
 *
 * - {@link RUN_DIR}: each run's own work directory.
 * - {@link MOMENTS}: when the scan was taken and when each worktree was last
 *   touched, which is when the run made it.
 * - {@link BYTES}: sizes, which each run measures on its own copy of the
 *   repository with `du`. Compared as measured or not.
 * - {@link SESSION_IDS}: the ids of the sessions each run started, by the
 *   order they were started in.
 */

export const RUN_DIR = 'each-runs-own-directory'
export const MOMENTS = 'scan-and-touch-times'
export const BYTES = 'sizes-measured-on-each-runs-copy'
export const SESSION_IDS = 'ids-each-run-gave-its-sessions'

const MOMENT_KEYS = new Set(['scannedAt', 'lastTouchedAt'])
const BYTE_KEYS = new Set(['sizeBytes', 'artifactBytes', 'freesBytes', 'freedBytes'])

export function normalizeWorktrees<T>(value: T, workDir: string, sessions: string[] = []): T {
  const walk = (v: unknown): unknown => {
    if (typeof v === 'string') {
      const session = sessions.indexOf(v)
      return session >= 0 ? `<session ${session}>` : v.split(workDir).join('<work>')
    }
    if (Array.isArray(v)) return v.map(walk)
    if (v === null || typeof v !== 'object') return v
    const out: Record<string, unknown> = {}
    for (const [key, field] of Object.entries(v).sort(([a], [b]) => (a < b ? -1 : 1))) {
      if (MOMENT_KEYS.has(key) && typeof field === 'string') out[key] = '<moment>'
      else if (BYTE_KEYS.has(key) && typeof field === 'number') out[key] = field > 0 ? '<bytes>' : 0
      else out[key] = walk(field)
    }
    return out
  }
  return walk(JSON.parse(JSON.stringify(value))) as T
}
