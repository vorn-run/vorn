// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, cleanup, act, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const openExternal = vi.hoisted(() => vi.fn())

vi.hoisted(() => {
  Object.defineProperty(window, 'matchMedia', {
    value: () => ({ matches: false, addEventListener: () => {}, removeEventListener: () => {} }),
    writable: true
  })
})

import { useAppStore } from '../src/renderer/stores'
import { CardStatusBar } from '../src/renderer/components/card/CardStatusBar'
import type { ExtensionFooterReading } from '../packages/shared/src/types'

const reading = (over: Partial<ExtensionFooterReading> = {}): ExtensionFooterReading => ({
  extensionId: 'demo',
  extensionName: 'Demo',
  footerId: 'checks',
  title: 'Checks',
  items: [{ label: 'tests', value: '345 passed', tone: 'ok' }],
  computedAt: '2026-09-08T12:00:00.000Z',
  ...over
})

function seed(readings: ExtensionFooterReading[]): void {
  act(() => {
    useAppStore.setState({
      terminals: new Map([
        [
          't1',
          {
            id: 't1',
            session: {
              id: 't1',
              agentType: 'claude',
              projectName: 'Vorn',
              projectPath: '/p',
              status: 'idle',
              createdAt: 0
            },
            status: 'idle',
            lastOutputTimestamp: 1
          }
        ]
      ]) as never,
      extensionFooters: new Map(readings.length > 0 ? [['t1', readings]] : []),
      config: null
    })
  })
}

/**
 * A bar with room for `chips` of them.
 *
 * jsdom lays nothing out, so the measurement the component steps down from has
 * to be supplied: a chip is one unit wide, and the bar holds as many as it is
 * told to. Restored per test, because it is a prototype-wide stub.
 */
function room(chips: number): () => void {
  const width = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientWidth')
  const scroll = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'scrollWidth')
  Object.defineProperty(HTMLElement.prototype, 'clientWidth', {
    configurable: true,
    get: () => chips
  })
  Object.defineProperty(HTMLElement.prototype, 'scrollWidth', {
    configurable: true,
    get(this: HTMLElement) {
      return this.querySelectorAll('[data-chip]').length
    }
  })
  return () => {
    if (width) Object.defineProperty(HTMLElement.prototype, 'clientWidth', width)
    if (scroll) Object.defineProperty(HTMLElement.prototype, 'scrollWidth', scroll)
  }
}

/** What hovering a chip would say. */
function tipOf(inside: HTMLElement): string {
  vi.useFakeTimers()
  try {
    fireEvent.mouseEnter(inside.closest('[data-chip]')!.parentElement!)
    act(() => {
      vi.advanceTimersByTime(1000)
    })
    return document.body.textContent ?? ''
  } finally {
    vi.useRealTimers()
  }
}

