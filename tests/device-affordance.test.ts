// @vitest-environment jsdom
import { describe, it, expect } from 'vitest'
import { shouldShowDeviceButton, isSelectable } from '../src/renderer/lib/device-affordance'
import type { DeviceInfo } from '../src/shared/types'

const EXPO = { isMobile: true, framework: 'expo' as const, needsDevClient: true }
const WEB = { isMobile: false, framework: null, needsDevClient: false }

const device = (over: Partial<DeviceInfo> = {}): DeviceInfo => ({
  udid: 'u1',
  name: 'iPhone 17',
  runtime: 'iOS 26.2',
  booted: false,
  ...over
})

describe('device button gate', () => {
  it('offers a device on a mobile project', () => {
    expect(shouldShowDeviceButton(EXPO, false)).toBe(true)
  })

  it('stays out of the way on a project with no mobile app in it', () => {
    // The whole reason for the gate: a simulator button on a web or backend
    // repo is a control that can only disappoint, sitting next to two that
    // always work.
    expect(shouldShowDeviceButton(WEB, false)).toBe(false)
  })

  it('shows nothing while the project is still unprobed', () => {
    // Defaulting to visible would flash the button in and out on every mount
    // as the probe resolves; defaulting to hidden means it simply appears.
    expect(shouldShowDeviceButton(undefined, false)).toBe(false)
  })

  it('keeps the button while a device pane is open, whatever detection says', () => {
    // Detection is a heuristic and will be wrong eventually. Having the control
    // vanish out from under a simulator someone is actively driving — leaving
    // no way to close it — is far worse than a button that should not be there.
    expect(shouldShowDeviceButton(WEB, true)).toBe(true)
    expect(shouldShowDeviceButton(undefined, true)).toBe(true)
  })
})

describe('device picker selectability', () => {
  it('allows a free device', () => {
    expect(isSelectable(device(), 's1')).toBe(true)
  })

  it('allows the session its own claim, so reopening a pane works', () => {
    expect(isSelectable(device({ claimedBy: 's1' }), 's1')).toBe(true)
  })

  it('refuses a device another session holds', () => {
    // Two agents tapping one screen produce garbage that reads as flaky app
    // behaviour, so contention has to be refused at the picker rather than
    // discovered later.
    expect(isSelectable(device({ claimedBy: 's2' }), 's1')).toBe(false)
  })
})
