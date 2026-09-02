// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const openExternalMock = vi.fn()

beforeEach(() => {
  openExternalMock.mockReset()
  // window.api used by SourceBadge
  Object.defineProperty(window, 'api', {
    configurable: true,
    value: { openExternal: openExternalMock }
  })
})

import { ConnectorIcon, SourceBadge } from '../src/renderer/components/ConnectorIcon'

describe('ConnectorIcon', () => {
  it('renders the GitHub SVG for the "github" id', () => {
    const { container } = render(<ConnectorIcon connectorId="github" />)
    expect(container.querySelector('svg')).toBeInTheDocument()
    expect(container.querySelector('svg')?.getAttribute('viewBox')).toBe('0 0 16 16')
  })

  it('renders the Linear SVG with the 24-unit brand-mark viewBox', () => {
    const { container } = render(<ConnectorIcon connectorId="linear" />)
    expect(container.querySelector('svg')).toBeInTheDocument()
    expect(container.querySelector('svg')?.getAttribute('viewBox')).toBe('0 0 24 24')
  })

  it('renders the MCP SVG for the "mcp" id', () => {
    const { container } = render(<ConnectorIcon connectorId="mcp" />)
    const svg = container.querySelector('svg')
    expect(svg).toBeInTheDocument()
    expect(svg?.getAttribute('viewBox')).toBe('0 0 24 24')
    // Two paths in the official ModelContextProtocol mark.
    expect(container.querySelectorAll('path')).toHaveLength(2)
  })

  it('falls back to an external-link icon for an unknown connector id', () => {
    const { container } = render(<ConnectorIcon connectorId="unknown-source" />)
    // Lucide ExternalLink renders an svg; we just assert something rendered.
    expect(container.querySelector('svg')).toBeInTheDocument()
  })

  // Knowing a connector's real id must not cost it the mark it already drew:
  // before the id was resolved these rendered as `mcp` and got the plug.
  it('keeps the plug for a packaged connector that ships no glyph', () => {
    const { container } = render(<ConnectorIcon connectorId="packdemo" packaged />)
    const svg = container.querySelector('svg')!
    expect(svg.getAttribute('viewBox')).toBe('0 0 24 24')
    expect(container.querySelectorAll('path')).toHaveLength(2)
  })

  it('still prefers a packaged connector own glyph when it has one', () => {
    const { container } = render(
      <ConnectorIcon
        connectorId="packdemo"
        packaged
        icon={{ viewBox: '0 0 24 24', paths: ['M2 2h9v9z'] }}
      />
    )
    expect(container.querySelector('path[d="M2 2h9v9z"]')).not.toBeNull()
  })

  it('honors the size and className props', () => {
    const { container } = render(
      <ConnectorIcon connectorId="github" size={24} className="text-red-500" />
    )
    const svg = container.querySelector('svg')!
    expect(svg.getAttribute('width')).toBe('24')
    expect(svg.getAttribute('height')).toBe('24')
    expect(svg.getAttribute('class')).toContain('text-red-500')
  })
})

describe('SourceBadge', () => {
  it('renders as a button (not an anchor) when a url is provided', () => {
    const { container } = render(
      <SourceBadge connectorId="github" url="https://example" label="#42" />
    )
    expect(container.querySelector('button')).toBeInTheDocument()
    expect(container.querySelector('a')).toBeNull()
  })

  it('calls openExternal when the badge is clicked', () => {
    const { container } = render(
      <SourceBadge connectorId="github" url="https://example/42" label="#42" />
    )
    fireEvent.click(container.querySelector('button')!)
    expect(openExternalMock).toHaveBeenCalledWith('https://example/42')
  })

  it('renders the label text inside the badge', () => {
    const { getByText } = render(<SourceBadge connectorId="github" url="https://x" label="#42" />)
    expect(getByText('#42')).toBeInTheDocument()
  })

  it('renders a static span (non-interactive) when no url is provided', () => {
    const { container } = render(<SourceBadge connectorId="github" label="#42" />)
    expect(container.querySelector('button')).toBeNull()
    expect(container.querySelector('span')).toBeInTheDocument()
  })

  it('stops event propagation so parent row click handlers do not fire', () => {
    const parentClick = vi.fn()
    const { container } = render(
      <div onClick={parentClick}>
        <SourceBadge connectorId="github" url="https://x" label="#1" />
      </div>
    )
    fireEvent.click(container.querySelector('button')!)
    expect(parentClick).not.toHaveBeenCalled()
  })
})
