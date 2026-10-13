/**
 * Intent bar completion engine.
 *
 * Suggestions are computed from the token under the cursor, using small
 * declarative "command outlines" (subcommands, flags, argument kinds) plus
 * live data sources: PATH executables, directory listings, git branches, and
 * package.json scripts. All sources are injected so the engine is pure and
 * testable; `defaultSources` wires the real ones over window.api.
 *
 * The engine never decides to run anything — it only proposes replacement
 * text for the current token. Submission stays a deliberate user action.
 */

import { generatedOutlineSource, type OutlineSource } from './completion-index'

export type CompletionKind = 'command' | 'subcommand' | 'flag' | 'path' | 'branch' | 'script'

export interface Completion {
  /** Replacement for the current token. */
  insert: string
  label: string
  detail?: string
  kind: CompletionKind
  /** Directory: inserting keeps the menu open to drill further. */
  continues?: boolean
}

export type ArgKind = 'path' | 'dir' | 'branch' | 'script' | 'none'

export interface FlagDef {
  flag: string
  detail?: string
}

export interface Outline {
  detail?: string
  arg?: ArgKind
  flags?: FlagDef[]
  sub?: Record<string, Outline>
}

const GIT: Outline = {
  detail: 'version control',
  sub: {
    status: { detail: 'working tree status', arg: 'none' },
    add: { detail: 'stage changes', arg: 'path', flags: [{ flag: '-p', detail: 'stage hunks' }] },
    commit: {
      detail: 'record staged changes',
      arg: 'none',
      flags: [
        { flag: '-m', detail: 'message' },
        { flag: '-a', detail: 'stage tracked files' },
        { flag: '--amend', detail: 'rewrite last commit' },
        { flag: '--no-verify', detail: 'skip hooks' }
      ]
    },
    push: {
      detail: 'update remote',
      arg: 'none',
      flags: [
        { flag: '-u', detail: 'set upstream' },
        { flag: '--force-with-lease', detail: 'safe force push' },
        { flag: '--tags', detail: 'push tags' }
      ]
    },
    pull: { detail: 'fetch and merge', arg: 'none', flags: [{ flag: '--rebase' }] },
    fetch: {
      detail: 'download refs',
      arg: 'none',
      flags: [{ flag: '--all' }, { flag: '--prune' }]
    },
    checkout: {
      detail: 'switch branch or restore',
      arg: 'branch',
      flags: [{ flag: '-b', detail: 'create branch' }]
    },
    switch: {
      detail: 'switch branch',
      arg: 'branch',
      flags: [{ flag: '-c', detail: 'create branch' }]
    },
    branch: {
      detail: 'list or manage branches',
      arg: 'branch',
      flags: [
        { flag: '-d', detail: 'delete merged' },
        { flag: '-D', detail: 'force delete' },
        { flag: '-m', detail: 'rename' }
      ]
    },
    merge: {
      detail: 'merge branch',
      arg: 'branch',
      flags: [{ flag: '--abort' }, { flag: '--no-ff' }]
    },
    rebase: {
      detail: 'reapply commits',
      arg: 'branch',
      flags: [{ flag: '--continue' }, { flag: '--abort' }, { flag: '-i', detail: 'interactive' }]
    },
    log: {
      detail: 'commit history',
      arg: 'none',
      flags: [{ flag: '--oneline' }, { flag: '--graph' }]
    },
    diff: {
      detail: 'show changes',
      arg: 'path',
      flags: [{ flag: '--staged' }, { flag: '--stat' }]
    },
    restore: {
      detail: 'discard changes',
      arg: 'path',
      flags: [{ flag: '--staged', detail: 'unstage' }]
    },
    reset: { detail: 'move HEAD', arg: 'none', flags: [{ flag: '--hard' }, { flag: '--soft' }] },
    stash: {
      detail: 'shelve changes',
      sub: {
        push: { detail: 'stash working tree', arg: 'none' },
        pop: { detail: 'apply and drop', arg: 'none' },
        list: { arg: 'none' },
        apply: { arg: 'none' },
        drop: { arg: 'none' }
      }
    },
    'cherry-pick': {
      detail: 'apply a commit',
      arg: 'none',
      flags: [{ flag: '--continue' }, { flag: '--abort' }]
    },
    worktree: {
      detail: 'manage worktrees',
      sub: {
        add: { detail: 'create worktree', arg: 'path' },
        list: { arg: 'none' },
        remove: { detail: 'delete worktree', arg: 'path' }
      }
    },
    remote: { detail: 'manage remotes', arg: 'none', flags: [{ flag: '-v' }] },
    tag: { detail: 'manage tags', arg: 'none' },
    show: { detail: 'inspect objects', arg: 'none' },
    blame: { detail: 'line authorship', arg: 'path' },
    revert: { detail: 'undo a commit', arg: 'none' },
    clone: { detail: 'copy a repository', arg: 'none' },
    init: { detail: 'create a repository', arg: 'none' }
  }
}

