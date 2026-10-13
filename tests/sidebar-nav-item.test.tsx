// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

import { SidebarNavItem } from '../packages/web/src/renderer/components/project-sidebar/SidebarNavItem'

const ICON = <svg data-testid="icon" />

describe('SidebarNavItem', () => {
  it('renders icon and label when expanded', () => {
    render(
      <SidebarNavItem
        isActive={false}
        isCollapsed={false}
        icon={ICON}
        label="All Projects"
        onClick={() => {}}
      />
    )
    expect(screen.getByTestId('icon')).toBeInTheDocument()
    expect(screen.getByText('All Projects')).toBeInTheDocument()
  })

  it('hides label and uses it as title when collapsed', () => {
    render(
      <SidebarNavItem
        isActive={false}
        isCollapsed={true}
        icon={ICON}
        label="All Projects"
        onClick={() => {}}
      />
    )
    expect(screen.queryByText('All Projects')).not.toBeInTheDocument()
    expect(screen.getByRole('button')).toHaveAttribute('title', 'All Projects')
  })

  it('renders zero badge (regression: do not hide 0)', () => {
    render(
      <SidebarNavItem
        isActive={false}
        isCollapsed={false}
        icon={ICON}
        label="All Projects"
        badge={0}
        onClick={() => {}}
      />
    )
    expect(screen.getByText('0')).toBeInTheDocument()
  })

  it('renders non-zero badge', () => {
    render(
      <SidebarNavItem
        isActive={false}
        isCollapsed={false}
        icon={ICON}
        label="All Projects"
        badge={7}
        onClick={() => {}}
      />
    )
    expect(screen.getByText('7')).toBeInTheDocument()
  })

  it('omits badge node when undefined', () => {
    const { container } = render(
      <SidebarNavItem
        isActive={false}
        isCollapsed={false}
        icon={ICON}
        label="All Projects"
        onClick={() => {}}
      />
    )
    expect(container.querySelector('.ml-auto')).toBeNull()
  })

  it('applies active styling when isActive', () => {
    render(
      <SidebarNavItem
        isActive={true}
        isCollapsed={false}
        icon={ICON}
        label="Active"
        onClick={() => {}}
      />
    )
    expect(screen.getByRole('button').className).toMatch(/bg-white\/\[0\.08\]/)
  })

  it('fires onClick when clicked', () => {
    const onClick = vi.fn()
    render(
      <SidebarNavItem
        isActive={false}
        isCollapsed={false}
        icon={ICON}
        label="x"
        onClick={onClick}
      />
    )
    fireEvent.click(screen.getByRole('button'))
    expect(onClick).toHaveBeenCalledTimes(1)
  })

  it('wraps icon with shrink-0 to prevent shrink under long labels', () => {
    const { container } = render(
      <SidebarNavItem
        isActive={false}
        isCollapsed={false}
        icon={ICON}
        label="x"
        onClick={() => {}}
      />
    )
    const wrapper = container.querySelector('span.shrink-0')
    expect(wrapper).not.toBeNull()
    expect(wrapper?.querySelector('[data-testid="icon"]')).not.toBeNull()
  })
})
