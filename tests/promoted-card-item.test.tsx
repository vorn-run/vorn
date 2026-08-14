// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, fireEvent, screen } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { PromotedCard } from '../src/renderer/hooks/usePromotedCards'

const setSelected = vi.fn()
const setFocusedTerminal = vi.fn()
const setActiveTabId = vi.fn()
const returnCard = vi.fn()
const closeCard = vi.fn()
let selectedTerminalId: string | null = null
let layoutMode = 'grid'

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) => {
    const state = {
      selectedTerminalId,
      config: { defaults: { layoutMode } },
      setSelectedTerminal: setSelected,
      setFocusedTerminal,
      setActiveTabId,
      returnCardToSession: returnCard,
      closeCard
    }
    return selector ? selector(state) : state
  }
}))

const { PromotedCardItem } =
  await import('../src/renderer/components/project-sidebar/PromotedCardItem')

const fileCard: PromotedCard = {
  id: 'card:t1:0',
  kind: 'editor',
  sessionId: 't1',
  subject: '/repo/src/server.ts'
}
const pageCard: PromotedCard = {
  id: 'card:t1:1',
  kind: 'browser',
  sessionId: 't1',
  subject: 'https://vorn.dev/docs'
}

/**
 * A card sits in the sidebar among session rows. It is a grid cell exactly as a
 * session is, so it has to read as a sibling of one — anything else says it is a
 * lesser kind of thing, in the one list whose job is telling them apart.
 */
describe('PromotedCardItem', () => {
  beforeEach(() => {
    selectedTerminalId = null
    layoutMode = 'grid'
    vi.clearAllMocks()
  })

  it("shares the session row's metrics rather than approximating them", () => {
    const { container } = render(<PromotedCardItem card={fileCard} />)
    const row = container.firstElementChild as HTMLElement

    // Copied from SessionItem: same padding, text size, gap and left edge. An
    // extra indent here made cards read as children of the row above them.
    for (const cls of ['px-2', 'py-1', 'text-[12px]', 'gap-2']) {
      expect(row.className).toContain(cls)
    }
    expect(row.className).not.toContain('pl-6')
  })

  it('names a file by its filename, not its path', () => {
    render(<PromotedCardItem card={fileCard} />)
    expect(screen.getByText('server.ts')).toBeInTheDocument()
  })

  it('names a page by its host', () => {
    render(<PromotedCardItem card={pageCard} />)
    expect(screen.getByText('vorn.dev')).toBeInTheDocument()
  })

  it('selects the card, and offers return and close', () => {
    render(<PromotedCardItem card={fileCard} />)

    fireEvent.click(screen.getByText('server.ts'))
    expect(setSelected).toHaveBeenCalledWith('card:t1:0')

    fireEvent.click(screen.getByRole('button', { name: /back in its session card/ }))
    expect(returnCard).toHaveBeenCalledWith('card:t1:0')

    fireEvent.click(screen.getByRole('button', { name: /Close server\.ts/ }))
    expect(closeCard).toHaveBeenCalledWith('card:t1:0')
  })

  it('closes through the one action that asks about unsaved edits', () => {
    // Not `closeEditorPane`/`closeBrowserPane` directly. This row and the card's
    // tab both used to pick the collection themselves and close it outright,
    // while only the card's own ✕ confirmed — so the same file discarded its
    // buffer silently or not depending on which ✕ you reached for.
    render(<PromotedCardItem card={pageCard} />)

    fireEvent.click(screen.getByRole('button', { name: /Close vorn\.dev/ }))
    expect(closeCard).toHaveBeenCalledWith('card:t1:1')
  })

  it('focuses the card, not the session it came from', () => {
    // The whole point. Focusing the owner handed back the terminal and every
    // pane with the file wedged in beside them, and the card then offered to go
    // "back" to the session it was already sitting inside.
    render(<PromotedCardItem card={fileCard} />)
    fireEvent.click(screen.getByText('server.ts'))

    expect(setFocusedTerminal).toHaveBeenCalledWith('card:t1:0')
    expect(setFocusedTerminal).not.toHaveBeenCalledWith('t1')
  })

  it("activates the card's own tab, not the owner's", () => {
    layoutMode = 'tabs'
    render(<PromotedCardItem card={pageCard} />)
    fireEvent.click(screen.getByText('vorn.dev'))

    expect(setActiveTabId).toHaveBeenCalledWith('card:t1:1')
    expect(setFocusedTerminal).toHaveBeenCalledWith(null)
  })
})