const YARN: Outline = {
  detail: 'package manager',
  arg: 'script',
  sub: {
    install: { detail: 'install dependencies', arg: 'none' },
    add: {
      detail: 'add dependency',
      arg: 'none',
      flags: [{ flag: '-D', detail: 'dev dependency' }]
    },
    remove: { detail: 'remove dependency', arg: 'none' },
    run: { detail: 'run package script', arg: 'script' },
    why: { detail: 'explain dependency', arg: 'none' },
    upgrade: { detail: 'upgrade dependencies', arg: 'none' }
  }
}

const NPM: Outline = {
  detail: 'package manager',
  sub: {
    install: {
      detail: 'install dependencies',
      arg: 'none',
      flags: [{ flag: '-D', detail: 'dev dependency' }]
    },
    run: { detail: 'run package script', arg: 'script' },
    test: { arg: 'none' },
    ci: { detail: 'clean install', arg: 'none' },
    init: { arg: 'none' },
    publish: { arg: 'none' }
  }
}

/** Commands with structured outlines; anything else falls back to paths. */
const OUTLINES: Record<string, Outline> = {
  git: GIT,
  yarn: YARN,
  npm: NPM,
  pnpm: { ...YARN, detail: 'package manager' },
  cd: { detail: 'change directory', arg: 'dir' },
  ls: { detail: 'list directory', arg: 'path' },
  cat: { detail: 'print file', arg: 'path' },
  code: { detail: 'open in editor', arg: 'path' },
  open: { detail: 'open with default app', arg: 'path' },
  mkdir: { detail: 'create directory', arg: 'dir' },
  rm: {
    detail: 'remove',
    arg: 'path',
    flags: [
      { flag: '-r', detail: 'recursive' },
      { flag: '-f', detail: 'force' }
    ]
  },
  cp: { detail: 'copy', arg: 'path' },
  mv: { detail: 'move or rename', arg: 'path' },
  touch: { detail: 'create file', arg: 'path' },
  vim: { detail: 'edit file', arg: 'path' },
  nano: { detail: 'edit file', arg: 'path' },
  head: { arg: 'path' },
  tail: { arg: 'path', flags: [{ flag: '-f', detail: 'follow' }] },
  node: { detail: 'run JavaScript', arg: 'path' },
  python: { detail: 'run Python', arg: 'path' },
  source: { detail: 'run in current shell', arg: 'path' },
  which: { detail: 'locate executable', arg: 'none' },
  grep: {
    detail: 'search text',
    arg: 'path',
    flags: [
      { flag: '-r', detail: 'recursive' },
      { flag: '-i', detail: 'ignore case' }
    ]
  }
}

/** Commands with a structured outline. Feeds intent-mode resolution. */
export function outlineNames(): string[] {
  return Object.keys(OUTLINES)
}

export interface DirEntry {
  name: string
  isDirectory: boolean
}

export interface CompletionSources {
  /** Live working directory of the session, when known. */
  cwd: string | null
  listDir(dir: string): Promise<DirEntry[]>
  listBranches(): Promise<string[]>
  listExecutables(): Promise<string[]>
  listScripts(): Promise<string[]>
  /**
   * Generated outlines, layered under the hand-written ones. Optional so
   * callers that only need the curated set keep working unchanged.
   */
  outlines?: OutlineSource
}

/**
 * Resolve a command's outline.
 *
 * Hand-written outlines win wholesale — no deep merge. The curated set
 * exists precisely because it is better than generic extraction (it knows
 * which arguments are branches and which are package scripts), and merging
 * generated flags into it would make its behaviour unpredictable.
 */
