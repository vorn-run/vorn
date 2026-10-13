// @vitest-environment jsdom
import { describe, it, expect } from 'vitest'
import { render } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { ProjectIcon } from '../packages/web/src/renderer/components/project-sidebar/ProjectIcon'
import { ICON_MAP } from '../packages/web/src/renderer/components/project-sidebar/icon-map'

describe('ProjectIcon', () => {
  it('renders an SVG for each known icon', () => {
    for (const name of Object.keys(ICON_MAP)) {
      const { container } = render(<ProjectIcon icon={name} />)
      const svg = container.querySelector('svg')
      expect(svg).toBeInTheDocument()
    }
  })

  it('falls back to default folder SVG for unknown icon', () => {
    const { container } = render(<ProjectIcon icon="NonExistent" />)
    const svg = container.querySelector('svg')
    expect(svg).toBeInTheDocument()
    const path = svg?.querySelector('path')
    expect(path?.getAttribute('d')).toContain('M3 7v10')
  })

  it('falls back to default folder SVG when no icon prop', () => {
    const { container } = render(<ProjectIcon />)
    const svg = container.querySelector('svg')
    expect(svg).toBeInTheDocument()
    const path = svg?.querySelector('path')
    expect(path?.getAttribute('d')).toContain('M3 7v10')
  })

  it('passes size prop to SVG dimensions', () => {
    const { container } = render(<ProjectIcon size={24} />)
    const svg = container.querySelector('svg')
    expect(svg?.getAttribute('width')).toBe('24')
    expect(svg?.getAttribute('height')).toBe('24')
  })

  it('defaults to size 14', () => {
    const { container } = render(<ProjectIcon />)
    const svg = container.querySelector('svg')
    expect(svg?.getAttribute('width')).toBe('14')
    expect(svg?.getAttribute('height')).toBe('14')
  })

  it('passes color prop to stroke', () => {
    const { container } = render(<ProjectIcon color="#ff0000" />)
    const svg = container.querySelector('svg')
    expect(svg?.getAttribute('stroke')).toBe('#ff0000')
  })

  it('uses default stroke when no color prop', () => {
    const { container } = render(<ProjectIcon />)
    const svg = container.querySelector('svg')
    expect(svg?.getAttribute('stroke')).toBe('currentColor')
  })
})
