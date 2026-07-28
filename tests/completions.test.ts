import { describe, it, expect } from 'vitest'
import {
  getCompletions,
  type Completion,
  type CompletionSources,
  type DirEntry
} from '../src/renderer/lib/completions'

function makeSources(overrides: Partial<CompletionSources> = {}): CompletionSources {
  return {
    cwd: '/repo',
    listDir: async (dir: string): Promise<DirEntry[]> => {
      if (dir === '/repo') {
        return [
          { name: 'src', isDirectory: true },
          { name: 'docs', isDirectory: true },
          { name: 'package.json', isDirectory: false },
          { name: 'README.md', isDirectory: false }
        ]
      }
      if (dir === '/repo/src/') {
        return [
          { name: 'main.ts', isDirectory: false },
          { name: 'lib', isDirectory: true }
        ]
      }
      return []
    },
    listBranches: async () => ['main', 'feat/terminal-command-bar', 'fix/build'],
    listExecutables: async () => ['git', 'gh', 'grep', 'node', 'npx', 'cargo'],
    listScripts: async () => ['dev', 'build', 'test', 'typecheck'],
    ...overrides
  }
}

function labels(cs: Completion[]): string[] {
  return cs.map((c) => c.label)
}

describe('getCompletions', () => {
  it('completes command names by prefix, outlined commands first', async () => {
    const result = await getCompletions('g', makeSources())
    expect(labels(result)).toEqual(['git', 'grep', 'gh'])
    expect(result[0].kind).toBe('command')
    expect(result[0].detail).toBe('version control')
  })

  it('returns nothing for an empty command position', async () => {
    expect(await getCompletions('', makeSources())).toEqual([])
    expect(await getCompletions('   ', makeSources())).toEqual([])
  })

  it('completes git subcommands with descriptions', async () => {
    const result = await getCompletions('git ch', makeSources())
    expect(labels(result)).toEqual(['checkout', 'cherry-pick'])
    expect(result[0].kind).toBe('subcommand')
    expect(result[0].detail).toBe('switch branch or restore')
  })

  it('completes branches after git checkout', async () => {
    const result = await getCompletions('git checkout f', makeSources())
    expect(labels(result)).toEqual(['feat/terminal-command-bar', 'fix/build'])
    expect(result[0].kind).toBe('branch')
  })

  it('lists all branches after git checkout with trailing space', async () => {
    const result = await getCompletions('git checkout ', makeSources())
    expect(labels(result)).toEqual(['main', 'feat/terminal-command-bar', 'fix/build'])
  })

  it('completes flags when the token starts with a dash', async () => {
    const result = await getCompletions('git commit --a', makeSources())
    expect(labels(result)).toEqual(['--amend'])
    expect(result[0].kind).toBe('flag')
    expect(result[0].detail).toBe('rewrite last commit')
  })

  it('completes nested subcommands (git stash)', async () => {
    const result = await getCompletions('git stash p', makeSources())
    expect(labels(result)).toEqual(['push', 'pop'])
  })

  it('completes paths for path-arg commands, directories first with slash', async () => {
    const result = await getCompletions('cat ', makeSources())
    expect(labels(result)).toEqual(['docs/', 'src/', 'package.json', 'README.md'])
    expect(result[0].continues).toBe(true)
    expect(result[0].insert).toBe('docs/')
  })

  it('drills into subdirectories keeping the dir prefix in the insert', async () => {
    const result = await getCompletions('cat src/m', makeSources())
    expect(labels(result)).toEqual(['main.ts'])
    expect(result[0].insert).toBe('src/main.ts')
  })

  it('cd offers only directories', async () => {
    const result = await getCompletions('cd ', makeSources())
    expect(labels(result)).toEqual(['docs/', 'src/'])
  })

  it('unknown commands fall back to path completion', async () => {
    const result = await getCompletions('mytool sr', makeSources())
    expect(labels(result)).toEqual(['src/'])
  })

  it('completes package scripts for yarn', async () => {
    const result = await getCompletions('yarn t', makeSources())
    expect(labels(result)).toEqual(['test', 'typecheck'])
    expect(result[0].kind).toBe('script')
  })

  it('yarn run also completes scripts', async () => {
    const result = await getCompletions('yarn run de', makeSources())
    expect(labels(result)).toEqual(['dev'])
  })

  it('escapes shell-significant characters in path inserts', async () => {
    const sources = makeSources({
      listDir: async () => [{ name: 'My Docs', isDirectory: true }]
    })
    const result = await getCompletions('cd M', sources)
    expect(result[0].insert).toBe('My\\ Docs/')
    expect(result[0].label).toBe('My Docs/')
  })

  it('only the last line of multiline input is completed', async () => {
    const result = await getCompletions('echo hi\ngit ch', makeSources())
    expect(labels(result)).toEqual(['checkout', 'cherry-pick'])
  })

  it('excludes an exact match from suggestions', async () => {
    const result = await getCompletions('git checkout main', makeSources())
    expect(labels(result)).not.toContain('main')
  })

  it('survives failing sources', async () => {
    const sources = makeSources({
      listBranches: () => Promise.reject(new Error('no repo')),
      listDir: () => {
        throw new Error('sync failure')
      }
    })
    expect(await getCompletions('git checkout m', sources)).toEqual([])
    expect(await getCompletions('cat sr', sources)).toEqual([])
  })

  it('returns nothing without a cwd for relative paths', async () => {
    const result = await getCompletions('cat sr', makeSources({ cwd: null }))
    expect(result).toEqual([])
  })
})
