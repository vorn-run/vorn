// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { render, act } from '@testing-library/react'
import { useRef } from 'react'
import { useOnScreen } from '../src/renderer/hooks/useOnScreen'

/** Drives the observer by hand, since jsdom has none of its own. */
let fire: ((intersecting: boolean) => void) | null = null
let disconnects = 0

function installObserver(): void {
  disconnects = 0
  fire = null
  class FakeIO {
    constructor(private cb: IntersectionObserverCallback) {
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
  onValue(useOnScreen(ref))
  return <div ref={ref} />
}

afterEach(() => {
  delete (globalThis as { IntersectionObserver?: unknown }).IntersectionObserver
})

describe('tracking whether a slot is on screen', () => {
  beforeEach(installObserver)

  it('starts off screen', () => {
    const seen: boolean[] = []
    render(<Probe onValue={(v) => seen.push(v)} />)
    expect(seen[0]).toBe(false)
  })

  it('reports both edges, unlike the latch', () => {
    const seen: boolean[] = []
    render(<Probe onValue={(v) => seen.push(v)} />)
    act(() => fire!(true))
    expect(seen.at(-1)).toBe(true)
    act(() => fire!(false))
    expect(seen.at(-1)).toBe(false)
  })

  it('stops watching when the slot unmounts', () => {
    const { unmount } = render(<Probe onValue={() => {}} />)
    unmount()
    expect(disconnects).toBe(1)
  })
})

describe('where there is no observer at all', () => {
  it('answers true, so nothing is hidden on a surface that cannot measure', () => {
    const seen: boolean[] = []
    render(<Probe onValue={(v) => seen.push(v)} />)
    expect(seen.at(-1)).toBe(true)
  })
})
