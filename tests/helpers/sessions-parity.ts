/**
 * What may differ between the terminals the server creates and changes and the
 * ones vornd does with the Native server switch on, and nothing else.
 *
 * A run is compared whole, as one transcript: what each call answered, what
 * each agent was started with, what every client was told and what the
 * registry lists at the end. {@link normalizeRun} applies each accepted
 * difference below, and the two transcripts must then be equal.
 *
 * - {@link MINTED}: ids each side makes up as it goes: a session's id, the id a
 *   pinned agent is told to start its conversation under, the hook session
 *   copilot is linked to. Each is `<minted n>`, numbered in the order the
 *   transcript first names it (an object's fields read in the order of their
 *   names, since the two sides write them in orders of their own), so an id
 *   has to recur where the other side's recurs. The ready marker a remote
 *   login waits for, made of a session's id, is `__VORN_READY_<id>__`.
 * - {@link MOMENTS}: when a record was made, started or ended, and the process
 *   it runs as.
 * - {@link REGISTRY_FIELDS}: the revision and stamps of vornd's copy of the
 *   registry, on the records it answers.
 * - {@link RUN_DIRS}: each run's own home and work directories, and their
 *   names, which a shell started in one is named after.
 * - {@link WORKTREE_ID}: the random id in a new worktree's directory.
 * - {@link STATUS}: a status decided as the agent runs, at moments of its own;
 *   `native-server-status.test.ts` compares those.
 * - {@link HOOK_LINK_LATER}: copilot's hooks are installed, and the session
 *   linked to the hook session they report under, once the server hears of
 *   the session. vornd answers a create before that, so its answer has no
 *   link; the server's own answer had it while it set the link in place. The
 *   record has it from its next change on, and the clients are told it then.
 *   Applied by {@link withoutHookLinks} to the answers of creates only.
 * - {@link OUTPUT_CHUNKS}: a headless agent's output reaches the clients in
 *   chunks cut where the pipe happened to deliver them, so each agent's output
 *   is compared whole ({@link outputWhole}).
 * - {@link RESYNC_AFTER_RESUME}: a pane attached to a session of an earlier
 *   run before it was resumed is told to attach again once it runs, by vornd,
 *   which answered the attach; the server answers it with nothing to follow.
 *   Asserted on its own in `native-server-restore.test.ts`, not normalized.
 * - {@link HEADLESS_CARRIED}: a headless agent the holder still holds after a
 *   restart is followed again with the switch on; the server never kept its
 *   record. Asserted on its own there too.
 * - {@link TYPED_BEFORE_PROMPT}: the server types a resumed agent's launch
 *   line after a fixed wait, which on a busy machine can come before the
 *   shell's prompt, and the line is then echoed twice; vornd types it once the
 *   shell has printed. Asserted on the switch-on run only.
 * - {@link PASSWORD_BEFORE_PROMPT}: the server takes the local shell's echo
 *   of the ssh line for the remote shell's ready marker, so it can type a
 *   remote session's command before ssh asks for the password, which then
 *   reads the command as the password; vornd waits for the remote shell to
 *   print the marker. A password login is asserted on the switch-on run only.
 * - {@link SAVED_HEAD_COMMIT}: the server looks up each record's head commit
 *   again as it saves, so a shell's record carries one; vornd writes a record
 *   down as it stands. What an offered session records of its head is left
 *   out by {@link withoutRecordedHeads}.
 */

export const MINTED = 'ids-each-side-makes-up'
export const MOMENTS = 'creation-times-and-process-ids'
export const REGISTRY_FIELDS = 'registry-revisions-and-stamps'
export const RUN_DIRS = 'each-runs-own-directories'
export const WORKTREE_ID = 'random-worktree-directory-ids'
export const STATUS = 'statuses-decided-as-agents-run'
export const HOOK_LINK_LATER = 'copilot-hook-link-after-the-create-answer'
export const OUTPUT_CHUNKS = 'headless-output-chunk-boundaries'
export const RESYNC_AFTER_RESUME = 'resync-told-after-a-cold-resume'
export const HEADLESS_CARRIED = 'headless-agents-carried-over-a-restart'
export const TYPED_BEFORE_PROMPT = 'launch-line-typed-before-the-prompt'
export const PASSWORD_BEFORE_PROMPT = 'remote-command-typed-before-the-password'
export const SAVED_HEAD_COMMIT = 'head-commit-refreshed-by-the-servers-save'

