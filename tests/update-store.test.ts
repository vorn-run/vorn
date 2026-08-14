import { describe, it, expect } from 'vitest'
import { vi } from 'vitest'
import type { UpdateStatus } from '../src/shared/types'

vi.mock('../src/renderer/lib/preferences', () => ({
  loadGridSettings: () => ({}),
  loadSidebarSettings: () => ({}),
  loadFlexibleLayouts: () => ({})
}))
;(global as unknown as { window: object }).window = {
  api: { saveConfig: vi.fn() }
}

const { createUISlice } = await import('../src/renderer/stores/ui-slice')

interface MinimalState {
  appUpdateStatus: UpdateStatus
  updateBannerDismissed: boolean
}

function makeSlice() {
  let state: MinimalState & Record<string, unknown> = {} as MinimalState
  const set = (updater: ((s: MinimalState) => Partial<MinimalState>) | Partial<MinimalState>) => {
    const patch = typeof updater === 'function' ? updater(state) : updater
    state = { ...state, ...patch }
  }
  const get = () => state as MinimalState
  const slice = createUISlice(set as never, get as never, {} as never)
  Object.assign(state, slice)
  return { state: () => state, slice }
}

describe('app update status store', () => {
  it('starts as unsupported, which is the truth until main says otherwise', () => {
    const { state } = makeSlice()
    expect(state().appUpdateStatus).toEqual({ kind: 'unsupported' })
    expect(state().updateBannerDismissed).toBe(false)
  })

  it('keeps the staged update when the banner is dismissed', () => {
    // The whole point of splitting dismissal from status: the old banner
    // nulled the version, so dismissing lost the update until relaunch.
    const { state, slice } = makeSlice()
    slice.setAppUpdateStatus({ kind: 'ready', version: '0.6.0' })
    slice.setUpdateBannerDismissed(true)

    expect(state().updateBannerDismissed).toBe(true)
    expect(state().appUpdateStatus).toEqual({ kind: 'ready', version: '0.6.0' })
  })

  it('holds the dismissal while the same version is re-announced', () => {
    const { state, slice } = makeSlice()
    slice.setAppUpdateStatus({ kind: 'ready', version: '0.6.0' })
    slice.setUpdateBannerDismissed(true)
    slice.setAppUpdateStatus({ kind: 'ready', version: '0.6.0' })

    expect(state().updateBannerDismissed).toBe(true)
  })

  it('lets a newer version re-earn the banner', () => {
    const { state, slice } = makeSlice()
    slice.setAppUpdateStatus({ kind: 'ready', version: '0.6.0' })
    slice.setUpdateBannerDismissed(true)
    slice.setAppUpdateStatus({ kind: 'ready', version: '0.7.0' })

    expect(state().updateBannerDismissed).toBe(false)
  })

  it('re-earns the banner after a fresh cycle through checking', () => {
    const { state, slice } = makeSlice()
    slice.setAppUpdateStatus({ kind: 'ready', version: '0.6.0' })
    slice.setUpdateBannerDismissed(true)
    slice.setAppUpdateStatus({ kind: 'checking' })
    slice.setAppUpdateStatus({ kind: 'ready', version: '0.6.0' })

    expect(state().updateBannerDismissed).toBe(false)
  })

  it('leaves dismissal alone for non-ready transitions', () => {
    const { state, slice } = makeSlice()
    slice.setUpdateBannerDismissed(true)
    slice.setAppUpdateStatus({ kind: 'downloading', version: '0.6.0', percent: 12 })

    expect(state().updateBannerDismissed).toBe(true)
  })
})
