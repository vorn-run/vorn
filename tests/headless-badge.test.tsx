// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { HeadlessSession } from '../packages/web/src/shared/types'

let headlessSessions: HeadlessSession[] = []

vi.mock('../packages/web/src/renderer/hooks/useFilteredHeadless', () => ({
  useFilteredHeadless: () => headlessSessions
}))

vi.mock('../packages/web/src/renderer/components/HeadlessPill', () => ({
  HeadlessPill: ({ session }: { session: HeadlessSession }) => (
    <div data-testid="headless-pill" data-id={session.id}>
      {session.id}
    </div>
  )
}))

import { HeadlessBadge } from '../packages/web/src/renderer/components/HeadlessBadge'

function headless(
  id: string,
  status: 'running' | 'exited' = 'running',
  exitCode = 0
): HeadlessSession {
  return {
    id,
    agentType: 'claude',
    projectName: 'p',
    projectPath: '/p',
    status,
    startedAt: Date.now(),
    exitCode: status === 'exited' ? exitCode : undefined,
    prompt: '',
    logs: ''
  } as unknown as HeadlessSession
}

beforeEach(() => {
  headlessSessions = []
})

describe('HeadlessBadge', () => {
  it('renders nothing when there are no headless sessions', () => {
    const { container } = render(<HeadlessBadge />)
    expect(container.firstChild).toBeNull()
  })

  it('renders a single icon + count badge when sessions exist (never inline pills)', () => {
    headlessSessions = [headless('h1'), headless('h2'), headless('h3')]
    const { getByLabelText, queryAllByTestId } = render(<HeadlessBadge />)
    const badge = getByLabelText('3 headless · 3 running')
    expect(badge).toBeInTheDocument()
    expect(badge.textContent).toContain('3')
    // Pills are not rendered until the popover is opened
    expect(queryAllByTestId('headless-pill')).toHaveLength(0)
  })

  it('opens a popover with all sessions when clicked', () => {
    headlessSessions = [headless('h1'), headless('h2')]
    const { getByLabelText, getAllByTestId } = render(<HeadlessBadge />)
    fireEvent.click(getByLabelText('2 headless · 2 running'))
    expect(getAllByTestId('headless-pill')).toHaveLength(2)
  })

  it('breakdown notes errored sessions when nothing is running', () => {
    headlessSessions = [headless('h1', 'exited', 1)]
    const { getByLabelText } = render(<HeadlessBadge />)
    expect(getByLabelText('1 headless · 1 errored')).toBeInTheDocument()
  })

  it('breakdown drops the secondary segment when nothing is running or errored', () => {
    headlessSessions = [headless('h1', 'exited', 0)]
    const { getByLabelText } = render(<HeadlessBadge />)
    expect(getByLabelText('1 headless')).toBeInTheDocument()
  })
})