const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g

/** What a remote shell prints once it is up: the session id's first eight characters. */
const READY_MARKER = /__VORN_READY_[0-9a-f-]{8}__/g

/** `<project>/<name>-<8 hex>` under `.vorn-worktrees`. */
const WORKTREE_DIR = /(\.vorn-worktrees\/[^/]+\/[A-Za-z0-9-]+)-[0-9a-f]{8}/g

/** The directories a run made for itself, each named by what it stands for. */
export type RunDirs = Record<string, string>

/** Every accepted difference applied to one run's transcript. */
export function normalizeRun<T>(value: T, dirs: RunDirs): T {
  const minted = new Map<string, string>()
  const mint = (id: string): string => {
    let token = minted.get(id)
    if (!token) {
      token = `<minted ${minted.size + 1}>`
      minted.set(id, token)
    }
    return token
  }
  // Longest first, so a directory inside another is named as itself.
  const roots = Object.entries(dirs).sort(([, a], [, b]) => b.length - a.length)
  const names = new Map(roots.map(([name, dir]) => [dir.split(/[\\/]/).pop()!, `<${name} name>`]))
  const text = (s: string): string => {
    const named = names.get(s)
    if (named) return named
    let out = s
    for (const [name, dir] of roots) out = out.split(dir).join(`<${name}>`)
    return out
      .replace(WORKTREE_DIR, '$1-<id>')
      .replace(READY_MARKER, '__VORN_READY_<id>__')
      .replace(UUID, mint)
  }
  const walk = (v: unknown): unknown => {
    if (typeof v === 'string') return text(v)
    if (Array.isArray(v)) return v.map(walk)
    if (v === null || typeof v !== 'object') return v
    const out: Record<string, unknown> = {}
    const fields = Object.entries(v).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    for (const [key, field] of fields) {
      if (key === 'rev' || key === 'statusAt' || key === 'exitAt') continue
      if (key === 'status' && typeof field === 'string' && isAgentStatus(field)) continue
      if (
        (key === 'createdAt' || key === 'startedAt' || key === 'endedAt') &&
        typeof field === 'number'
      )
        out[key] = '<moment>'
      else if (key === 'pid' && typeof field === 'number') out[key] = field > 0 ? '<pid>' : 0
      else out[key] = walk(field)
    }
    return out
  }
  return walk(JSON.parse(JSON.stringify(value))) as T
}

function isAgentStatus(s: string): boolean {
  return s === 'running' || s === 'waiting' || s === 'idle' || s === 'error'
}

/** Every record in `answers` without the hook session it is linked to. */
export function withoutHookLinks<T>(answers: T): T {
  const walk = (v: unknown): unknown => {
    if (Array.isArray(v)) return v.map(walk)
    if (v === null || typeof v !== 'object') return v
    const out: Record<string, unknown> = {}
    for (const [key, field] of Object.entries(v)) {
      if (key !== 'hookSessionId') out[key] = walk(field)
    }
    return out
  }
  return walk(answers) as T
}

/** Each agent's output whole, from the `headless:data` chunks the clients were told. */
export function outputWhole(
  told: readonly { id: string; data: string }[],
  ids: Readonly<Record<string, string>>
): Record<string, string> {
  return Object.fromEntries(
    Object.entries(ids).map(([name, id]) => [
      name,
      told
        .filter((t) => t.id === id)
        .map((t) => t.data)
        .join('')
    ])
  )
}

/** Each offered session's environment without the head commit its record carried. */
export function withoutRecordedHeads<T extends { environment?: unknown }>(offered: T[]): T[] {
  return offered.map((one) => {
    const environment = one.environment as
      | { head?: { recorded?: unknown; actual?: unknown } }
      | undefined
    if (!environment?.head) return one
    const { recorded: _recorded, ...head } = environment.head
    return { ...one, environment: { ...environment, head } }
  })
}
