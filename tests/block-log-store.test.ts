import { describe, it, expect, beforeEach } from 'vitest'
import { captureBlock, clearBlockLog, getBlockLog } from '../src/renderer/lib/block-log'

/**
 * The log backs a useSyncExternalStore subscription, which compares snapshots
 * by identity. A snapshot that is rebuilt on every read loops forever; one that
 * is mutated in place never re-renders. Both failure modes are silent, so they
 * are pinned here.
 */

const BUFFER = { length: 1, getLine: () => undefined } as never

function add(terminalId: string, command: string): void {
  captureBlock({
    terminalId,
    buffer: BUFFER,
    startLine: 0,
    endLine: 0,
    command,
    exitCode: 0,
    durationMs: 1,
    cwd: null
  })
}

beforeEach(() => {
  clearBlockLog('t')
})

describe('block log snapshots', () => {
  it('returns the same reference while nothing changes', () => {
    add('t', 'ls')
    expect(getBlockLog('t')).toBe(getBlockLog('t'))
  })

  it('returns the same reference for a log that does not exist', () => {
    expect(getBlockLog('never-seen')).toBe(getBlockLog('also-never-seen'))
  })

  it('changes identity when a command is captured', () => {
    const before = getBlockLog('t')
    add('t', 'ls')
    expect(getBlockLog('t')).not.toBe(before)
  })

  it('leaves an earlier snapshot untouched', () => {
    add('t', 'first')
    const snapshot = getBlockLog('t')
    add('t', 'second')
    expect(snapshot).toHaveLength(1)
  })
})

describe('block log bound', () => {
  it('keeps the most recent commands and drops the oldest', () => {
    for (let i = 0; i < 105; i++) add('t', `cmd-${i}`)
    const log = getBlockLog('t')
    expect(log).toHaveLength(100)
    expect(log[0].command).toBe('cmd-5')
    expect(log[log.length - 1].command).toBe('cmd-104')
  })
})
