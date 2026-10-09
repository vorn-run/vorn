/**
 * The launch builders in `vorn_agents::launch`, through `vorn_core.node`,
 * against what the TypeScript they replaced gave: the shared corpus
 * (`fixtures/launch-lines.json`) and a run of seeded random command lines
 * (`fixtures/js-reference/launch-random.json`), errors included. The random
 * lines are what pins the refusal set: a line the TypeScript declined to
 * read must be declined still. The Rust is checked against the same corpus
 * in `crates/agents/tests/launch_corpus.rs`.
 */
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { loadNativeCore } from '../packages/server/src/native-core'
import { JsReference } from './helpers/js-reference'
import {
  fill,
  launchEnv,
  makeBin,
  randomCases,
  readCorpus,
  viaNative,
  type LaunchCase,
  type NativeLaunch
} from './helpers/launch-parity'

const builtCore = path.resolve(__dirname, '../packages/core/vorn_core.node')
const loaded = fs.existsSync(builtCore)
  ? (loadNativeCore([builtCore]) as unknown as Partial<NativeLaunch>)
  : null
const native = typeof loaded?.launchLine === 'function' ? (loaded as NativeLaunch) : null

const corpus = readCorpus()
const made: string[] = []
let empty = ''
let shimRoot = ''

beforeAll(() => {
  empty = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-launch-empty-'))
  shimRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-launch-shims-'))
})

afterAll(() => {
  for (const dir of [empty, shimRoot, ...made]) fs.rmSync(dir, { recursive: true, force: true })
})

/** The case with `{bin}` made and filled in, and the values to fill its expectations with. */
function prepare(c: LaunchCase): { filled: LaunchCase; vars: Record<string, string> } {
  const bin = makeBin(c.bin ?? [])
  made.push(bin)
  const vars = { bin }
  return { filled: fill({ ...c, env: launchEnv(c) }, vars), vars }
}

describe.skipIf(!native)('vorn_core.node against the corpus', () => {
  it.each(corpus.launch.map((c) => [c.name, c] as const))('launches %s', (_name, c) => {
    const { filled, vars } = prepare(c)
    expect(viaNative(native!, filled)).toEqual(fill({ line: c.line, headless: c.headless }, vars))
  })

  it('reads the corpus lines', () => {
    for (const t of corpus.tokens) {
      expect({ line: t.line, tokens: native!.launchTokens(t.line) }).toEqual({
        line: t.line,
        tokens: t.tokens
      })
    }
  })

  it.each(
    corpus.shell.map(
      (s, i) => [`${s.shell} ${s.minimalPrompt ? 'minimal' : 'own prompt'} ${i}`, s] as const
    )
  )('sets up %s', (_name, s) => {
    const got = native!.shellSetup(s.shell, s.minimalPrompt, s.env, s.home, shimRoot)
    expect({ env: got.env, args: got.args ?? null }).toEqual(fill(s.setup, { shims: shimRoot }))
  })

  it('agrees on seeded random command lines, refusals included', async () => {
    const reference = new JsReference('launch-random')
    for (const { c, line } of randomCases(4000)) {
      const want = fill(
        await reference.want<Record<string, unknown>>(c.name, () => {
          throw new Error('the TypeScript launch builders are gone; nothing to record')
        }),
        {}
      )
      const { tokens, ...launched } = want
      // The case rides along, so a failure shows what to replay.
      expect({ c, ...viaNative(native!, c) }).toEqual({ c, ...launched })
      expect({ line, tokens: native!.launchTokens(line) }).toEqual({ line, tokens })
    }
  })
})
