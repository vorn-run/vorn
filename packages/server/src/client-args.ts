import { parseArgs } from 'node:util'

/**
 * The grammar for the commands that talk to a running server.
 *
 * Kept apart from `server-args.ts` on purpose: that grammar is shared with the
 * entry point Electron forks, so a flag only a person types has no business in
 * it. Both are built on `node:util`'s `parseArgs`, which costs no dependency at
 * the node22 target.
 */
export interface ClientArgs {
  /** The noun, its verb, and their operands: `session`, `start`, an id. */
  positionals: string[]
  agent?: string
  prompt?: string
  /** Project name. Defaults to the basename of the resolved path. */
  project?: string
  /** Project directory. Defaults to the git root of the working directory. */
  path?: string
  /** Display name for a session. */
  name?: string
  branch?: string
  /** Workflow id or name, for the commands that filter by one. */
  workflow?: string
  /** `--input key=value`, repeated: the values a manual run was started with. */
  inputs?: Record<string, string>
  dataDir?: string
  lines?: number
  limit?: number
  timeoutMs?: number
  headless: boolean
  worktree: boolean
  recent: boolean
  /** Send input exactly as given, without the Enter that submits it. */
  raw: boolean
  json: boolean
  help: boolean
}

export class ClientArgsError extends Error {}

/**
 * The options this grammar accepts.
 *
 * Exported because the dispatcher has to know which of them take a value, to
 * find the command in `vorn --data-dir /tmp session list` without mistaking the
 * directory for it.
 */
export const CLIENT_OPTIONS = {
  agent: { type: 'string' },
  prompt: { type: 'string' },
  project: { type: 'string' },
  path: { type: 'string' },
  name: { type: 'string' },
  branch: { type: 'string' },
  workflow: { type: 'string' },
  input: { type: 'string', multiple: true },
  'data-dir': { type: 'string' },
  lines: { type: 'string' },
  limit: { type: 'string' },
  timeout: { type: 'string' },
  headless: { type: 'boolean' },
  worktree: { type: 'boolean' },
  recent: { type: 'boolean' },
  raw: { type: 'boolean' },
  json: { type: 'boolean' },
  help: { type: 'boolean', short: 'h' }
} as const

/** A count, a line budget, a millisecond ceiling: all of them positive integers. */
function positiveInt(raw: string | undefined, flag: string): number | undefined {
  if (raw === undefined) return undefined
  const value = Number.parseInt(raw, 10)
  if (!Number.isInteger(value) || value <= 0) {
    throw new ClientArgsError(`${flag} must be a positive number, got "${raw}"`)
  }
  return value
}

/**
 * Parse a client command line, accepting both `--lines=200` and `--lines 200`.
 *
 * Throws `ClientArgsError` rather than exiting, so the caller decides between
 * printing usage and failing — the same contract `parseServerArgs` has.
 */
export function parseClientArgs(argv: string[]): ClientArgs {
  let values: Record<string, string | boolean | string[] | undefined>
  let positionals: string[]

  try {
    const parsed = parseArgs({
      args: argv,
      options: CLIENT_OPTIONS,
      allowPositionals: true,
      strict: true
    })
    values = parsed.values
    positionals = parsed.positionals
  } catch (err) {
    throw new ClientArgsError(err instanceof Error ? err.message : String(err))
  }

  /** `--input pr=42` pairs, refused rather than guessed at when malformed. */
  const inputs = ((): Record<string, string> | undefined => {
    const given = values.input
    if (!Array.isArray(given) || given.length === 0) return undefined
    const pairs: Record<string, string> = {}
    for (const entry of given) {
      const at = entry.indexOf('=')
      if (at <= 0) throw new ClientArgsError(`--input wants key=value, got "${entry}"`)
      pairs[entry.slice(0, at)] = entry.slice(at + 1)
    }
    return pairs
  })()

  const str = (name: string): string | undefined => {
    const value = values[name]
    return typeof value === 'string' ? value : undefined
  }

  // An empty path is not a path. Left to run, `--data-dir=` would resolve to
  // wherever the command was typed and look for a server nobody started there.
  const dataDir = str('data-dir')
  if (dataDir !== undefined && dataDir.trim() === '') {
    throw new ClientArgsError('--data-dir needs a directory')
  }

  return {
    positionals,
    agent: str('agent'),
    prompt: str('prompt'),
    project: str('project'),
    path: str('path'),
    name: str('name'),
    branch: str('branch'),
    workflow: str('workflow'),
    inputs,
    dataDir,
    lines: positiveInt(str('lines'), '--lines'),
    limit: positiveInt(str('limit'), '--limit'),
    timeoutMs: positiveInt(str('timeout'), '--timeout'),
    headless: values.headless === true,
    worktree: values.worktree === true,
    recent: values.recent === true,
    raw: values.raw === true,
    json: values.json === true,
    help: values.help === true
  }
}
