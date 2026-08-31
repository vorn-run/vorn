import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest'
import {
  claimSpawningTranscript,
  releaseSpawningTranscript,
  spawningTranscripts,
  resetTranscriptClaims
} from '../packages/server/src/transcript-claims'

/**
 * The window where a session has started but cannot yet say what it is writing.
 *
 * codex and opencode take no id at launch, so `agentSessionId` only arrives from
 * the capture seconds later. Resuming two cold panes in that gap is the case
 * this covers: the first spawn holds its transcript, so the second resolves to a
 * different one instead of starting a second agent on one conversation.
 */

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
    vi.advanceTimersByTime(15_001)
    expect(spawningTranscripts()).toEqual(new Set())
    expect(claimSpawningTranscript('transcript-a', 'term-2')).toBeUndefined()
  })

  it('still holds inside the window', () => {
    vi.useFakeTimers()
    claimSpawningTranscript('transcript-a', 'term-1')
    vi.advanceTimersByTime(14_000)
    expect(claimSpawningTranscript('transcript-a', 'term-2')).toBe('term-1')
  })
})
