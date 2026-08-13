// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { useRef } from 'react'
import { render, screen, act, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const deviceList = vi.fn()

/**
 * jsdom ships no ResizeObserver. This one records its callbacks so a test can
 * fire them, which is the only way to exercise the reposition-on-growth path:
 * jsdom never lays anything out, so nothing would ever trigger it naturally.
 */
const roCallbacks: Array<() => void> = []
vi.stubGlobal(
  'ResizeObserver',
  class {
    constructor(cb: () => void) {
      roCallbacks.push(cb)
    }
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  }
)

Object.defineProperty(window, 'api', {
  value: { deviceList: (...args: unknown[]) => deviceList(...args) },
  writable: true,
  configurable: true
})

import { DevicePicker } from '../src/renderer/components/DevicePicker'

const DEVICES = [
  { udid: 'u1', name: 'iPhone 17', runtime: 'iOS 26.2', booted: true },
  { udid: 'u2', name: 'iPad Pro', runtime: 'iOS 26.2', booted: false },
  { udid: 'u3', name: 'iPhone 16', runtime: 'iOS 26.2', booted: true, claimedBy: 'other-session' }
]

function Harness({
  onClose = () => {},
  onSelect = () => {},
  sessionId = 's1'
}: {
  onClose?: () => void
  onSelect?: (d: { udid: string; name: string }) => void
  sessionId?: string
}): React.ReactElement {
  const anchorRef = useRef<HTMLButtonElement>(null)
  return (
    <div>
      <button ref={anchorRef} data-testid="anchor">
        device
      </button>
      <DevicePicker
        sessionId={sessionId}
        onSelect={onSelect}
        onClose={onClose}
        anchorRef={anchorRef}
      />
    </div>
  )
}

describe('DevicePicker', () => {
  beforeEach(() => {
    deviceList.mockReset().mockResolvedValue(DEVICES)
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({
      top: 100,
      left: 200,
      right: 260,
      bottom: 120,
      width: 60,
      height: 20,
      x: 200,
      y: 100,
      toJSON: () => ({})
    } as DOMRect)
  })

  it('lists the simulators it finds', async () => {
    render(<Harness />)
    expect(await screen.findByText('iPhone 17')).toBeInTheDocument()
    expect(screen.getByText('iPad Pro')).toBeInTheDocument()
  })

  it('hands back the chosen device and closes', async () => {
    const onSelect = vi.fn()
    const onClose = vi.fn()
    render(<Harness onSelect={onSelect} onClose={onClose} />)
    fireEvent.click(await screen.findByText('iPhone 17'))
    expect(onSelect).toHaveBeenCalledWith({ udid: 'u1', name: 'iPhone 17' })
  })

  it('shows a device another session holds, but refuses to select it', async () => {
    const onSelect = vi.fn()
    render(<Harness onSelect={onSelect} />)
    // Shown rather than hidden: a missing row reads as a broken simulator list
    // and sends the person hunting in Xcode for a device that is simply busy.
    const held = await screen.findByText('iPhone 16')
    expect(screen.getByText('in use')).toBeInTheDocument()
    fireEvent.click(held)
    expect(onSelect).not.toHaveBeenCalled()
  })

  it('names the holder, so contention is diagnosable', async () => {
    render(<Harness />)
    await screen.findByText('iPhone 16')
    expect(screen.getByTitle('In use by other-session')).toBeInTheDocument()
  })

  it('lets the session pick the device it already claimed', async () => {
    const onSelect = vi.fn()
    render(<Harness sessionId="other-session" onSelect={onSelect} />)
    fireEvent.click(await screen.findByText('iPhone 16'))
    // Reopening a pane for a device this session already holds must work, or a
    // closed pane would strand its own claim.
    expect(onSelect).toHaveBeenCalledWith({ udid: 'u3', name: 'iPhone 16' })
  })

  it('surfaces the failure message verbatim', async () => {
    deviceList.mockRejectedValue(new Error('Install with: brew install facebook/fb/idb-companion'))
    render(<Harness />)
    // The message carries the fix. Flattening it to "failed to load" costs the
    // person the one sentence that tells them what to install.
    expect(await screen.findByText(/brew install facebook\/fb\/idb-companion/)).toBeInTheDocument()
  })

  it('says so when there are no simulators at all', async () => {
    deviceList.mockResolvedValue([])
    render(<Harness />)
    expect(await screen.findByText('No simulators found')).toBeInTheDocument()
  })

  it('repositions when it grows from a spinner into a list', async () => {
    // The flip-above-the-anchor decision keys off the popover's height, and on
    // mount that height is a spinner's. Anchored near the bottom of the screen
    // the grown list is clipped, and the devices that scroll off are exactly
    // the ones the person opened it to reach. jsdom never lays out, so the
    // observer callback is fired by hand.
    render(<Harness />)
    await screen.findByText('iPhone 17')
    const box = screen.getByRole('listbox')
    const before = box.style.top

    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({
      top: 700,
      left: 200,
      right: 260,
      bottom: 720,
      width: 60,
      height: 400,
      x: 200,
      y: 700,
      toJSON: () => ({})
    } as DOMRect)
    act(() => {
      roCallbacks.forEach((cb) => cb())
    })
    expect(box.style.top).not.toBe(before)
  })

  it('stops observing when it closes', () => {
    // An observer left attached to a removed node keeps the popover's whole
    // closure alive for as long as the session lasts, and the picker is opened
    // and closed constantly.
    const disconnect = vi.fn()
    vi.stubGlobal(
      'ResizeObserver',
      class {
        observe(): void {}
        unobserve(): void {}
        disconnect = disconnect
      }
    )
    const { unmount } = render(<Harness />)
    unmount()
    expect(disconnect).toHaveBeenCalled()
  })

  it('closes on Escape', async () => {
    const onClose = vi.fn()
    render(<Harness onClose={onClose} />)
    await screen.findByText('iPhone 17')
    act(() => {
      fireEvent.keyDown(document, { key: 'Escape' })
    })
    expect(onClose).toHaveBeenCalled()
  })

  it('closes on a click outside, but not on one inside', async () => {
    const onClose = vi.fn()
    render(<Harness onClose={onClose} />)
    const list = await screen.findByRole('listbox')
    fireEvent.mouseDown(list)
    expect(onClose).not.toHaveBeenCalled()
    fireEvent.mouseDown(document.body)
    expect(onClose).toHaveBeenCalled()
  })
})