describe('what an extension says in the card status bar', () => {
  let restore: (() => void) | undefined

  beforeEach(() => {
    ;(window as unknown as { api: Record<string, unknown> }).api = {
      isWorktreeDirty: () => Promise.resolve(false),
      getGitDiffStat: () => Promise.resolve(null),
      getGitBranch: () => Promise.resolve(null),
      detectIDEs: () => Promise.resolve([]),
      openInIDE: () => {},
      openExternal
    }
    openExternal.mockClear()
    restore = room(99)
  })

  afterEach(() => {
    restore?.()
    restore = undefined
    cleanup()
  })

  it('says nothing at all when no extension has anything to say', () => {
    seed([])
    render(<CardStatusBar terminalId="t1" />)
    expect(document.querySelector('[data-testid="extension-items-t1"]')).toBeNull()
  })

  it('draws every item in the bar, its footers in a settled order', () => {
    // Given in the wrong order on purpose: they arrive one footer at a time
    // from processes the host started in whatever order it got to them.
    seed([
      reading({
        extensionId: 'usage',
        extensionName: 'Usage',
        footerId: 'window',
        title: 'Usage',
        items: [{ label: 'context', value: '13%' }]
      }),
      reading()
    ])
    render(<CardStatusBar terminalId="t1" />)

    const chips = document.querySelectorAll('[data-testid="extension-items-t1"] [data-chip]')
    expect(chips).toHaveLength(2)
    expect(chips[0]).toHaveTextContent('tests345 passed')
    expect(chips[1]).toHaveTextContent('context13%')
  })

  // Only something wrong takes colour, so a bar of passing checks cannot drown
  // the one that failed.
  it('lets a reading that is fine recede, and colours only the one that is not', () => {
    seed([
      reading({
        items: [
          { label: 'tests', value: 'passing', tone: 'ok' },
          { label: 'lint', value: '2 warnings', tone: 'danger' },
          { label: 'ci', value: 'running' }
        ]
      })
    ])
    render(<CardStatusBar terminalId="t1" />)

    expect(screen.getByText('passing')).toHaveClass('text-ink-faint')
    expect(screen.getByText('2 warnings')).toHaveClass('text-danger')
    expect(screen.getByText('running')).toHaveClass('text-ink-secondary')
  })

  it('names the footer and holds the reason back for the tooltip when it failed', () => {
    seed([reading({ items: [], error: 'gh: not signed in' })])
    render(<CardStatusBar terminalId="t1" />)

    const failed = screen.getByText('Checks')
    expect(failed).toHaveClass('text-danger')
    // The bar has no room to spell it out; hovering does.
    expect(screen.queryByText('gh: not signed in')).toBeNull()
    expect(tipOf(failed)).toContain('gh: not signed in')
  })

  it('says which footer of whose extension a chip belongs to, and when it said so', () => {
    seed([reading()])
    render(<CardStatusBar terminalId="t1" />)

    expect(tipOf(screen.getByText('tests'))).toContain('Checks · Demo')
  })

  // Outside, not framed: an item links to a run or an issue on the web, and the
  // host has already held the address to http.
  it('opens an item that carries a link outside the app', () => {
    seed([reading({ items: [{ label: 'ci', value: '#568', href: 'https://example.com/568' }] })])
    render(<CardStatusBar terminalId="t1" />)

    fireEvent.click(screen.getByText('#568'))
    expect(openExternal).toHaveBeenCalledWith('https://example.com/568')
  })

  it('collapses a footer to one chip when the bar has no room for its items', () => {
    restore?.()
    restore = room(2)
    seed([
      reading({
        items: [
          { label: 'tests', value: 'passing', tone: 'ok' },
          { label: 'lint', value: '2 warnings', tone: 'danger' },
          { label: 'ci', value: 'running' }
        ]
      })
    ])
    render(<CardStatusBar terminalId="t1" />)

    expect(screen.queryByText('passing')).toBeNull()
    const chip = screen.getByText(/Checks ·/)
    expect(chip).toHaveTextContent('Checks · 3')
    // Something in there is wrong, and a chip standing for the lot has to say so.
    expect(chip).toHaveClass('text-danger')
  })

  it('drops footers from the right when even a chip each will not fit', () => {
    restore?.()
    restore = room(1)
    seed([
      reading(),
      reading({
        extensionId: 'usage',
        extensionName: 'Usage',
        footerId: 'window',
        title: 'Usage',
        items: [{ label: 'context', value: '13%' }]
      })
    ])
    render(<CardStatusBar terminalId="t1" />)

    expect(screen.getByText(/Checks ·/)).toBeInTheDocument()
    expect(screen.queryByText(/Usage ·/)).toBeNull()
  })

  it("keeps the card's own chips beside them", () => {
    seed([reading()])
    render(<CardStatusBar terminalId="t1" />)
    // One bar, still 22px, still carrying what the card says about itself.
    const bars = document.querySelectorAll('.h-\\[22px\\]')
    expect(bars).toHaveLength(1)
    expect(bars[0]).toContainElement(screen.getByTestId('extension-items-t1'))
  })
})
