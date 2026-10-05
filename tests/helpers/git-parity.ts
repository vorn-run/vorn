/**
 * What may differ between the server's answer to a `git:`, `file:` or `ide:`
 * call and vornd's own answer to it, and nothing else.
 *
 * Both answers are compared as the frames a client receives, parsed: the id
 * is the caller's and is dropped, and key order is not compared. Everything
 * else must be equal after the normalizers below, each of which names one
 * accepted difference.
 *
 * - {@link fixtureRoot}: a call that changes a repository runs once on each
 *   side, each on its own copy of the fixture, so the copy's directory is in
 *   paths and in git's messages.
 * - {@link worktreeMadeUp}: a new worktree's directory ends in a random id
 *   on either side (so does its branch when the name is taken), and one
 *   created without a name gets a random name.
 * - {@link linkedWorktreeOrder}: git lists linked worktrees in the order of
 *   their directory names, which the random ids decide.
 * - {@link fileMtime}: the same file written on two copies is written at two
 *   moments.
 */

/** A received JSON-RPC frame without its id. */
export type Answer = Record<string, unknown>

/** The frame as a client reads it, without the id it echoes. */
export function answerOf(frame: Record<string, unknown>): Answer {
  const { id: _id, ...rest } = frame
  return JSON.parse(JSON.stringify(rest)) as Answer
}

/** Every path and message naming `root` names `<root>` instead. */
export function fixtureRoot<T>(value: T, root: string): T {
  return mapStrings(value, (s) => s.split(root).join('<root>'))
}

/**
 * What creating worktrees made up on one side, in the order it was made:
 * each worktree's directory id, and the name of each created without one.
 * The nth value is `<made up n>` on both sides, so a value has to recur
 * where the other side's recurs.
 */
export function worktreeMadeUp<T>(value: T, made: readonly string[]): T {
  return mapStrings(value, (s) =>
    made.reduce((out, token, i) => out.split(token).join(`<made up ${i + 1}>`), s)
  )
}

/**
 * The values `git:createWorktree` made up in its answer: the id its
 * directory ends in, and the name when the call gave none. A call git
 * refused names the directory in its error instead.
 */
export function madeUpBy(answer: Answer, givenName: string | undefined): string[] {
  const result = answer.result as { worktreePath?: unknown; name?: unknown } | undefined
  const error = answer.error as { message?: unknown } | undefined
  let dir: string | undefined
  if (result && typeof result.worktreePath === 'string') {
    dir = result.worktreePath.split(/[\\/]/).pop()
  } else if (error && typeof error.message === 'string') {
    dir = /\.vorn-worktrees[\\/][^\\/\s]+[\\/]([A-Za-z0-9-]+)/.exec(error.message)?.[1]
  }
  const parts = dir ? /^(.*)-([0-9a-f]{8})$/.exec(dir) : null
  if (!parts) return []
  return givenName ? [parts[2]] : [parts[2], parts[1]]
}

/**
 * A `git:listWorktrees` answer with the linked worktrees after the main one
 * sorted by path. Apply after {@link worktreeMadeUp}, so the order does not
 * depend on the ids either.
 */
export function linkedWorktreeOrder(answer: Answer): Answer {
  if (!Array.isArray(answer.result)) return answer
  const list = answer.result as Array<{ isMain?: unknown; path?: unknown }>
  const main = list.filter((w) => w.isMain === true)
  const linked = list
    .filter((w) => w.isMain !== true)
    .sort((a, b) => String(a.path).localeCompare(String(b.path)))
  return { ...answer, result: [...main, ...linked] }
}

/** A stamp's `mtimeMs` is `<mtime>`; its size still has to match. */
export function fileMtime<T>(value: T): T {
  const walk = (v: unknown): unknown => {
    if (Array.isArray(v)) return v.map(walk)
    if (v && typeof v === 'object') {
      return Object.fromEntries(
        Object.entries(v).map(([k, inner]) =>
          k === 'mtimeMs' && typeof inner === 'number' ? [k, '<mtime>'] : [k, walk(inner)]
        )
      )
    }
    return v
  }
  return walk(value) as T
}

function mapStrings<T>(value: T, f: (s: string) => string): T {
  const walk = (v: unknown): unknown => {
    if (typeof v === 'string') return f(v)
    if (Array.isArray(v)) return v.map(walk)
    if (v && typeof v === 'object') {
      return Object.fromEntries(Object.entries(v).map(([k, inner]) => [k, walk(inner)]))
    }
    return v
  }
  return walk(value) as T
}
