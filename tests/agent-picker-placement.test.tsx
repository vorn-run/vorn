// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { AgentPicker } from '../src/renderer/components/AgentPicker'
import type { AiAgentType } from '../src/shared/types'

vi.mock('react-dom', async () => {
  const actual = await vi.importActual<typeof import('react-dom')>('react-dom')
  return { ...actual, createPortal: (node: React.ReactNode) => node }
})

vi.mock('framer-motion', () => ({
  motion: {
    div: ({ children, ...props }: React.PropsWithChildren<Record<string, unknown>>) => (
      <div {...props}>{children}</div>
    )
  },
  AnimatePresence: ({ children }: React.PropsWithChildren) => <>{children}</>
}))

const ALL_INSTALLED: Record<AiAgentType, boolean> = {
  claude: true,
  copilot: true,
  codex: true,
  opencode: true,
  gemini: true
}

/**
 * The picker sits on the intent bar at the bottom of a card, so a menu that
 * only ever drops downward opens off the bottom of the window and lands behind
 * the status bar.
 */

function renderAt(top: number): HTMLElement {
  Object.defineProperty(window, 'innerHeight', { value: 800, writable: true })
  const { container } = render(
    <AgentPicker currentAgent="claude" onChange={vi.fn()} installStatus={ALL_INSTALLED} />
  )
  const trigger = container.querySelector('button')!
  trigger.getBoundingClientRect = () => ({ top, bottom: top + 20, left: 40, width: 90 }) as DOMRect
  fireEvent.click(trigger)
  return container.querySelector('.fixed') as HTMLElement
}

describe('AgentPicker menu placement', () => {
  it('opens upward when there is no room below', () => {
    const menu = renderAt(760)
    // Anchored by its bottom edge to the trigger's top, so the menu's own
    // height never has to be known.
    expect(menu.style.bottom).toBe('44px')
    expect(menu.style.top).toBe('')
  })

  it('opens downward when there is room', () => {
    const menu = renderAt(100)
    expect(menu.style.top).toBe('124px')
    expect(menu.style.bottom).toBe('')
  })

  it('stays downward when flipping would also overflow the top', () => {
    // A short window has no good direction; dropping down is the predictable
    // one, and the menu scrolls with the page rather than being clipped above.
    Object.defineProperty(window, 'innerHeight', { value: 200, writable: true })
    const menu = renderAt(120)
    expect(menu.style.top).toBe('144px')
  })
})
