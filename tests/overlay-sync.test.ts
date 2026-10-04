// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  QUIET_FRAMES,
  SAFETY_INTERVAL_MS,
  startOverlaySync,
  type OverlaySync
} from '../src/renderer/lib/overlay-sync'

let frames: Array<() => void> = []
let sync: OverlaySync | null = null
let root: HTMLElement
let slot: HTMLElement
/** What `sync` reports for the next frames: moved or not. */
let moving = false
const syncCalls = vi.fn((): boolean => moving)

/** Runs the frames queued so far, as one vsync would. */
function vsync(): void {
  const now = frames
  frames = []
  for (const f of now) f()
}

/** Runs frames until the loop stops, and says how many it took. */
function settle(limit = 100): number {
  let n = 0
  while (frames.length && n < limit) {
    vsync()
    n++
  }
  return n
}

const flushMutations = (): Promise<void> => new Promise((r) => setTimeout(r, 0))

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] })
  frames = []
  moving = false
  syncCalls.mockClear()
  vi.stubGlobal('requestAnimationFrame', (cb: () => void) => frames.push(cb))
  vi.stubGlobal('cancelAnimationFrame', () => {
    frames = []
  })
  document.body.innerHTML = ''
  root = document.createElement('div')
  slot = document.createElement('div')
  document.body.append(slot, root)
  sync = startOverlaySync({
    root,
    ids: () => ['t1'],
    sync: syncCalls,
    slots: () => [slot]
  })
})

afterEach(() => {
  sync?.stop()
  sync = null
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('the overlay sync', () => {
  it('looks once at the start, then stops when nothing moves', () => {
    expect(settle()).toBe(QUIET_FRAMES)
    expect(frames).toHaveLength(0)
  })

  it('keeps going while something moves, and stops a few frames after it stops', () => {
    settle()
    moving = true
    sync!.request()
    for (let i = 0; i < 20; i++) vsync()
    expect(frames).toHaveLength(1)
    moving = false
    expect(settle()).toBe(QUIET_FRAMES)
  })

  it('queues one frame when a sync itself asks for the next one', () => {
    settle()
    syncCalls.mockImplementationOnce(() => {
      sync!.request()
      return true
    })
    sync!.request()
    vsync()
    expect(frames).toHaveLength(1)
  })

  it('wakes for a change in the page outside the overlay', async () => {
    settle()
    slot.style.transform = 'translateX(10px)'
    await flushMutations()
    expect(frames).toHaveLength(1)
  })

  it('stays asleep for changes inside the overlay, which are its own and xterm drawing', async () => {
    settle()
    const wrapper = document.createElement('div')
    root.appendChild(wrapper)
    wrapper.style.left = '12px'
    await flushMutations()
    expect(frames).toHaveLength(0)
  })

  it('wakes for a scroll anywhere, a resize, a transition (and its delayed start) and hover', () => {
    for (const fire of [
      () => slot.dispatchEvent(new Event('scroll')),
      () => window.dispatchEvent(new Event('resize')),
      () => slot.dispatchEvent(new Event('transitionrun', { bubbles: true })),
      () => slot.dispatchEvent(new Event('transitionstart', { bubbles: true })),
      () => slot.dispatchEvent(new Event('pointerover', { bubbles: true }))
    ]) {
      settle()
      fire()
      expect(frames).toHaveLength(1)
    }
  })

  it('keeps going while an animation on an ancestor of a slot runs, even with no movement seen', () => {
    settle()
    const animations = [{ playState: 'running', effect: { target: document.body } }]
    ;(document as unknown as { getAnimations: () => unknown[] }).getAnimations = () => animations
    sync!.request()
    for (let i = 0; i < 30; i++) vsync()
    expect(frames).toHaveLength(1)
    animations[0].playState = 'finished'
    expect(settle()).toBeGreaterThan(0)
    expect(frames).toHaveLength(0)
    delete (document as unknown as { getAnimations?: unknown }).getAnimations
  })

  it('ignores animations that hold no slot, such as a spinner elsewhere', () => {
    settle()
    const spinner = document.createElement('span')
    document.body.appendChild(spinner)
    ;(document as unknown as { getAnimations: () => unknown[] }).getAnimations = () => [
      { playState: 'running', effect: { target: spinner } }
    ]
    sync!.request()
    expect(settle()).toBe(QUIET_FRAMES)
    delete (document as unknown as { getAnimations?: unknown }).getAnimations
  })

  it('catches a move nothing reported at the safety beat', () => {
    settle()
    syncCalls.mockClear()
    vi.advanceTimersByTime(SAFETY_INTERVAL_MS)
    expect(syncCalls).toHaveBeenCalledTimes(1)
    expect(frames).toHaveLength(0)
    moving = true
    vi.advanceTimersByTime(SAFETY_INTERVAL_MS)
    expect(frames).toHaveLength(1)
  })

  it('does nothing once stopped', async () => {
    settle()
    sync!.stop()
    sync!.request()
    slot.style.top = '5px'
    window.dispatchEvent(new Event('resize'))
    await flushMutations()
    syncCalls.mockClear()
    vi.advanceTimersByTime(SAFETY_INTERVAL_MS * 3)
    expect(frames).toHaveLength(0)
    expect(syncCalls).not.toHaveBeenCalled()
  })
})
