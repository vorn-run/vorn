// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

// Mocks must be hoisted before imports that use them
vi.mock('react-dom', async () => {
  const actual = await vi.importActual<typeof import('react-dom')>('react-dom')
  return { ...actual, createPortal: (node: React.ReactNode) => node }
})
vi.mock('framer-motion', () => ({
  motion: {
    div: ({ children, ...props }: React.PropsWithChildren<Record<string, unknown>>) => (
      <div {...props}>{children}</div>
    )
  },
  AnimatePresence: ({ children }: React.PropsWithChildren) => <>{children}</>
}))

// Spy on the props every AgentPicker render receives so tests can read
// `allowFromTask` for each re-render without mounting the real component.
const agentPickerProps: Array<Record<string, unknown>> = []
vi.mock('../src/renderer/components/AgentPicker', () => ({
  AgentPicker: (props: Record<string, unknown>) => {
    agentPickerProps.push(props)
    return (
      <div data-testid="agent-picker-mock" data-allow-from-task={String(props.allowFromTask)} />
    )
  }
}))

// Capture props each render so tests can read allowFromContext / isFromContext
// without mounting the real picker.
const projectPickerProps: Array<Record<string, unknown>> = []
vi.mock('../src/renderer/components/ProjectPicker', () => ({
  ProjectPicker: (props: Record<string, unknown>) => {
    projectPickerProps.push(props)
    return <div data-testid="project-picker" />
  }
}))
vi.mock('../src/renderer/components/rich-editor/RichMarkdownEditor', () => ({
  RichMarkdownEditor: () => <div data-testid="rich-md" />
}))
vi.mock('../src/renderer/components/workflow-editor/panels/VariableAutocomplete', () => ({
  VariableAutocomplete: () => <div data-testid="variable-autocomplete" />
}))
vi.mock('../src/renderer/components/Tooltip', () => ({
  Tooltip: ({ children }: React.PropsWithChildren) => <>{children}</>
}))
vi.mock('../src/renderer/hooks/useAgentInstallStatus', () => ({
  useAgentInstallStatus: () => ({
    status: {
      claude: true,
      copilot: true,
      codex: true,
      opencode: true,
      gemini: true
    }
  })
}))

// Minimal zustand-style store mock — LaunchAgentConfigForm reads projects,
// tasks, and defaults.defaultAgent via useAppStore selectors.
const mockConfig = {
  projects: [],
  tasks: [],
  defaults: { defaultAgent: 'claude' as const }
}
vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) => {
    const state = { config: mockConfig }
    return selector ? selector(state) : state
  }
}))

// window.api shim — LaunchAgentConfigForm only calls isGitRepo + listWorktrees
// in an effect that's harmless if we stub them as resolved empty arrays.
beforeEach(() => {
  agentPickerProps.length = 0
  projectPickerProps.length = 0
  ;(globalThis as unknown as { window: { api: Record<string, unknown> } }).window = {
    api: {
      isGitRepo: vi.fn().mockResolvedValue(true),
      listWorktrees: vi.fn().mockResolvedValue([])
    }
  }
})

const { LaunchAgentConfigForm } =
  await import('../src/renderer/components/workflow-editor/panels/LaunchAgentConfigForm')

import type { LaunchAgentConfig } from '../src/shared/types'

function baseConfig(overrides: Partial<LaunchAgentConfig> = {}): LaunchAgentConfig {
  return {
    agentType: 'claude',
    projectName: '',
    projectPath: '',
    ...overrides
  }
}

describe('LaunchAgentConfigForm — canUseFromTask visibility', () => {
  it('passes allowFromTask=true when trigger is taskStatusChanged', () => {
    render(
      <LaunchAgentConfigForm
        config={baseConfig()}
        onChange={vi.fn()}
        triggerType="taskStatusChanged"
      />
    )
    const last = agentPickerProps.at(-1)!
    expect(last.allowFromTask).toBe(true)
  })

  it('passes allowFromTask=true when trigger is taskCreated', () => {
    render(
      <LaunchAgentConfigForm config={baseConfig()} onChange={vi.fn()} triggerType="taskCreated" />
    )
    expect(agentPickerProps.at(-1)!.allowFromTask).toBe(true)
  })

  it('passes allowFromTask=false when trigger is manual and prompt source is inline', () => {
    render(<LaunchAgentConfigForm config={baseConfig()} onChange={vi.fn()} triggerType="manual" />)
    expect(agentPickerProps.at(-1)!.allowFromTask).toBe(false)
  })

  it('passes allowFromTask=true when prompt source is queue, even without a task trigger', () => {
    render(
      <LaunchAgentConfigForm
        config={baseConfig({ taskFromQueue: true })}
        onChange={vi.fn()}
        triggerType="manual"
      />
    )
    expect(agentPickerProps.at(-1)!.allowFromTask).toBe(true)
  })

  it('passes allowFromTask=true when prompt source is task (static taskId)', () => {
    render(
      <LaunchAgentConfigForm
        config={baseConfig({ taskId: 't1' })}
        onChange={vi.fn()}
        triggerType="manual"
      />
    )
    expect(agentPickerProps.at(-1)!.allowFromTask).toBe(true)
  })
})

