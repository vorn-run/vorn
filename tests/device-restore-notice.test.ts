// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { DeviceRestoreRefusal } from '../src/renderer/stores/types'

const toast = vi.fn()
vi.mock('../src/renderer/components/Toast', () => ({ toast: (...a: unknown[]) => toast(...a) }))

const restore = vi.fn<() => Promise<DeviceRestoreRefusal[]>>()
vi.mock('../src/renderer/stores', () => ({
  useAppStore: { getState: () => ({ restoreDevicePanes: restore }) }
}))

const { restoreDevicePanes } = await import('../src/renderer/lib/device-restore')

const device = { udid: 'udid-1', name: 'iPhone 17 Pro' }

beforeEach(() => {
  toast.mockReset()
  restore.mockReset().mockResolvedValue([])
})

/** The one line of text this whole path exists to produce. */
async function noticeFor(failure: DeviceRestoreRefusal['failure']): Promise<string> {
  restore.mockResolvedValue([{ sessionId: 'term-1', device, failure }])
  await restoreDevicePanes()
  return toast.mock.calls[0][0] as string
}

describe('telling someone their simulator did not come back', () => {
  it('says nothing when every one of them did', async () => {
    await restoreDevicePanes()
    expect(toast).not.toHaveBeenCalled()
  })

  it('names the session holding it', async () => {
    const text = await noticeFor({ reason: 'held-by-session', holder: 'term-9', message: 'held' })
    expect(text).toContain('iPhone 17 Pro')
    expect(text).toContain('term-9')
  })

  it('names the other Vorn driving it', async () => {
    const text = await noticeFor({ reason: 'held-by-other-vorn', pid: 4242, message: 'held' })
    expect(text).toContain('iPhone 17 Pro')
    expect(text).toContain('4242')
  })

  it('passes on why it would not boot, which names a machine problem', async () => {
    const text = await noticeFor({ reason: 'boot-failed', message: 'Unable to boot device' })
    expect(text).toContain('iPhone 17 Pro')
    expect(text).toContain('Unable to boot device')
  })

  it('names the device rather than repeating a message written for a picker', async () => {
    // The claim's own wording answers someone who just asked for this device.
    // Nobody asked here -- the pane is being put back -- so the sentence has to
    // start by saying which simulator it is about.
    const text = await noticeFor({ reason: 'held-by-session', holder: 'term-9', message: 'held' })
    expect(text.startsWith('iPhone 17 Pro')).toBe(true)
  })

  it('warns rather than errors, since nothing the person did has failed', async () => {
    await noticeFor({ reason: 'boot-failed', message: 'nope' })
    expect(toast.mock.calls[0][1]).toBe('warning')
  })

  it('tells them about every one that did not come back', async () => {
    restore.mockResolvedValue([
      { sessionId: 'term-1', device, failure: { reason: 'boot-failed', message: 'a' } },
      {
        sessionId: 'term-2',
        device: { udid: 'udid-2', name: 'iPad Pro' },
        failure: { reason: 'held-by-other-vorn', pid: 7, message: 'b' }
      }
    ])
    await restoreDevicePanes()
    expect(toast).toHaveBeenCalledTimes(2)
    expect(toast.mock.calls[1][0]).toContain('iPad Pro')
  })
})
