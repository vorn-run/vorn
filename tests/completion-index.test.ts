import { describe, it, expect } from 'vitest'
import { generatedOutlineSource, staticOutlineSource } from '../src/renderer/lib/completion-index'
import {
  getCompletions,
  type CompletionSources,
  type Outline
} from '../src/renderer/lib/completions'

function sources(over: Partial<CompletionSources> = {}): CompletionSources {
  return {
    cwd: '/tmp/vorn',
    listDir: async () => [],
    listBranches: async () => [],
    listExecutables: async () => [],
    listScripts: async () => [],
    ...over
  }
}

describe('generatedOutlineSource', () => {
  it('exposes every indexed command name', async () => {
    const names = await generatedOutlineSource().names()
    expect(names.size).toBeGreaterThan(50)
    // Indexed despite having no top-level description upstream — a command
    // with an outline must never be missing from name completion.
    expect(names.has('kubectl')).toBe(true)
    expect(names.get('kubectl')).toBeUndefined()
    expect(names.get('git')).toBeTruthy()
  })

  it('loads an outline on demand', async () => {
    const outline = await generatedOutlineSource().outline('kubectl')
    expect(outline?.sub).toBeDefined()
    expect(Object.keys(outline?.sub ?? {})).toContain('get')
  })

  it('resolves unknown commands to undefined rather than throwing', async () => {
    await expect(
      generatedOutlineSource().outline('definitely-not-a-command')
    ).resolves.toBeUndefined()
  })
})

describe('outline precedence', () => {
  it('prefers the hand-written outline wholesale over the generated one', async () => {
    // The curated outlines know that `git switch` takes a branch; generic
    // extraction cannot. Merging the two would make that unpredictable.
    const generated: Record<string, Outline> = {
      git: { detail: 'generated detail', sub: { bogus: { detail: 'not real' } } }
    }
    const results = await getCompletions(
      'git swi',
      sources({ outlines: staticOutlineSource(generated) })
    )
    expect(results.map((r) => r.insert)).toContain('switch')
    expect(results.map((r) => r.insert)).not.toContain('bogus')
  })

  it('falls back to the generated outline for commands with no curated entry', async () => {
    const generated: Record<string, Outline> = {
      madeupctl: { detail: 'test tool', sub: { deploy: { detail: 'ship it' } } }
    }
    const results = await getCompletions(
      'madeupctl dep',
      sources({ outlines: staticOutlineSource(generated) })
    )
    expect(results.map((r) => r.insert)).toEqual(['deploy'])
  })

  it('ranks curated names before generated ones, and both before bare PATH names', async () => {
    const generated: Record<string, Outline> = { gitleaks: { detail: 'scan for secrets' } }
    const results = await getCompletions(
      'git',
      sources({
        outlines: staticOutlineSource(generated),
        listExecutables: async () => ['gitk', 'gitleaks']
      })
    )
    const order = results.map((r) => r.insert)
    expect(order.indexOf('gitleaks')).toBeLessThan(order.indexOf('gitk'))
  })

  it('surfaces the generated description on a command suggestion', async () => {
    const generated: Record<string, Outline> = { madeupctl: { detail: 'test tool' } }
    const results = await getCompletions(
      'madeup',
      sources({ outlines: staticOutlineSource(generated) })
    )
    expect(results[0]).toMatchObject({ insert: 'madeupctl', detail: 'test tool' })
  })

  it('works with no outline source at all', async () => {
    // The field is optional so existing callers keep working untouched.
    const results = await getCompletions('git stat', sources())
    expect(results.map((r) => r.insert)).toContain('status')
  })
})