describe('LaunchAgentConfigForm — auto-revert on context loss', () => {
  it('reverts agentType from "fromTask" to the default agent when canUseFromTask flips to false', async () => {
    const onChange = vi.fn()
    const { rerender } = render(
      <LaunchAgentConfigForm
        config={baseConfig({ agentType: 'fromTask' })}
        onChange={onChange}
        triggerType="taskStatusChanged"
      />
    )

    // Simulate the user clearing the task trigger — rerender with a
    // non-task trigger. The form's useEffect should fire onChange with a
    // concrete agent.
    rerender(
      <LaunchAgentConfigForm
        config={baseConfig({ agentType: 'fromTask' })}
        onChange={onChange}
        triggerType="manual"
      />
    )

    const reverted = onChange.mock.calls.find(
      ([c]: [LaunchAgentConfig]) => c.agentType !== 'fromTask'
    )
    expect(reverted).toBeDefined()
    const [nextConfig] = reverted as [LaunchAgentConfig]
    expect(nextConfig.agentType).toBe('claude')
  })

  it('does not revert when the user selects a concrete agent with a task trigger still active', () => {
    const onChange = vi.fn()
    render(
      <LaunchAgentConfigForm
        config={baseConfig({ agentType: 'codex' })}
        onChange={onChange}
        triggerType="taskStatusChanged"
      />
    )
    // Initial render must not call onChange (no revert needed).
    expect(onChange).not.toHaveBeenCalled()
  })
})

describe('LaunchAgentConfigForm — UI sections', () => {
  it('renders Agent, Project, Prompt, Execution sections', () => {
    const { container } = render(<LaunchAgentConfigForm config={baseConfig()} onChange={vi.fn()} />)
    const text = container.textContent ?? ''
    expect(text).toContain('Agent')
    expect(text).toContain('Project')
    expect(text).toContain('Prompt')
    expect(text).toContain('Execution')
  })

  it('shows queue hint when promptSource is queue', () => {
    const { container } = render(
      <LaunchAgentConfigForm
        config={baseConfig({ taskFromQueue: true, projectName: 'demo' })}
        onChange={vi.fn()}
      />
    )
    expect(container.textContent).toContain('Auto-picks the next todo task')
  })

  it('renders Tab Name input when not headless', () => {
    const { container } = render(
      <LaunchAgentConfigForm config={baseConfig({ headless: false })} onChange={vi.fn()} />
    )
    expect(container.textContent).toContain('Tab Name')
  })

  it('hides Tab Name input when headless', () => {
    const { container } = render(
      <LaunchAgentConfigForm config={baseConfig({ headless: true })} onChange={vi.fn()} />
    )
    expect(container.textContent).not.toContain('Tab Name')
  })

  it('toggles the headless mode when the switch is clicked', () => {
    const onChange = vi.fn()
    const { container } = render(
      <LaunchAgentConfigForm config={baseConfig({ headless: false })} onChange={onChange} />
    )
    const switchButton = container.querySelector('button[role="switch"]') as HTMLButtonElement
    if (switchButton) switchButton.click()
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ headless: true }))
  })

  it('renders the Advanced toggle', () => {
    const { container } = render(<LaunchAgentConfigForm config={baseConfig()} onChange={vi.fn()} />)
    const advancedToggle = Array.from(container.querySelectorAll('button')).find((b) =>
      b.textContent?.includes('Advanced')
    )
    expect(advancedToggle).toBeDefined()
  })

  it('opens Advanced section pre-expanded when args are present', () => {
    const { container } = render(
      <LaunchAgentConfigForm config={baseConfig({ args: ['--flag'] })} onChange={vi.fn()} />
    )
    expect(container.textContent).toContain('Extra Arguments')
  })

  it('renders Worktree picker with options when project is a git repo', async () => {
    const { container } = render(
      <LaunchAgentConfigForm config={baseConfig({ branch: 'feature/x' })} onChange={vi.fn()} />
    )
    await new Promise((r) => setTimeout(r, 0))
    expect(container.textContent).toContain('Worktree')
  })

  it('shows fromStep guidance text when worktree mode is fromStep', () => {
    const { container } = render(
      <LaunchAgentConfigForm
        config={baseConfig({ branch: 'feature/x', worktreeMode: 'fromStep' })}
        onChange={vi.fn()}
      />
    )
    expect(container.textContent).toContain('Reuses the worktree created')
  })

  it('shows existing-worktree guidance when worktree mode is existing', () => {
    const { container } = render(
      <LaunchAgentConfigForm
        config={baseConfig({ branch: 'feature/x', worktreeMode: 'existing' })}
        onChange={vi.fn()}
      />
    )
    expect(container.textContent).toContain('existing worktree on disk')
  })

  it('shows new-worktree guidance when worktree mode is new', () => {
    const { container } = render(
      <LaunchAgentConfigForm
        config={baseConfig({ branch: 'feature/x', worktreeMode: 'new' })}
        onChange={vi.fn()}
      />
    )
    expect(container.textContent).toContain('Isolated directory')
  })

  it('renders the branch input', async () => {
    const { container } = render(<LaunchAgentConfigForm config={baseConfig()} onChange={vi.fn()} />)
    await new Promise((r) => setTimeout(r, 0))
    expect(container.querySelector('input[placeholder="feature/my-branch"]')).toBeTruthy()
  })

  it('renders task selector when promptSource is task', () => {
    const { container } = render(
      <LaunchAgentConfigForm config={baseConfig({ taskId: 't1' })} onChange={vi.fn()} />
    )
    expect(container.textContent).toContain('Select task')
  })

  it('renders fromStep selector UI when worktree mode is fromStep', () => {
    const { container } = render(
      <LaunchAgentConfigForm
        config={baseConfig({ branch: 'feature/x', worktreeMode: 'fromStep' })}
        onChange={vi.fn()}
      />
    )
    expect(container.textContent).toContain('Select step')
  })

  it('renders existing-worktree selector UI when worktree mode is existing', () => {
    const { container } = render(
      <LaunchAgentConfigForm
        config={baseConfig({ branch: 'feature/x', worktreeMode: 'existing' })}
        onChange={vi.fn()}
      />
    )
    expect(container.textContent).toContain('Select worktree')
  })

  it('shows the agent type label and project name in headless mode', () => {
    const { container } = render(
      <LaunchAgentConfigForm config={baseConfig({ headless: true })} onChange={vi.fn()} />
    )
    expect(container.textContent).toContain('Headless')
  })
})

