import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest'
import {
  claimSpawningTranscript,
  releaseSpawningTranscript,
  releaseSpawningTranscriptsFor,
  spawningTranscripts,
  resetTranscriptClaims
} from '../packages/server/src/transcript-claims'

beforeEach(() => resetTranscriptClaims())
afterEach(() => vi.useRealTimers())

describe('a transcript being started', () => {
  it('is granted once, and names the holder to whoever asks next', () => {
    expect(claimSpawningTranscript('transcript-a', 'term-1')).toBeUndefined()
    expect(claimSpawningTranscript('transcript-a', 'term-2')).toBe('term-1')
  })

  it('is counted as held while it starts, so a resolve skips it', () => {
    claimSpawningTranscript('transcript-a', 'term-1')
    expect(spawningTranscripts()).toEqual(new Set(['transcript-a']))
  })

  it('leaves other transcripts alone', () => {
    claimSpawningTranscript('transcript-a', 'term-1')
    expect(claimSpawningTranscript('transcript-b', 'term-2')).toBeUndefined()
  })

  it('is free again once the spawn that took it releases', () => {
    claimSpawningTranscript('transcript-a', 'term-1')
    releaseSpawningTranscript('transcript-a', 'term-1')
    expect(claimSpawningTranscript('transcript-a', 'term-2')).toBeUndefined()
  })

  it('ignores a release from a session that does not hold it', () => {
    claimSpawningTranscript('transcript-a', 'term-1')
    releaseSpawningTranscript('transcript-a', 'term-2')
    expect(claimSpawningTranscript('transcript-a', 'term-3')).toBe('term-1')
  })

  it('lapses rather than locks, so a spawn that died never wedges it', () => {
    vi.useFakeTimers()
    claimSpawningTranscript('transcript-a', 'term-1')
    vi.advanceTimersByTime(60_001)
    expect(spawningTranscripts()).toEqual(new Set())
    expect(claimSpawningTranscript('transcript-a', 'term-2')).toBeUndefined()
  })

  it('outlasts the capture that asks an agent which conversation it took', () => {
    vi.useFakeTimers()
    claimSpawningTranscript('transcript-a', 'term-1')
    // The last attempt lands at forty seconds; a claim gone by then is the bug.
    vi.advanceTimersByTime(40_000)
    expect(claimSpawningTranscript('transcript-a', 'term-2')).toBe('term-1')
  })

  it('lets go of everything one session was holding', () => {
    claimSpawningTranscript('transcript-a', 'term-1')
    claimSpawningTranscript('transcript-b', 'term-2')
    releaseSpawningTranscriptsFor('term-1')
    expect(spawningTranscripts()).toEqual(new Set(['transcript-b']))
  })

  it('lets go by session, so a resume given an alternate conversation still clears', () => {
    // A pane pinned to `transcript-a` whose resume resolved `transcript-b`
    // instead: the release names the session, never the id it hoped for.
    claimSpawningTranscript('transcript-b', 'term-1')
    releaseSpawningTranscriptsFor('term-1')
    expect(spawningTranscripts()).toEqual(new Set())
    expect(claimSpawningTranscript('transcript-b', 'term-2')).toBeUndefined()
  })

  it('ignores a release naming a conversation the session never took', () => {
    claimSpawningTranscript('transcript-b', 'term-1')
    releaseSpawningTranscript('transcript-a', 'term-1')
    expect(spawningTranscripts()).toEqual(new Set(['transcript-b']))
  })
})
