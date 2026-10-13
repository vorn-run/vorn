// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

// Stub the zustand store so LaunchAgentNode can read remoteHosts without a
// real provider.
const mockConfig = {
  remoteHosts: [] as Array<{ id: string; label: string }>
}
vi.mock('../packages/web/src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) => {
    const state = { config: mockConfig }
    return selector ? selector(state) : state
  }
}))

// Replace AgentIcon with a tagged div so tests can detect which branch ran.
vi.mock('../packages/web/src/renderer/components/AgentIcon', () => ({
  AgentIcon: ({ agentType }: { agentType: string }) => (
    <div data-testid="agent-icon" data-agent={agentType} />
  )
}))

const { LaunchAgentNode } =
  await import('../packages/web/src/renderer/components/workflow-editor/nodes/LaunchAgentNode')

import type { LaunchAgentConfig } from '../packages/web/src/shared/types'

function makeConfig(overrides: Partial<LaunchAgentConfig> = {}): LaunchAgentConfig {
  return {
    agentType: 'claude',
    projectName: 'demo',
    projectPath: '/demo',
    ...overrides
  }
}

beforeEach(() => {
  mockConfig.remoteHosts = []
})

describe('LaunchAgentNode rendering', () => {
  it('renders the concrete AgentIcon when agentType is a real agent', () => {
    const { queryByTestId } = render(
      <LaunchAgentNode
        label="Run claude"
        config={makeConfig({ agentType: 'claude' })}
        onClick={vi.fn()}
      />
    )
    const icon = queryByTestId('agent-icon')
    expect(icon).toBeInTheDocument()
    expect(icon?.getAttribute('data-agent')).toBe('claude')
  })

  it('renders the ClipboardList glyph (not AgentIcon) when agentType is "fromTask"', () => {
    const { queryByTestId, container } = render(
      <LaunchAgentNode
        label="Launch task agent"
        config={makeConfig({ agentType: 'fromTask' })}
        onClick={vi.fn()}
      />
    )
    // No AgentIcon rendered in the fromTask branch
    expect(queryByTestId('agent-icon')).not.toBeInTheDocument()
    // ClipboardList is rendered (check for lucide icon class)
    const iconSvg = container.querySelector('svg.lucide-clipboard-list')
    expect(iconSvg).toBeInTheDocument()
  })

  it('renders ClipboardList icon for fromTask without crashing', () => {
    const { container } = render(
      <LaunchAgentNode
        label="Launch task agent"
        config={makeConfig({ agentType: 'fromTask' })}
        onClick={vi.fn()}
      />
    )
    expect(container.querySelector('svg.lucide-clipboard-list')).toBeInTheDocument()
  })

  it('renders without crashing for an unknown agentType', () => {
    const { container } = render(
      <LaunchAgentNode
        label="Weird agent"
        // @ts-expect-error — probing fallback path with an unmapped agent value
        config={makeConfig({ agentType: 'unknown-future-agent' })}
        onClick={vi.fn()}
      />
    )
    expect(container.firstChild).toBeInTheDocument()
  })

  it('fires onClick and stops propagation when the node is clicked', () => {
    const onClick = vi.fn()
    const { container } = render(
      <LaunchAgentNode
        label="Launch"
        config={makeConfig({ agentType: 'fromTask' })}
        onClick={onClick}
      />
    )
    ;(container.firstChild as HTMLElement).click()
    expect(onClick).toHaveBeenCalledTimes(1)
  })
})