describe('LaunchAgentConfigForm — contextual workflow surface', () => {
  it('passes allowFromContext=true to ProjectPicker when trigger is contextual', () => {
    render(
      <LaunchAgentConfigForm
        config={baseConfig()}
        onChange={vi.fn()}
        triggerType="manual"
        isContextualTrigger
      />
    )
    expect(projectPickerProps.at(-1)!.allowFromContext).toBe(true)
  })

  it('passes allowFromContext=false when the trigger is not contextual', () => {
    render(<LaunchAgentConfigForm config={baseConfig()} onChange={vi.fn()} triggerType="manual" />)
    expect(projectPickerProps.at(-1)!.allowFromContext).toBe(false)
  })

  it('flags isFromContext on the ProjectPicker when projectName holds the sentinel', () => {
    render(
      <LaunchAgentConfigForm
        config={baseConfig({
          projectName: '{{context.projectName}}',
          projectPath: '{{context.projectPath}}'
        })}
        onChange={vi.fn()}
        triggerType="manual"
        isContextualTrigger
      />
    )
    expect(projectPickerProps.at(-1)!.isFromContext).toBe(true)
  })

  it('renders the From Context branch chip when branch holds the sentinel', () => {
    const { container } = render(
      <LaunchAgentConfigForm
        config={baseConfig({ branch: '{{context.branch}}' })}
        onChange={vi.fn()}
        triggerType="manual"
        isContextualTrigger
      />
    )
    expect(container.textContent).toContain('From Context')
  })

  it("describes 'From Context' worktree mode in the helper text", () => {
    const { container } = render(
      <LaunchAgentConfigForm
        config={baseConfig({ branch: 'feature/x', useWorktree: 'fromContext' })}
        onChange={vi.fn()}
        triggerType="manual"
        isContextualTrigger
      />
    )
    expect(container.textContent).toContain("won't be auto-cleaned")
  })

  it('clears From Context fields when the trigger flips off contextual', () => {
    const onChange = vi.fn()
    const { rerender } = render(
      <LaunchAgentConfigForm
        config={baseConfig({
          projectName: '{{context.projectName}}',
          projectPath: '{{context.projectPath}}',
          branch: '{{context.branch}}',
          useWorktree: 'fromContext'
        })}
        onChange={onChange}
        triggerType="manual"
        isContextualTrigger
      />
    )

    rerender(
      <LaunchAgentConfigForm
        config={baseConfig({
          projectName: '{{context.projectName}}',
          projectPath: '{{context.projectPath}}',
          branch: '{{context.branch}}',
          useWorktree: 'fromContext'
        })}
        onChange={onChange}
        triggerType="manual"
        isContextualTrigger={false}
      />
    )

    const reset = onChange.mock.calls.find(([c]: [LaunchAgentConfig]) => {
      return (
        c.projectName === '' &&
        c.projectPath === '' &&
        c.branch === undefined &&
        c.useWorktree === undefined
      )
    })
    expect(reset).toBeDefined()
  })

  it('exposes context vars to the prompt autocomplete when contextual', () => {
    // hasTemplateVars becomes true via isContextualTrigger which switches the
    // prompt slot to VariableAutocomplete (mocked) — the absence of a crash is
    // enough; the prior path renders RichMarkdownEditor.
    const { container } = render(
      <LaunchAgentConfigForm
        config={baseConfig()}
        onChange={vi.fn()}
        triggerType="manual"
        isContextualTrigger
      />
    )
    expect(container.querySelector('[data-testid="variable-autocomplete"]')).toBeTruthy()
  })
})
