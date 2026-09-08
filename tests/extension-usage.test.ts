import { afterEach, describe, expect, it } from 'vitest'
import { mkdirSync, mkdtempSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { TerminalSession } from '@vornrun/shared/types'
import { usageFor } from '../packages/server/src/extensions/usage'

const temps: string[] = []

function home(): string {
  const dir = mkdtempSync(join(tmpdir(), 'vorn-usage-home-'))
  temps.push(dir)
  return dir
}

afterEach(() => {
  while (temps.length > 0) rmSync(temps.pop() as string, { recursive: true, force: true })
})

function session(over: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id: 's1',
    agentType: 'claude',
    projectName: 'vorn',
    projectPath: '/Users/someone/dev/vorn',
    status: 'running',
    createdAt: 0,
    pid: 1,
    agentSessionId: 'abc-123',
    ...over
  } as TerminalSession
}

/** A conversation is filed under its working directory, flattened. */
function writeTranscript(root: string, cwd: string, id: string, lines: unknown[]): void {
  const dir = join(root, '.claude', 'projects', cwd.replace(/[^A-Za-z0-9]/g, '-'))
  mkdirSync(dir, { recursive: true })
  writeFileSync(join(dir, `${id}.jsonl`), lines.map((line) => JSON.stringify(line)).join('\n'))
}

const turn = (usage: Record<string, number>): unknown => ({ type: 'assistant', message: { usage } })

describe('what a session has spent', () => {
  it('counts the context and the cache from the newest turn', () => {
    const root = home()
    writeTranscript(root, '/Users/someone/dev/vorn', 'abc-123', [
      { type: 'user', message: { content: 'hello' } },
      turn({ input_tokens: 10, cache_creation_input_tokens: 10, cache_read_input_tokens: 80 }),
      turn({ input_tokens: 2, cache_creation_input_tokens: 98, cache_read_input_tokens: 900 })
    ])
    expect(usageFor(session(), root)).toEqual({ contextTokens: 1000, cacheHitRate: 0.9 })
  })

  it('reads the worktree conversation before the project one', () => {
    const root = home()
    writeTranscript(root, '/Users/someone/dev/vorn', 'abc-123', [
      turn({ input_tokens: 1, cache_read_input_tokens: 9 })
    ])
    writeTranscript(root, '/Users/someone/work/tree', 'abc-123', [
      turn({ input_tokens: 5, cache_read_input_tokens: 95 })
    ])
    const reading = usageFor(session({ worktreePath: '/Users/someone/work/tree' }), root)
    expect(reading.contextTokens).toBe(100)
  })

  // Every unknown is left out rather than guessed, so a footer showing one shows something true.
  it('says nothing about an agent that publishes nothing', () => {
    const root = home()
    expect(usageFor(session({ agentSessionId: undefined }), root)).toEqual({})
    expect(usageFor(session({ agentType: 'shell', agentSessionId: 'abc-123' }), root)).toEqual({})
  })

  it('says nothing when there is no conversation on disk', () => {
    expect(usageFor(session(), home())).toEqual({})
  })

  // The newest turn is at the end, so a long conversation is read from the end.
  it('reads the newest turn without reading the whole conversation', () => {
    const root = home()
    const dir = join(root, '.claude', 'projects', '-Users-someone-dev-vorn')
    mkdirSync(dir, { recursive: true })
    const filler = JSON.stringify({ type: 'user', message: { content: 'x'.repeat(4000) } })
    const lines = Array.from({ length: 200 }, () => filler)
    lines.push(JSON.stringify(turn({ input_tokens: 5, cache_read_input_tokens: 95 })))
    const path = join(dir, 'abc-123.jsonl')
    writeFileSync(path, lines.join('\n'))
    expect(statSync(path).size).toBeGreaterThan(600 * 1024)
    expect(usageFor(session(), root)).toEqual({ contextTokens: 100, cacheHitRate: 0.95 })
  })

  // A conversation's id names a file, so an id that is a path names nothing here.
  it('reads nothing for an id that is not a name', () => {
    const root = home()
    writeTranscript(root, '/Users/someone/dev/vorn', 'abc-123', [
      turn({ input_tokens: 1, cache_read_input_tokens: 99 })
    ])
    expect(usageFor(session({ agentSessionId: '../../../etc/passwd' }), root)).toEqual({})
    expect(usageFor(session({ agentSessionId: 'a/b' }), root)).toEqual({})
  })

  it('skips a half-written line rather than failing on it', () => {
    const root = home()
    const dir = join(root, '.claude', 'projects', '-Users-someone-dev-vorn')
    mkdirSync(dir, { recursive: true })
    writeFileSync(
      join(dir, 'abc-123.jsonl'),
      `${JSON.stringify(turn({ input_tokens: 4, cache_read_input_tokens: 96 }))}\n{"type":"assis`
    )
    expect(usageFor(session(), root)).toEqual({ contextTokens: 100, cacheHitRate: 0.96 })
  })
})
