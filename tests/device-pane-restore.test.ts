// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { DeviceClaimResult } from '../packages/shared/src/types'

const deviceClaim = vi.fn<(sessionId: string, udid: string) => Promise<DeviceClaimResult>>()

Object.defineProperty(window, 'api', {
  value: { notifyWidgetStatus: vi.fn(), deviceClaim, deviceRelease: vi.fn(async () => {}) },
  writable: true
})

const { useAppStore } = await import('../src/renderer/stores')

const KEY = 'vorn:devicePanes'

/** Two sessions on the board, both with a device open when the app closed. */
function boardWith(...sessionIds: string[]): void {
  useAppStore.setState({
    terminals: new Map(sessionIds.map((id) => [id, { id } as never])),
    devicePanes: new Map(),
    cardSplits: {}
  })
}

function requested(record: Record<string, { udid: string; name: string }>): void {
  localStorage.setItem(KEY, JSON.stringify(record))
}

const claimed = (udid: string, name: string): DeviceClaimResult => ({
  ok: true,
  udid,
  name,
  booted: true
})

beforeEach(() => {
  localStorage.clear()
  deviceClaim.mockReset()
  boardWith()
})

describe('remembering which simulator a session had open', () => {
  it('writes the device down when the pane opens', () => {
    useAppStore.getState().openDevicePane('term-1', { udid: 'udid-1', name: 'iPhone 17' })
    expect(JSON.parse(localStorage.getItem(KEY)!)).toEqual({
      'term-1': { udid: 'udid-1', name: 'iPhone 17' }
    })
  })

  it('forgets it when the pane closes', () => {
    useAppStore.getState().openDevicePane('term-1', { udid: 'udid-1', name: 'iPhone 17' })
    useAppStore.getState().closeDevicePane('term-1')
    expect(JSON.parse(localStorage.getItem(KEY)!)).toEqual({})
  })

  it('takes back the card the phone was sized for', () => {
    useAppStore.getState().openDevicePane('term-1', { udid: 'udid-1', name: 'iPhone 17' })
    expect(useAppStore.getState().cardSplits['term-1']).toBeDefined()
    useAppStore.getState().closeDevicePane('term-1')
    expect(useAppStore.getState().cardSplits['term-1']).toBeUndefined()
  })

  it('leaves a divider the person has dragged where they put it', () => {
    useAppStore.getState().openDevicePane('term-1', { udid: 'udid-1', name: 'iPhone 17' })
    useAppStore.setState({ cardSplits: { 'term-1': { terminal: 0.5, panes: [] } } })
    useAppStore.getState().closeDevicePane('term-1')
    expect(useAppStore.getState().cardSplits['term-1']).toEqual({ terminal: 0.5, panes: [] })
  })
})

describe('taking the simulators back on launch', () => {
  it('claims again, and only then shows the pane', async () => {
    boardWith('term-1')
    requested({ 'term-1': { udid: 'udid-1', name: 'iPhone 17' } })
    deviceClaim.mockResolvedValue(claimed('udid-1', 'iPhone 17'))
    const refused = await useAppStore.getState().restoreDevicePanes()
    expect(deviceClaim).toHaveBeenCalledWith('term-1', 'udid-1')
    expect(useAppStore.getState().devicePanes.get('term-1')?.udid).toBe('udid-1')
    expect(refused).toEqual([])
  })

  it('skips a session that never came back', async () => {
    boardWith('term-1')
    requested({ 'term-2': { udid: 'udid-2', name: 'iPad' } })
    await useAppStore.getState().restoreDevicePanes()
    expect(deviceClaim).not.toHaveBeenCalled()
  })

  it('never frames a device it did not get', async () => {
    boardWith('term-1')
    requested({ 'term-1': { udid: 'udid-1', name: 'iPhone 17' } })
    deviceClaim.mockResolvedValue({
      ok: false,
      reason: 'held-by-session',
      holder: 'term-9',
      message: 'held'
    })
    const refused = await useAppStore.getState().restoreDevicePanes()
    expect(useAppStore.getState().devicePanes.has('term-1')).toBe(false)
    expect(refused).toHaveLength(1)
    expect(refused[0].failure.reason).toBe('held-by-session')
  })

  it('keeps the record when another Vorn is driving it, so the next launch tries again', async () => {
    boardWith('term-1')
    requested({ 'term-1': { udid: 'udid-1', name: 'iPhone 17' } })
    deviceClaim.mockResolvedValue({
      ok: false,
      reason: 'held-by-other-vorn',
      pid: 999,
      message: 'held'
    })
    await useAppStore.getState().restoreDevicePanes()
    expect(JSON.parse(localStorage.getItem(KEY)!)['term-1']).toBeDefined()
  })

  it('forgets a simulator that is no longer on this machine', async () => {
    boardWith('term-1')
    requested({ 'term-1': { udid: 'udid-1', name: 'iPhone 17' } })
    deviceClaim.mockResolvedValue({ ok: false, reason: 'gone', message: 'no such udid' })
    const refused = await useAppStore.getState().restoreDevicePanes()
    expect(JSON.parse(localStorage.getItem(KEY)!)['term-1']).toBeUndefined()
    expect(refused).toEqual([])
  })

  it('goes one at a time rather than booting every simulator at once', async () => {
    boardWith('term-1', 'term-2')
    requested({
      'term-1': { udid: 'udid-1', name: 'iPhone 17' },
      'term-2': { udid: 'udid-2', name: 'iPad' }
    })
    let inFlight = 0
    let overlapped = false
    deviceClaim.mockImplementation(async (_sessionId, udid) => {
      inFlight++
      if (inFlight > 1) overlapped = true
      await Promise.resolve()
      inFlight--
      return claimed(udid, udid)
    })
    await useAppStore.getState().restoreDevicePanes()
    expect(overlapped).toBe(false)
    expect(deviceClaim).toHaveBeenCalledTimes(2)
  })

  it('reports a claim that fails outright rather than throwing out of the launch', async () => {
    boardWith('term-1')
    requested({ 'term-1': { udid: 'udid-1', name: 'iPhone 17' } })
    deviceClaim.mockRejectedValue(new Error('channel is gone'))
    const refused = await useAppStore.getState().restoreDevicePanes()
    expect(refused[0].failure.reason).toBe('boot-failed')
  })

  it('ignores a record that is not the shape it expects', async () => {
    boardWith('term-1')
    for (const raw of ['{', '[]', 'null', '{"term-1":{"udid":"udid-1"}}', '{"term-1":7}']) {
      localStorage.setItem(KEY, raw)
      await expect(useAppStore.getState().restoreDevicePanes()).resolves.toEqual([])
    }
    expect(deviceClaim).not.toHaveBeenCalled()
  })
})
