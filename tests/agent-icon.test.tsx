// @vitest-environment jsdom
import { describe, it, expect } from 'vitest'
import { render } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { AgentIcon } from '../packages/web/src/renderer/components/AgentIcon'
import type { AgentType } from '../packages/web/src/shared/types'

describe('AgentIcon', () => {
  const types: AgentType[] = ['claude', 'copilot', 'codex', 'opencode', 'gemini', 'shell']

  it.each(types)('renders an svg for %s', (agentType) => {
    const { container } = render(<AgentIcon agentType={agentType} />)
    const svg = container.querySelector('svg')
    expect(svg).toBeInTheDocument()
  })

  it('renders a terminal icon for shell (not an AI agent icon)', () => {
    const { container } = render(<AgentIcon agentType="shell" />)
    const svg = container.querySelector('svg')
    expect(svg).toBeInTheDocument()
    expect(svg?.classList.contains('text-gray-400')).toBe(true)
  })

  it('defaults to size 16', () => {
    const { container } = render(<AgentIcon agentType="claude" />)
    const svg = container.querySelector('svg')
    expect(svg?.getAttribute('width')).toBe('16')
    expect(svg?.getAttribute('height')).toBe('16')
  })

  it('passes custom size', () => {
    const { container } = render(<AgentIcon agentType="copilot" size={24} />)
    const svg = container.querySelector('svg')
    expect(svg?.getAttribute('width')).toBe('24')
    expect(svg?.getAttribute('height')).toBe('24')
  })

  it('renders claude icon with correct fill color', () => {
    const { container } = render(<AgentIcon agentType="claude" />)
    const path = container.querySelector('path')
    expect(path?.getAttribute('fill')).toBe('#D97757')
  })
})
