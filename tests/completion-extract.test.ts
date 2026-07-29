import { describe, it, expect, beforeEach } from 'vitest'
import {
  argKind,
  assertBudget,
  compactDetail,
  extractSpec,
  MAX_DETAIL,
  MAX_FLAGS,
  MAX_SUBCOMMANDS,
  resetTruncations,
  truncations
} from '../scripts/lib/completion-extract.mjs'

beforeEach(() => resetTruncations())

describe('compactDetail', () => {
  it('collapses whitespace', () => {
    expect(compactDetail('  record   changes \n to the repo ', 'commit')).toBe(
      'record changes to the repo'
    )
  })

  it('drops a description that only restates the name', () => {
    expect(compactDetail('Commit', 'commit')).toBeUndefined()
  })

  it('drops empty and non-string descriptions', () => {
    expect(compactDetail('', 'x')).toBeUndefined()
    expect(compactDetail(undefined, 'x')).toBeUndefined()
  })

  it('truncates long descriptions with an ellipsis', () => {
    const result = compactDetail('x'.repeat(200), 'cmd')
    expect(result).toHaveLength(MAX_DETAIL)
    expect(result?.endsWith('…')).toBe(true)
  })
})

describe('argKind', () => {
  it('maps filesystem templates', () => {
    expect(argKind({ template: 'filepaths' })).toBe('path')
    expect(argKind({ template: 'folders' })).toBe('dir')
    expect(argKind([{ template: ['folders', 'filepaths'] }])).toBe('dir')
  })

  it('treats everything else as no argument', () => {
    // Branch and script arguments come from the live session, which knows
    // the actual repository; the corpus only guesses.
    expect(argKind(undefined)).toBe('none')
    expect(argKind({ name: 'branch', generators: {} })).toBe('none')
  })
})

describe('extractSpec', () => {
  it('reduces a spec to names, descriptions, flags and subcommands', () => {
    const result = extractSpec({
      name: 'demo',
      description: 'A demo tool',
      subcommands: [
        {
          name: 'build',
          description: 'Build the thing',
          args: { template: 'filepaths' },
          options: [{ name: ['-r', '--release'], description: 'Release mode' }]
        }
      ]
    })
    expect(result?.name).toBe('demo')
    expect(result?.outline.detail).toBe('A demo tool')
    expect(result?.outline.sub?.build).toMatchObject({
      detail: 'Build the thing',
      arg: 'path',
      flags: [
        { flag: '-r', detail: 'Release mode' },
        { flag: '--release', detail: 'Release mode' }
      ]
    })
  })

  it('takes the first of aliased subcommand names', () => {
    const result = extractSpec({
      name: ['demo', 'dm'],
      description: 'd',
      subcommands: [{ name: ['remove', 'rm'], description: 'Remove it' }]
    })
    expect(result?.name).toBe('demo')
    expect(Object.keys(result?.outline.sub ?? {})).toEqual(['remove'])
  })

  it('skips hidden and deprecated entries', () => {
    const result = extractSpec({
      name: 'demo',
      description: 'd',
      subcommands: [
        { name: 'keep', description: 'Keep' },
        { name: 'gone', description: 'Gone', hidden: true },
        { name: 'old', description: 'Old', deprecated: true }
      ]
    })
    expect(Object.keys(result?.outline.sub ?? {})).toEqual(['keep'])
  })

  it('ignores non-flag option names', () => {
    const result = extractSpec({
      name: 'demo',
      description: 'd',
      options: [{ name: 'notaflag' }, { name: '--real' }]
    })
    expect(result?.outline.flags).toEqual([{ flag: '--real', detail: undefined }])
  })

  it('caps flags per node', () => {
    const options = Array.from({ length: 40 }, (_, i) => ({ name: `--flag${i}` }))
    const result = extractSpec({ name: 'demo', description: 'd', options })
    expect(result?.outline.flags).toHaveLength(MAX_FLAGS)
  })

  it('caps subcommands and reports what it dropped', () => {
    const subcommands = Array.from({ length: MAX_SUBCOMMANDS + 7 }, (_, i) => ({
      name: `sub${i}`,
      description: `Sub ${i}`
    }))
    extractSpec({ name: 'huge', description: 'd', subcommands })
    expect(truncations).toEqual(['huge: dropped 7 subcommands'])
  })

  it('stops nesting at the depth limit', () => {
    const result = extractSpec({
      name: 'demo',
      description: 'd',
      subcommands: [
        {
          name: 'a',
          subcommands: [{ name: 'b', subcommands: [{ name: 'c', description: 'too deep' }] }]
        }
      ]
    })
    expect(result?.outline.sub?.a.sub?.b).toBeDefined()
    expect(result?.outline.sub?.a.sub?.b.sub).toBeUndefined()
  })

  it('rejects specs with nothing static worth indexing', () => {
    // A bare name adds nothing the PATH scan does not already provide.
    expect(extractSpec({ name: 'bare' })).toBeNull()
    expect(extractSpec({})).toBeNull()
    expect(extractSpec(null)).toBeNull()
  })
})

describe('assertBudget', () => {
  it('passes under the limit and throws over it', () => {
    expect(() => assertBudget('thing', 100, 200)).not.toThrow()
    expect(() => assertBudget('thing', 300, 200)).toThrow(/over the/)
  })
})
