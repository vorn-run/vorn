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

describe('the band an extension writes under the status bar', () => {
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
  })

  afterEach(() => cleanup())

  it('draws nothing at all when no extension has anything to say', () => {
    seed([])
    render(<CardStatusBar terminalId="t1" />)
    expect(document.querySelector('[data-testid^="extension-footer-"]')).toBeNull()
  })

  it('draws one band per footer, named after it, beneath the bar it belongs to', () => {
    // Given in the wrong order on purpose: they arrive one footer at a time
    // from processes the host started in whatever order it got to them.
    seed([
      reading({
        extensionId: 'usage',
        extensionName: 'Usage',
        title: 'Usage',
        items: [{ label: 'context', value: '13%' }]
      }),
      reading()
    ])
    render(<CardStatusBar terminalId="t1" />)

    const bands = document.querySelectorAll('[data-testid^="extension-footer-t1-"]')
    expect(bands).toHaveLength(2)
    // Sorted by extension, so a band does not jump about as its neighbours report.
    expect(bands[0]).toHaveAttribute('data-testid', 'extension-footer-t1-demo-checks')
    expect(bands[1]).toHaveAttribute('data-testid', 'extension-footer-t1-usage-checks')
    expect(screen.getByText('345 passed')).toBeInTheDocument()
    expect(screen.getByText('13%')).toBeInTheDocument()
  })

  // Only something wrong takes colour, so a row of passing checks cannot drown
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

  it('says what went wrong instead of the items when the footer failed', () => {
    seed([reading({ items: [], error: 'gh: not signed in' })])
    render(<CardStatusBar terminalId="t1" />)

    const failed = screen.getByText('gh: not signed in')
    expect(failed).toHaveClass('text-danger')
    expect(screen.getByText('Checks')).toBeInTheDocument()
  })

  // Outside, not framed: an item links to a run or an issue on the web, and the
  // host has already held the address to http.
  it('opens an item that carries a link outside the app', () => {
    seed([reading({ items: [{ label: 'ci', value: '#568', href: 'https://example.com/568' }] })])
    render(<CardStatusBar terminalId="t1" />)

    fireEvent.click(screen.getByText('#568'))
    expect(openExternal).toHaveBeenCalledWith('https://example.com/568')
  })

  it("keeps drawing the card's own status bar beside them", () => {
    seed([reading()])
    render(<CardStatusBar terminalId="t1" />)
    // The bar's own content, which the band must not replace.
    expect(document.querySelector('.h-\\[22px\\]')).toBeInTheDocument()
  })
})
