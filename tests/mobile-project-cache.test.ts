// @vitest-environment jsdom
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act } from '@testing-library/react'
import { useAppStore } from '../src/renderer/stores'

const detectMobileProject = vi.fn()

const MOBILE = { isMobile: true, framework: 'expo' as const, needsDevClient: true }

beforeEach(() => {
  detectMobileProject.mockReset()
  detectMobileProject.mockResolvedValue(MOBILE)
  ;(window as unknown as { api: unknown }).api = { detectMobileProject }
  act(() => useAppStore.setState({ mobileProjectCache: new Map() }))
})

// The TTL timestamps are module-level and deliberately outlive any store
// reset — they track real elapsed time, not store lifetime. So each test uses
// its own project path rather than reaching in to clear them.
describe('mobile project cache', () => {
  it('stores the probe result under the project path', async () => {
    await act(async () => useAppStore.getState().loadMobileProject('/store'))
    expect(useAppStore.getState().mobileProjectCache.get('/store')).toEqual(MOBILE)
  })

  it('probes each project separately', async () => {
    detectMobileProject.mockResolvedValueOnce(MOBILE)
    detectMobileProject.mockResolvedValueOnce({
      isMobile: false,
      framework: null,
      needsDevClient: false
    })
    await act(async () => {
      await useAppStore.getState().loadMobileProject('/sep-a')
      await useAppStore.getState().loadMobileProject('/sep-b')
    })
    // Keying by project path is the whole point: one cache entry per directory,
    // so a web repo and an Expo repo open side by side don't answer for each
    // other and show the device button in the wrong card.
    expect(useAppStore.getState().mobileProjectCache.get('/sep-a')?.isMobile).toBe(true)
    expect(useAppStore.getState().mobileProjectCache.get('/sep-b')?.isMobile).toBe(false)
  })

  it('probes once when several callers ask for the same project at once', async () => {
    // Every session row for a project calls this on mount. Without the stamp
    // being set before the await, ten rows would mean ten readdir sweeps of the
    // same directory on every render pass.
    await act(async () => {
      await Promise.all([
        useAppStore.getState().loadMobileProject('/dedupe'),
        useAppStore.getState().loadMobileProject('/dedupe'),
        useAppStore.getState().loadMobileProject('/dedupe')
      ])
    })
    expect(detectMobileProject).toHaveBeenCalledTimes(1)
  })

  it('re-probes when forced', async () => {
    await act(async () => useAppStore.getState().loadMobileProject('/forced'))
    await act(async () => useAppStore.getState().loadMobileProject('/forced', true))
    expect(detectMobileProject).toHaveBeenCalledTimes(2)
  })

  it('retries after a failure instead of pinning the project as unprobed', async () => {
    detectMobileProject.mockRejectedValueOnce(new Error('bridge down'))
    await act(async () => useAppStore.getState().loadMobileProject('/flaky'))
    expect(useAppStore.getState().mobileProjectCache.has('/flaky')).toBe(false)

    // A transient bridge failure at startup must not cost the device button for
    // the whole TTL — the stamp is cleared on the way out so the next mount
    // tries again.
    await act(async () => useAppStore.getState().loadMobileProject('/flaky'))
    expect(useAppStore.getState().mobileProjectCache.get('/flaky')).toEqual(MOBILE)
  })

  it('ignores an empty project path', async () => {
    await act(async () => useAppStore.getState().loadMobileProject(''))
    expect(detectMobileProject).not.toHaveBeenCalled()
  })
})
