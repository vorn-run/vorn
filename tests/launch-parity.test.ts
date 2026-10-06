/**
 * The launch builders, in TypeScript and in `vorn_agents::launch`, against
 * one shared corpus (`fixtures/launch-lines.json`) and against each other.
 *
 * The TypeScript is checked against the corpus here, and the Rust in
 * `crates/agents/tests/launch_corpus.rs`; with `vorn_core.node` built, the
 * test-only exports put every corpus case and a run of seeded random lines
 * through both and expect the same answer, errors included. The random lines
 * are what pins the refusal set: a line either side declines to read must be
 * declined by the other.
 *
 * `VORN_WRITE_LAUNCH_CORPUS=1` rewrites the corpus's expectations from the
 * TypeScript, for when the TypeScript changes on purpose.
 */
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { tokenize, stripSessionSelectors } from '../packages/server/src/launch-tokens'
import { resumeCwdFor } from '../packages/server/src/resume-cwd'
import { filterEnv, normalizePassthrough } from '../packages/server/src/process-utils'
import { loadNativeCore } from '../packages/server/src/native-core'
import { displayNameFromPrompt } from '@vornrun/shared/string-utils'
import {
  CORPUS_PATH,
  fill,
  launchEnv,
  makeBin,
  readCorpus,
  seeded,
  viaNative,
  viaTypeScript,
  type LaunchCase,
  type NativeLaunch,
  type ShellCase
} from './helpers/launch-parity'

const builtCore = path.resolve(__dirname, '../packages/core/vorn_core.node')
const loaded = fs.existsSync(builtCore)
  ? (loadNativeCore([builtCore]) as unknown as Partial<NativeLaunch>)
  : null
const native = typeof loaded?.launchLine === 'function' ? (loaded as NativeLaunch) : null
const writing = process.env.VORN_WRITE_LAUNCH_CORPUS === '1'

const corpus = readCorpus()
const made: string[] = []
let empty = ''
let tmp = ''
let savedTmp: string | undefined

type ShellIntegration = typeof import('../packages/server/src/shell-integration')
type Shim = typeof import('../packages/server/src/shell-integration/shim')
let shells: ShellIntegration
let shimRoot = ''

beforeAll(async () => {
  empty = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-launch-empty-'))
  // The server writes its shims under os.tmpdir(), fixed when the module
  // loads; pointed at a directory of this test's own before it does.
  tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-launch-tmp-'))
  savedTmp = process.env.TMPDIR
  process.env.TMPDIR = tmp
  shells = await import('../packages/server/src/shell-integration')
  shimRoot = ((await import('../packages/server/src/shell-integration/shim')) as Shim).SHIM_ROOT
  if (savedTmp === undefined) delete process.env.TMPDIR
  else process.env.TMPDIR = savedTmp
})

afterAll(() => {
  for (const dir of [empty, tmp, ...made]) fs.rmSync(dir, { recursive: true, force: true })
  if (writing) fs.writeFileSync(CORPUS_PATH, JSON.stringify(corpus, null, 2) + '\n')
})

/** The case with `{bin}` made and filled in, and what to put back when writing. */
function prepare(c: LaunchCase): { filled: LaunchCase; vars: Record<string, string> } {
  const bin = makeBin(c.bin ?? [])
  made.push(bin)
  const vars = { bin }
  return { filled: fill({ ...c, env: launchEnv(c) }, vars), vars }
}

/** `value` with each of `vars`' values put back as its `{name}`. */
function unfill<T>(value: T, vars: Record<string, string>): T {
  let text = JSON.stringify(value)
  for (const [name, at] of Object.entries(vars)) {
    text = text.split(JSON.stringify(at).slice(1, -1)).join(`{${name}}`)
  }
  return JSON.parse(text) as T
}

describe('the TypeScript against the corpus', () => {
  it.each(corpus.launch.map((c) => [c.name, c] as const))('launches %s', (_name, c) => {
    const { filled, vars } = prepare(c)
    const got = viaTypeScript(filled, empty)
    if (writing) {
      c.line = unfill(got.line, vars)
      c.headless = unfill(got.headless, vars)
    }
    expect(got.line).toEqual(fill(c.line, vars))
    expect(got.headless).toEqual(fill(c.headless, vars))
  })

  it('reads lines', () => {
    for (const t of corpus.tokens) {
      const got = tokenize(t.line)?.map(({ raw, value }) => ({ raw, value })) ?? null
      if (writing) t.tokens = got
      expect({ line: t.line, tokens: got }).toEqual({ line: t.line, tokens: t.tokens })
    }
  })

  it('strips selectors', () => {
    for (const s of corpus.strip) {
      const got = stripSessionSelectors(s.line, s.agent, s.command.length)
      if (writing) s.expected = got
      expect({ line: s.line, stripped: got }).toEqual({ line: s.line, stripped: s.expected })
    }
  })

  it('names sessions from prompts', () => {
    for (const d of corpus.displayNames) {
      const got = displayNameFromPrompt(d.prompt, d.maxLen) ?? null
      if (writing) d.name = got
      expect({ prompt: d.prompt, name: got }).toEqual({ prompt: d.prompt, name: d.name })
    }
  })

  it('finds where a session resumes', () => {
    for (const r of corpus.resumeCwd) {
      const got = resumeCwdFor(r.session, (at) => r.dirs.includes(at))
      if (writing) r.result = got
      expect(got).toEqual(r.result)
    }
  })

  it('filters environments', () => {
    for (const e of corpus.env) {
      const source = Object.fromEntries(e.source)
      const safe = Object.entries(filterEnv(source, new Set()))
      const launch = filterEnv(source, normalizePassthrough(e.passthrough))
      if (e.dataDir) launch.VORN_DATA_DIR = e.dataDir
      if (writing) {
        e.safe = safe
        e.launch = Object.entries(launch)
      }
      expect(safe).toEqual(e.safe)
      expect(Object.entries(launch)).toEqual(e.launch)
    }
  })
})

