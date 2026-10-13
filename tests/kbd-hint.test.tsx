// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.mock('../packages/web/src/renderer/lib/keyboard-shortcuts', () => ({
  getShortcut: (id: string) => {
    const map: Record<string, { display: string }> = {
      'new-session': { display: '\u2318N' },
      escape: { display: 'Esc' }
    }
    return map[id]
  }
}))

import { KbdHint } from '../packages/web/src/renderer/components/KbdHint'

describe('KbdHint', () => {
  it('renders display text for a known shortcut', () => {
    render(<KbdHint shortcutId="new-session" />)
    expect(screen.getByText('\u2318N')).toBeInTheDocument()
  })

  it('renders nothing for an unknown shortcut', () => {
    const { container } = render(<KbdHint shortcutId="nonexistent" />)
    expect(container.innerHTML).toBe('')
  })

  it('renders a <kbd> element', () => {
    const { container } = render(<KbdHint shortcutId="escape" />)
    const kbd = container.querySelector('kbd')
    expect(kbd).toBeInTheDocument()
    expect(kbd).toHaveTextContent('Esc')
  })

  it('applies custom className', () => {
    const { container } = render(<KbdHint shortcutId="escape" className="ml-2" />)
    const kbd = container.querySelector('kbd')
    expect(kbd?.className).toContain('ml-2')
  })
})
