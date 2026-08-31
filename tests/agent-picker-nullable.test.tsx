// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { Mock } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { AgentPicker } from '../src/renderer/components/AgentPicker'
import type { AiAgentType, LaunchAgentType } from '../src/shared/types'

// Mock createPortal to render inline
vi.mock('react-dom', async () => {
  const actual = await vi.importActual<typeof import('react-dom')>('react-dom')
  return {
    ...actual,
    createPortal: (node: React.ReactNode) => node
  }
})

// Mock framer-motion
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

describe('AgentPicker with allowNone', () => {
  // Typed, because an untyped `vi.fn()` infers a zero-argument signature and
  // will not satisfy the prop it is passed as.
  let onChange: Mock<(agent: LaunchAgentType | null) => void>

  beforeEach(() => {
    onChange = vi.fn()
  })

  it('renders "Unassigned" when currentAgent is null', () => {
    const { getByText } = render(
      <AgentPicker
        currentAgent={null}
        onChange={onChange}
        installStatus={ALL_INSTALLED}
        allowNone
      />
    )
    expect(getByText('Unassigned')).toBeInTheDocument()
  })

  it('renders agent label when currentAgent is set', () => {
    const { getByText } = render(
      <AgentPicker
        currentAgent="claude"
        onChange={onChange}
        installStatus={ALL_INSTALLED}
        allowNone
      />
    )
    expect(getByText('Claude')).toBeInTheDocument()
  })

  it('shows "None" option in dropdown when allowNone is true', () => {
    const { getByText, getAllByRole } = render(
      <AgentPicker
        currentAgent="claude"
        onChange={onChange}
        installStatus={ALL_INSTALLED}
        allowNone
      />
    )
    // Open dropdown
    fireEvent.click(getAllByRole('button')[0])
    expect(getByText('None')).toBeInTheDocument()
  })

  it('does not show "None" option when allowNone is false', () => {
    const { queryByText, getAllByRole } = render(
      <AgentPicker currentAgent="claude" onChange={onChange} installStatus={ALL_INSTALLED} />
    )
    fireEvent.click(getAllByRole('button')[0])
    expect(queryByText('None')).not.toBeInTheDocument()
  })

  it('calls onChange(null) when "None" is selected', () => {
    const { getByText, getAllByRole } = render(
      <AgentPicker
        currentAgent="claude"
        onChange={onChange}
        installStatus={ALL_INSTALLED}
        allowNone
      />
    )
    fireEvent.click(getAllByRole('button')[0])
    fireEvent.click(getByText('None'))
    expect(onChange).toHaveBeenCalledWith(null)
  })

  it('calls onChange with agent type when an agent is selected', () => {
    const { getByText, getAllByRole } = render(
      <AgentPicker
        currentAgent={null}
        onChange={onChange}
        installStatus={ALL_INSTALLED}
        allowNone
      />
    )
    fireEvent.click(getAllByRole('button')[0])
    fireEvent.click(getByText('Copilot'))
    expect(onChange).toHaveBeenCalledWith('copilot')
  })

  it('does not call onChange for uninstalled agents', () => {
    const status = { ...ALL_INSTALLED, codex: false }
    const { getByText, getAllByRole } = render(
      <AgentPicker currentAgent="claude" onChange={onChange} installStatus={status} allowNone />
    )
    fireEvent.click(getAllByRole('button')[0])
    fireEvent.click(getByText('Codex'))
    expect(onChange).not.toHaveBeenCalled()
  })

  it('shows check mark on "None" when currentAgent is null', () => {
    const { container, getAllByRole } = render(
      <AgentPicker
        currentAgent={null}
        onChange={onChange}
        installStatus={ALL_INSTALLED}
        allowNone
      />
    )
    fireEvent.click(getAllByRole('button')[0])
    // The None button should contain a Check icon (svg with lucide class)
    const noneButton = container.querySelector('button .italic')?.closest('button')
    const checkIcon = noneButton?.querySelector('svg:last-child')
    expect(checkIcon).toBeInTheDocument()
  })
})

describe('AgentPicker with allowFromTask', () => {
  let onChange: Mock<(agent: LaunchAgentType | null) => void>

  beforeEach(() => {
    onChange = vi.fn()
  })

  it('renders "From Task" as the selected label when currentAgent is "fromTask"', () => {
    const { getByText } = render(
      <AgentPicker
        currentAgent="fromTask"
        onChange={onChange}
        installStatus={ALL_INSTALLED}
        allowFromTask
      />
    )
    expect(getByText('From Task')).toBeInTheDocument()
  })

  it('shows "From Task" option in the dropdown when allowFromTask is true', () => {
    const { getAllByText, getAllByRole } = render(
      <AgentPicker
        currentAgent="claude"
        onChange={onChange}
        installStatus={ALL_INSTALLED}
        allowFromTask
      />
    )
    // Open dropdown
    fireEvent.click(getAllByRole('button')[0])
    // "From Task" appears once in the menu (button renders Claude in the trigger)
    expect(getAllByText('From Task').length).toBeGreaterThanOrEqual(1)
  })

  it('does not show "From Task" option when allowFromTask is false', () => {
    const { queryByText, getAllByRole } = render(
      <AgentPicker currentAgent="claude" onChange={onChange} installStatus={ALL_INSTALLED} />
    )
    fireEvent.click(getAllByRole('button')[0])
    expect(queryByText('From Task')).not.toBeInTheDocument()
  })

  it('calls onChange("fromTask") when the "From Task" item is clicked', () => {
    const { getByText, getAllByRole } = render(
      <AgentPicker
        currentAgent="claude"
        onChange={onChange}
        installStatus={ALL_INSTALLED}
        allowFromTask
      />
    )
    fireEvent.click(getAllByRole('button')[0])
    fireEvent.click(getByText('From Task'))
    expect(onChange).toHaveBeenCalledWith('fromTask')
  })

  it('keeps "From Task" enabled even when no agents are installed', () => {
    const none: Record<AiAgentType, boolean> = {
      claude: false,
      copilot: false,
      codex: false,
      opencode: false,
      gemini: false
    }
    const { getByText, getAllByRole } = render(
      <AgentPicker
        currentAgent={null}
        onChange={onChange}
        installStatus={none}
        allowFromTask
        allowNone
      />
    )
    fireEvent.click(getAllByRole('button')[0])
    fireEvent.click(getByText('From Task'))
    expect(onChange).toHaveBeenCalledWith('fromTask')
  })

  it('shows a check mark next to "From Task" when it is the current value', () => {
    const { getByText } = render(
      <AgentPicker
        currentAgent="fromTask"
        onChange={onChange}
        installStatus={ALL_INSTALLED}
        allowFromTask
      />
    )
    // Open dropdown
    fireEvent.click(getByText('From Task'))
    // Find the button with "From Task" label that contains a Check icon
    const fromTaskMenuItems = Array.from(document.querySelectorAll('button')).filter(
      (b) => b.textContent?.trim() === 'From Task'
    )
    // At least the dropdown item should have a trailing check (svg with class containing "text-gray-400")
    const hasCheck = fromTaskMenuItems.some((b) => !!b.querySelector('svg.lucide-check'))
    expect(hasCheck).toBe(true)
  })
})
