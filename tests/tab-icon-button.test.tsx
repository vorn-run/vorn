// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { ReactNode } from 'react'

vi.mock('../packages/web/src/renderer/components/Tooltip', () => ({
  Tooltip: ({ children }: { children: ReactNode }) => <>{children}</>
}))

// Stub every store/hook/API the module imports transitively.
Object.defineProperty(window, 'api', {
  value: { detectIDEs: vi.fn().mockResolvedValue([]) },
  writable: true
})

import { TabIconButton } from '../packages/web/src/renderer/components/TabView'

describe('TabIconButton', () => {
  it('fires onClick and prevents bubbling so the tab does not activate', () => {
    const onClick = vi.fn()
    const onParentClick = vi.fn()
    render(
      <div onClick={onParentClick}>
        <TabIconButton label="Browse files" icon={<span>icon</span>} onClick={onClick} />
      </div>
    )
    fireEvent.click(screen.getByRole('button', { name: 'Browse files' }))
    expect(onClick).toHaveBeenCalledTimes(1)
    expect(onParentClick).not.toHaveBeenCalled()
  })

  it('allows propagation when no onClick is provided (for ConfirmPopover wrapping)', () => {
    const onParentClick = vi.fn()
    render(
      <div onClick={onParentClick}>
        <TabIconButton label="Close session" icon={<span>icon</span>} />
      </div>
    )
    fireEvent.click(screen.getByRole('button', { name: 'Close session' }))
    expect(onParentClick).toHaveBeenCalledTimes(1)
  })

  it('applies a custom hover class', () => {
    render(
      <TabIconButton
        label="Close session"
        icon={<span>icon</span>}
        hoverClass="hover:text-red-400"
      />
    )
    expect(screen.getByRole('button', { name: 'Close session' }).className).toContain(
      'hover:text-red-400'
    )
  })
})