/** Runs `fn` with the process environment the shell integration reads set as the case says. */
function inShellEnv<T>(s: ShellCase, fn: () => T): T {
  const keys = ['ZDOTDIR', 'XDG_DATA_DIRS', 'PROMPT', 'HOME'] as const
  const saved = Object.fromEntries(keys.map((k) => [k, process.env[k]]))
  for (const k of keys) delete process.env[k]
  Object.assign(process.env, s.env, { HOME: s.home })
  try {
    return fn()
  } finally {
    for (const k of keys) {
      if (saved[k] === undefined) delete process.env[k]
      else process.env[k] = saved[k]
    }
  }
}

describe('shell integration', () => {
  it.each(
    corpus.shell.map(
      (s, i) => [`${s.shell} ${s.minimalPrompt ? 'minimal' : 'own prompt'} ${i}`, s] as const
    )
  )('sets up %s', (_name, s) => {
    // Written afresh each time, so every version of a shim is the one on disk once.
    shells.resetShellIntegrationCache()
    const got = inShellEnv(s, () =>
      shells.getShellIntegration({ shell: s.shell, minimalPrompt: s.minimalPrompt })
    )
    const vars = { shims: shimRoot }
    if (writing) s.setup = unfill(got, vars)
    expect(got).toEqual(fill(s.setup, vars))
    if (!native) return
    const fromCore = native.shellSetup(s.shell, s.minimalPrompt, s.env, s.home, shimRoot)
    expect({ env: fromCore.env, args: fromCore.args ?? null }).toEqual(got)
  })
})

describe.skipIf(!native)('vorn_core.node against the TypeScript', () => {
  it.each(corpus.launch.map((c) => [c.name, c] as const))('launches %s', (_name, c) => {
    const { filled } = prepare(c)
    expect(viaNative(native!, filled)).toEqual(viaTypeScript(filled, empty))
  })

  it('reads the corpus lines', () => {
    for (const t of corpus.tokens) {
      expect({ line: t.line, tokens: native!.launchTokens(t.line) }).toEqual({
        line: t.line,
        tokens: t.tokens
      })
    }
  })

  const FRAGMENTS = [
    'claude',
    'codex',
    'npx',
    '--resume',
    '--resume=',
    '-r',
    '-rold',
    '-ir',
    '--continue',
    '-c',
    '--session-id',
    '--session',
    '-s',
    'resume',
    '--last',
    '--',
    '--model',
    '-m',
    '-mx',
    'model=x',
    'x',
    'old',
    "'a b'",
    "'",
    '"',
    '"q $x"',
    '"a\\"b"',
    '\\',
    '\\ ',
    '$(',
    '$',
    ')',
    '(',
    '`',
    '|',
    '&&',
    ';',
    '>',
    '<',
    '\n',
    '\r',
    '\t',
    'é',
    '😀',
    '\u00a0',
    '$HOME',
    '~/bin'
  ]
  const SEPARATORS = [' ', ' ', ' ', '', '\t', '  ']
  const AGENTS = ['claude', 'copilot', 'codex', 'opencode', 'gemini'] as const

  it('agrees on seeded random command lines, refusals included', () => {
    const rand = seeded(0x5eed1)
    const pick = <T>(list: readonly T[]): T => list[Math.floor(rand() * list.length)]!
    const maybe = <T>(list: readonly T[]): T | undefined => (rand() < 0.5 ? undefined : pick(list))
    for (let i = 0; i < 4000; i++) {
      let line = ''
      const words = 1 + Math.floor(rand() * 7)
      for (let w = 0; w < words; w++) line += (w ? pick(SEPARATORS) : '') + pick(FRAGMENTS)
      if (rand() < 0.1) line += pick(SEPARATORS)
      const agentType = pick(AGENTS)
      const c: LaunchCase = {
        name: `random ${i}`,
        platform: rand() < 0.7 ? 'posix' : 'win32',
        shell: pick(['cmd.exe', 'C:\\pwsh.exe']),
        payload: {
          agentType,
          resumeSessionId: maybe(['new', '', 'a b', "it's"]),
          sessionId: maybe(['pin', '', '50%']),
          model: rand() < 0.15 ? pick(['m', '-bad', 'a b']) : undefined,
          initialPrompt: maybe(['p', '', 'it\'s "x"', 'a\nb']),
          remoteHostId: rand() < 0.1 ? 'host' : undefined,
          args: rand() < 0.3 ? [pick(FRAGMENTS), pick(FRAGMENTS)] : undefined
        },
        commands: { [agentType]: { command: line, args: rand() < 0.5 ? [] : [pick(FRAGMENTS)] } }
      }
      const reference = viaTypeScript(c, empty)
      // The case rides along, so a failure shows what to replay.
      expect({ c, ...viaNative(native!, c) }).toEqual({ c, ...reference })
      const expected = tokenize(line)?.map(({ raw, value }) => ({ raw, value })) ?? null
      expect({ line, tokens: native!.launchTokens(line) }).toEqual({ line, tokens: expected })
    }
  })
})