async function resolveOutline(
  name: string,
  sources: CompletionSources
): Promise<Outline | undefined> {
  return OUTLINES[name] ?? (await safeCall(async () => sources.outlines?.outline(name), undefined))
}

const MAX_RESULTS = 12

async function safeCall<T>(fn: () => Promise<T>, fallback: T): Promise<T> {
  try {
    return await fn()
  } catch {
    return fallback
  }
}

/** Escape shell-significant characters so an inserted name stays one token. */
function escapeToken(name: string): string {
  return name.replace(/([ '"\\()&;|<>*?$`#])/g, '\\$1')
}

function prefixFilter(names: string[], prefix: string): string[] {
  const lower = prefix.toLowerCase()
  return names.filter((n) => n.toLowerCase().startsWith(lower) && n !== prefix)
}

async function completePaths(
  current: string,
  sources: CompletionSources,
  dirsOnly: boolean
): Promise<Completion[]> {
  if (current.startsWith('~')) return []
  const slash = current.lastIndexOf('/')
  const dirPart = slash >= 0 ? current.slice(0, slash + 1) : ''
  const base = slash >= 0 ? current.slice(slash + 1) : current
  let dir: string
  if (dirPart.startsWith('/')) {
    dir = dirPart
  } else {
    if (!sources.cwd) return []
    dir = dirPart ? `${sources.cwd}/${dirPart}` : sources.cwd
  }
  const entries = await safeCall(() => sources.listDir(dir), [] as DirEntry[])
  const lower = base.toLowerCase()
  return entries
    .filter((e) => e.name.toLowerCase().startsWith(lower) && (!dirsOnly || e.isDirectory))
    .sort((a, b) =>
      a.isDirectory === b.isDirectory ? a.name.localeCompare(b.name) : a.isDirectory ? -1 : 1
    )
    .slice(0, MAX_RESULTS)
    .map((e) => ({
      insert: dirPart + escapeToken(e.name) + (e.isDirectory ? '/' : ''),
      label: e.name + (e.isDirectory ? '/' : ''),
      kind: 'path' as const,
      continues: e.isDirectory
    }))
}

async function completeCommands(
  current: string,
  sources: CompletionSources
): Promise<Completion[]> {
  if (!current) return []
  const executables = await safeCall(() => sources.listExecutables(), [] as string[])
  const generated = await safeCall(
    async () => (await sources.outlines?.names()) ?? new Map<string, string | undefined>(),
    new Map<string, string | undefined>()
  )
  const names = new Set([...Object.keys(OUTLINES), ...generated.keys(), ...executables])
  // Curated first, then anything with a description, then bare PATH names —
  // a name with a hint is more use than one without.
  const rank = (name: string): number => {
    if (name in OUTLINES) return 0
    return generated.get(name) ? 1 : 2
  }
  const matches = prefixFilter([...names], current).sort((a, b) => {
    const ra = rank(a)
    const rb = rank(b)
    return ra !== rb ? ra - rb : a.localeCompare(b)
  })
  return matches.slice(0, MAX_RESULTS).map((name) => ({
    insert: name,
    label: name,
    detail: OUTLINES[name]?.detail ?? generated.get(name),
    kind: 'command' as const
  }))
}

/**
 * Completions for the token at the end of `input` (last line only).
 * Returns [] when there is nothing useful to offer.
 */
export async function getCompletions(
  input: string,
  sources: CompletionSources
): Promise<Completion[]> {
  const line = input.slice(input.lastIndexOf('\n') + 1)
  const endsWithSpace = /\s$/.test(line)
  const words = line.trim() ? line.trim().split(/\s+/) : []
  const current = endsWithSpace ? '' : (words.pop() ?? '')
  const prior = words

  // Command position
  if (prior.length === 0) {
    return completeCommands(current, sources)
  }

  // Walk the outline chain through matched subcommands.
  let node: Outline | undefined = await resolveOutline(prior[0], sources)
  let unmatchedArgs = 0
  for (const token of prior.slice(1)) {
    if (token.startsWith('-')) continue
    const child: Outline | undefined = node?.sub?.[token]
    if (child) {
      node = child
    } else {
      unmatchedArgs++
    }
  }

  const results: Completion[] = []

  if (current.startsWith('-')) {
    const flags = node?.flags ?? []
    for (const f of prefixFilterFlags(flags, current)) {
      results.push({ insert: f.flag, label: f.flag, detail: f.detail, kind: 'flag' })
    }
    return results.slice(0, MAX_RESULTS)
  }

  // Subcommand position: directly after the (sub)command, nothing unmatched yet.
  if (node?.sub && unmatchedArgs === 0) {
    for (const name of prefixFilter(Object.keys(node.sub), current)) {
      results.push({
        insert: name,
        label: name,
        detail: node.sub[name].detail,
        kind: 'subcommand'
      })
    }
  }

  const argKind: ArgKind | undefined = node ? node.arg : 'path'
  if (argKind === 'branch') {
    const branches = await safeCall(() => sources.listBranches(), [] as string[])
    for (const b of prefixFilter(branches, current)) {
      results.push({ insert: b, label: b, kind: 'branch' })
    }
  } else if (argKind === 'script') {
    const scripts = await safeCall(() => sources.listScripts(), [] as string[])
    for (const s of prefixFilter(scripts, current)) {
      results.push({ insert: s, label: s, detail: 'package script', kind: 'script' })
    }
  } else if (argKind === 'path' || argKind === 'dir') {
    results.push(...(await completePaths(current, sources, argKind === 'dir')))
  } else if (!argKind && (current.includes('/') || current.startsWith('.'))) {
    // No outline guidance but the token is path-shaped.
    results.push(...(await completePaths(current, sources, false)))
  }

  return results.slice(0, MAX_RESULTS)
}

function prefixFilterFlags(flags: FlagDef[], prefix: string): FlagDef[] {
  return flags.filter((f) => f.flag.startsWith(prefix) && f.flag !== prefix)
}

// --- Default sources over window.api, with small renderer-side caches ---

let executablesCache: { at: number; promise: Promise<string[]> } | null = null
const dirCache = new Map<string, { at: number; promise: Promise<DirEntry[]> }>()
const branchCache = new Map<string, { at: number; promise: Promise<string[]> }>()
const scriptCache = new Map<string, { at: number; promise: Promise<string[]> }>()

const EXECUTABLES_TTL = 5 * 60_000
const DIR_TTL = 5_000
const BRANCH_TTL = 10_000
const SCRIPT_TTL = 30_000

export function resetCompletionCaches(): void {
  executablesCache = null
  dirCache.clear()
  branchCache.clear()
  scriptCache.clear()
}

function cached<T>(
  map: Map<string, { at: number; promise: Promise<T> }>,
  key: string,
  ttl: number,
  load: () => Promise<T>
): Promise<T> {
  const hit = map.get(key)
  if (hit && Date.now() - hit.at < ttl) return hit.promise
  const promise = load()
  map.set(key, { at: Date.now(), promise })
  return promise
}

export function defaultSources(cwd: string | null, repoPath: string | null): CompletionSources {
  return {
    cwd,
    listDir: (dir) =>
      cached(dirCache, dir, DIR_TTL, async () => {
        const entries = await window.api.listDir(dir)
        return entries.map((e) => ({ name: e.name, isDirectory: e.isDirectory }))
      }),
    listBranches: () => {
      if (!repoPath) return Promise.resolve([])
      return cached(branchCache, repoPath, BRANCH_TTL, async () => {
        const result = await window.api.listBranches(repoPath)
        return result.local ?? []
      })
    },
    listExecutables: () => {
      if (executablesCache && Date.now() - executablesCache.at < EXECUTABLES_TTL) {
        return executablesCache.promise
      }
      const promise = window.api.listShellExecutables()
      executablesCache = { at: Date.now(), promise }
      return promise
    },
    outlines: generatedOutlineSource(),
    listScripts: () => {
      if (!cwd) return Promise.resolve([])
      return cached(scriptCache, cwd, SCRIPT_TTL, async () => {
        const content = await window.api.readFileContent(`${cwd}/package.json`, 64 * 1024)
        if (!content) return []
        try {
          const scripts = JSON.parse(content).scripts
          return scripts ? Object.keys(scripts) : []
        } catch {
          return []
        }
      })
    }
  }
}
