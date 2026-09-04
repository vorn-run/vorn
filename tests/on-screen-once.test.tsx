// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { render, act } from '@testing-library/react'
import { useRef } from 'react'
import { useOnScreenOnce } from '../src/renderer/hooks/useOnScreenOnce'

/** Drives the observer by hand, since jsdom has none of its own. */
let fire: ((intersecting: boolean) => void) | null = null
let disconnects = 0
let lastOptions: IntersectionObserverInit | undefined

function installObserver(): void {
  disconnects = 0
  fire = null
  class FakeIO {
    constructor(
      private cb: IntersectionObserverCallback,
      options?: IntersectionObserverInit
    ) {
      lastOptions = options
      fire = (intersecting) => this.cb([{ isIntersecting: intersecting }] as never, this as never)
    }
    observe(): void {}
    disconnect(): void {
      disconnects++
    }
  }
  ;(globalThis as { IntersectionObserver?: unknown }).IntersectionObserver = FakeIO
}

function Probe({ onValue }: { onValue: (v: boolean) => void }) {
  const ref = useRef<HTMLDivElement>(null)
  onValue(useOnScreenOnce(ref))
  return <div ref={ref} />
}

afterEach(() => {
  delete (globalThis as { IntersectionObserver?: unknown }).IntersectionObserver
})

describe('waiting until a slot is worth attaching', () => {
  beforeEach(installObserver)

  it('starts closed, so nothing attaches on mount', () => {
    const seen: boolean[] = []
    render(<Probe onValue={(v) => seen.push(v)} />)
    expect(seen[0]).toBe(false)
  })

  it('opens when the element comes near the viewport', () => {
    const seen: boolean[] = []
    render(<Probe onValue={(v) => seen.push(v)} />)
    act(() => fire!(true))
    expect(seen.at(-1)).toBe(true)
  })

  it('stays open when it leaves again', () => {
    // A latch, not a tracker. The cost is paid once, so there is nothing to
    // undo -- and scrolling a long board must not tear terminals down.
    const seen: boolean[] = []
    render(<Probe onValue={(v) => seen.push(v)} />)
    act(() => fire!(true))
    act(() => fire!(false))
    expect(seen.at(-1)).toBe(true)
  })

  it('stops watching once it has latched', () => {
    render(<Probe onValue={() => {}} />)
    act(() => fire!(true))
    expect(disconnects).toBeGreaterThan(0)
  })

  it('reaches ahead of the viewport, so scrolling finds a drawn card', () => {
    render(<Probe onValue={() => {}} />)
    expect(lastOptions?.rootMargin).toBeTruthy()
  })
})

describe('where there is no observer at all', () => {
  it('answers true immediately, rather than reporting everything off screen', () => {
    // jsdom has none. Without this every terminal test would render a pane that
    // never attaches, and the suite would be asserting on empty cards.
    delete (globalThis as { IntersectionObserver?: unknown }).IntersectionObserver
    const seen: boolean[] = []
    render(<Probe onValue={(v) => seen.push(v)} />)
    expect(seen[0]).toBe(true)
  })
})
