import { describe, expect, it, vi } from 'vitest'
import { onePerKey } from '../packages/server/src/one-per-key'

describe('onePerKey', () => {
  it('hands a second caller the running call for its key, and runs nothing twice', async () => {
    const once = onePerKey<string>()
    let finish!: (value: string) => void
    const run = vi.fn(() => new Promise<string>((resolve) => (finish = resolve)))
    const first = once('conversation-a', run)
    const second = once('conversation-a', run)
    finish('session-1')
    expect(await first).toBe('session-1')
    expect(await second).toBe('session-1')
    expect(run).toHaveBeenCalledTimes(1)
  })

  it('keeps keys apart, and runs again once the last call has settled', async () => {
    const once = onePerKey<string>()
    expect(await once('a', async () => 'a1')).toBe('a1')
    expect(await once('b', async () => 'b1')).toBe('b1')
    expect(await once('a', async () => 'a2')).toBe('a2')
  })

  it('lets a failure go, so the next call tries afresh', async () => {
    const once = onePerKey<string>()
    await expect(
      once('a', async () => {
        throw new Error('worktree could not be made')
      })
    ).rejects.toThrow('worktree could not be made')
    expect(await once('a', async () => 'second try')).toBe('second try')
  })
})
