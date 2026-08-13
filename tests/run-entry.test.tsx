// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.mock('framer-motion', () => ({
  motion: {
    div: ({ children, ...props }: React.PropsWithChildren<Record<string, unknown>>) => (
      <div {...props}>{children}</div>
    )
  },
  AnimatePresence: ({ children }: React.PropsWithChildren) => <>{children}</>
}))

vi.mock('react-dom', async () => {
  const actual = await vi.importActual<typeof import('react-dom')>('react-dom')
  return { ...actual, createPortal: (node: React.ReactNode) => node }
})

vi.mock('lucide-react', async (importOriginal) => ({
  ...(await importOriginal<typeof import('lucide-react')>()),
  ChevronDown: (p: Record<string, unknown>) => <svg data-testid="chev-down" {...p} />,
  ChevronRight: (p: Record<string, unknown>) => <svg data-testid="chev-right" {...p} />,
  Maximize2: (p: Record<string, unknown>) => <svg data-testid="maximize" {...p} />,
  RotateCcw: (p: Record<string, unknown>) => <svg data-testid="rotate-ccw" {...p} />,
  Square: (p: Record<string, unknown>) => <svg data-testid="square" {...p} />
}))

import type { TaskConfig } from '../src/shared/types'
import { RunEntry, RunStepsList } from '../src/renderer/components/workflow-editor/RunEntry'
import { __resetConnectionsCacheForTests } from '../src/renderer/lib/use-connections'
import type { WorkflowExecution, WorkflowNode, NodeExecutionState } from '../src/shared/types'

function makeExec(overrides: Partial<WorkflowExecution> = {}): WorkflowExecution {
  return {
    runId: 'run-1',
    workflowId: 'wf-1',
    startedAt: '2026-04-20T10:00:00Z',
    completedAt: '2026-04-20T10:00:05Z',
    status: 'success',
    nodeStates: [],
    ...overrides
  }
}

function makeNode(overrides: Partial<WorkflowNode> = {}): WorkflowNode {
  return {
    id: 'node-1',
    type: 'launchAgent',
    label: 'Run Claude',
    slug: 'run-claude',
    config: {
      agentType: 'claude',
      projectName: 'test',
      projectPath: '/test',
      branch: 'main',
      useWorktree: true,
      headless: true,
      prompt: 'hi'
    },
    position: { x: 0, y: 0 },
    ...overrides
  }
}

function makeState(overrides: Partial<NodeExecutionState> = {}): NodeExecutionState {
  return {
    nodeId: 'node-1',
    status: 'success',
    startedAt: '2026-04-20T10:00:01Z',
    completedAt: '2026-04-20T10:00:05Z',
    logs: 'some streaming logs',
    ...overrides
  }
}

describe('RunEntry', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-04-20T10:00:10Z'))
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  it('expands and shows the Resume button when agentSessionId is present', () => {
    const onResume = vi.fn()
    const exec = makeExec({ nodeStates: [makeState({ agentSessionId: 'agent-abc' })] })
    const { getByText, getByLabelText } = render(
      <RunEntry execution={exec} nodes={[makeNode()]} onResumeSession={onResume} />
    )
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)
    const resumeBtn = getByLabelText('Resume session')
    fireEvent.click(resumeBtn)
    expect(onResume).toHaveBeenCalledWith('agent-abc', 'claude', 'test', '/test', 'main', true)
  })

  it('reads a link to a task the same way wherever it appears', () => {
    // The chip on a step and the chip on the run header were blue and violet:
    // two colours for one kind of thing, a link to a task. They now share one
    // constant, and this is what keeps them from drifting apart again.
    const exec = makeExec({
      triggerTaskId: 'task-1',
      nodeStates: [makeState({ taskId: 'task-2' })]
    })
    const tasks = [
      { id: 'task-1', title: 'Triggering task' },
      { id: 'task-2', title: 'Created task' }
    ] as TaskConfig[]
    const { getByText, getByTitle } = render(
      <RunEntry execution={exec} nodes={[makeNode()]} tasks={tasks} />
    )
    const triggerChip = getByTitle('Triggering task')
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    const stepChip = getByTitle('Created task')

    const shared = (el: Element): string[] =>
      el.className.split(' ').filter((c) => !c.startsWith('max-w-') && c !== 'shrink-0')
    expect(shared(stepChip)).toEqual(shared(triggerChip))
  })

  it('calls onViewFullOutput with node logs', () => {
    const onView = vi.fn()
    const exec = makeExec({ nodeStates: [makeState({ agentSessionId: 'agent-abc' })] })
    const { getByText, getByLabelText } = render(
      <RunEntry
        execution={exec}
        nodes={[makeNode()]}
        onViewFullOutput={onView}
        onResumeSession={vi.fn()}
      />
    )
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)
    fireEvent.click(getByLabelText('View full output'))
    expect(onView).toHaveBeenCalledWith('some streaming logs')
  })

  it('uses the resolved agent/project captured in node state (fromTask sentinel)', () => {
    const onResume = vi.fn()
    const exec = makeExec({
      triggerTaskId: 'task-1',
      nodeStates: [
        makeState({
          agentSessionId: 'agent-abc',
          agentType: 'claude',
          projectName: 'from-task',
          projectPath: '/abs/from-task',
          taskId: 'task-1'
        })
      ]
    })
    const fromTaskNode = makeNode({
      config: {
        agentType: 'fromTask',
        projectName: '',
        projectPath: '',
        headless: true,
        prompt: 'hi'
      }
    })
    const { getByText, getByLabelText } = render(
      <RunEntry execution={exec} nodes={[fromTaskNode]} onResumeSession={onResume} />
    )
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)
    fireEvent.click(getByLabelText('Resume session'))
    expect(onResume).toHaveBeenCalledWith(
      'agent-abc',
      'claude',
      'from-task',
      '/abs/from-task',
      undefined,
      undefined
    )
  })

  it('hides Resume when project cannot be resolved (fromTask node without recorded state)', () => {
    const fromTaskNode = makeNode({
      config: {
        agentType: 'fromTask',
        projectName: '',
        projectPath: '',
        headless: true,
        prompt: 'hi'
      }
    })
    const exec = makeExec({ nodeStates: [makeState({ agentSessionId: 'agent-abc' })] })
    const { getByText, queryByLabelText } = render(
      <RunEntry execution={exec} nodes={[fromTaskNode]} onResumeSession={vi.fn()} />
    )
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)
    expect(queryByLabelText('Resume session')).not.toBeInTheDocument()
  })

  it('hides Resume button for unsupported agents (gemini)', () => {
    const exec = makeExec({ nodeStates: [makeState({ agentSessionId: 'agent-abc' })] })
    const geminiNode = makeNode({
      config: {
        agentType: 'gemini',
        projectName: 'test',
        projectPath: '/test',
        headless: true,
        prompt: 'hi'
      }
    })
    const { getByText, queryByLabelText } = render(
      <RunEntry execution={exec} nodes={[geminiNode]} onResumeSession={vi.fn()} />
    )
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)
    expect(queryByLabelText('Resume session')).not.toBeInTheDocument()
  })

  it('shows Resume button on error-only branch (no logs, just error)', () => {
    const onResume = vi.fn()
    const exec = makeExec({
      status: 'error',
      nodeStates: [
        makeState({
          status: 'error',
          agentSessionId: 'agent-abc',
          logs: undefined,
          error: 'boom'
        })
      ]
    })
    const { getByText, getByLabelText } = render(
      <RunEntry execution={exec} nodes={[makeNode()]} onResumeSession={onResume} />
    )
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)
    fireEvent.click(getByLabelText('Resume session'))
    expect(onResume).toHaveBeenCalled()
  })

  it('shows the running empty state when an expanded step has no output yet', () => {
    const exec = makeExec({
      status: 'running',
      completedAt: undefined,
      nodeStates: [makeState({ status: 'running', logs: undefined })]
    })
    const { getByText } = render(<RunEntry execution={exec} nodes={[makeNode()]} />)
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)
    expect(getByText(/No output captured yet/)).toBeInTheDocument()
  })

  it('shows engine notes instead of a dead end when the step produced no output', () => {
    // The failure this exists for: an agent that hung and wrote nothing. The
    // log is empty, so the timeline is the only thing left to read.
    const exec = makeExec({
      status: 'error',
      nodeStates: [
        makeState({
          status: 'error',
          logs: undefined,
          error:
            'Step timed out after 60 minutes. The agent was started but never produced any output.',
          diagnostics:
            '[+0.0s] Launching claude in /p\n[+0.4s] Session sess-1 started (pid 4242): claude --dangerously-skip-permissions -p'
        })
      ]
    })
    const { getByText, queryByText } = render(<RunEntry execution={exec} nodes={[makeNode()]} />)
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)

    expect(getByText(/Session sess-1 started \(pid 4242\)/)).toBeInTheDocument()
    expect(getByText(/Launching claude in \/p/)).toBeInTheDocument()
    expect(queryByText(/No output recorded/)).not.toBeInTheDocument()
  })

  it('shows engine notes and the agent log in one ordered timeline', () => {
    const exec = makeExec({
      nodeStates: [
        makeState({
          status: 'success',
          logs: 'agent said this',
          diagnostics:
            '[+0.0s] Launching claude in /p\n[+1.2s] First output from the agent (15 bytes)\n[+9.0s] Agent exited (code 0)'
        })
      ]
    })
    const { getByText, container } = render(<RunEntry execution={exec} nodes={[makeNode()]} />)
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)

    expect(getByText('agent said this')).toBeInTheDocument()
    expect(getByText(/Launching claude in \/p/)).toBeInTheDocument()

    // Setup before the agent, outcome after — the point of merging the panels.
    const text = container.textContent ?? ''
    expect(text.indexOf('Launching claude')).toBeLessThan(text.indexOf('agent said this'))
    expect(text.indexOf('agent said this')).toBeLessThan(text.indexOf('Agent exited'))
  })

  it("shows the pending empty state for a step that hasn't started", () => {
    const exec = makeExec({
      status: 'running',
      completedAt: undefined,
      nodeStates: [
        makeState({
          status: 'pending',
          logs: undefined,
          startedAt: undefined,
          completedAt: undefined
        })
      ]
    })
    const { getByText } = render(<RunEntry execution={exec} nodes={[makeNode()]} />)
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)
    expect(getByText(/Step hasn't started yet/)).toBeInTheDocument()
  })

  it('shows the skipped empty state for a skipped step', () => {
    const exec = makeExec({
      status: 'error',
      nodeStates: [makeState({ status: 'skipped', logs: undefined })]
    })
    const { getByText } = render(<RunEntry execution={exec} nodes={[makeNode()]} />)
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)
    expect(getByText(/Step was skipped/)).toBeInTheDocument()
  })
})

describe('RunEntry — run inputs', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-04-20T10:00:10Z'))
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  function expand(getByText: (m: RegExp) => HTMLElement): void {
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
  }

  it('shows what the run was launched with', () => {
    const exec = makeExec({
      nodeStates: [makeState()],
      inputs: { issue: 'gh-42', force: false }
    })
    const { getByText } = render(<RunEntry execution={exec} nodes={[makeNode()]} />)
    expand(getByText)

    expect(getByText('gh-42')).toBeInTheDocument()
    expect(getByText('false')).toBeInTheDocument()
  })

  it('labels the row and pairs each key with its own value chip', () => {
    const exec = makeExec({
      nodeStates: [makeState()],
      inputs: { issue: 'gh-42', force: false }
    })
    const { getByText } = render(<RunEntry execution={exec} nodes={[makeNode()]} />)
    expand(getByText)

    expect(getByText('Inputs')).toBeInTheDocument()
    // Each input is one chip carrying both halves, so a key can never be read
    // against a neighbouring value.
    const chip = getByText('issue').closest('span[title]')!
    expect(chip).toHaveAttribute('title', 'issue=gh-42')
    expect(chip.textContent).toBe('issuegh-42')
  })

  it('clips a large object-valued input instead of flooding the row', () => {
    const exec = makeExec({
      nodeStates: [makeState()],
      inputs: { item: { body: 'x'.repeat(200) } }
    })
    const { getByText } = render(<RunEntry execution={exec} nodes={[makeNode()]} />)
    expand(getByText)

    const rendered = getByText(/^\{"body"/)
    expect(rendered.textContent!.length).toBeLessThanOrEqual(61)
    expect(rendered.textContent!.endsWith('…')).toBe(true)
  })

  it('renders no inputs row for a run that had none', () => {
    const exec = makeExec({ nodeStates: [makeState()] })
    const { getByText, queryByText } = render(<RunEntry execution={exec} nodes={[makeNode()]} />)
    expand(getByText)

    // Queried the same way as the positive case above: the row renders the
    // value in its own node, so a regex over `key=value` would match nothing
    // even when the row *is* present and the assertion could never fail.
    expect(queryByText('gh-42')).not.toBeInTheDocument()
    expect(queryByText('issue')).not.toBeInTheDocument()
  })
})

describe('RunStepsList — step icons', () => {
  beforeEach(() => {
    __resetConnectionsCacheForTests()
  })
  afterEach(() => {
    __resetConnectionsCacheForTests()
    // @ts-expect-error - clearing the per-test window.api stub
    delete window.api
  })

  const triggerNode = makeNode({
    id: 'trig',
    type: 'trigger',
    label: 'GitHub Trigger',
    config: { triggerType: 'connectorPoll', connectionId: 'conn-1', event: 'prOpened', cron: '*' }
  })
  const scriptNode = makeNode({
    id: 'scr',
    type: 'script',
    label: 'Execute Script',
    config: { scriptType: 'bash', scriptContent: 'echo hi', projectName: 'vorn' }
  })
  const exec = makeExec({
    nodeStates: [makeState({ nodeId: 'trig', logs: undefined }), makeState({ nodeId: 'scr' })]
  })

  function stubConnections(connections: unknown[]) {
    // @ts-expect-error - minimal window.api stub for this test
    window.api = { listConnections: vi.fn().mockResolvedValue(connections) }
  }

  it("shows a connector-bound step under its connector's brand mark", async () => {
    stubConnections([{ id: 'conn-1', connectorId: 'github', name: 'GitHub' }])

    const { container } = render(
      <RunStepsList execution={exec} nodes={[triggerNode, scriptNode]} includeTrigger />
    )
    await waitFor(() =>
      expect(container.querySelector('svg[viewBox="0 0 16 16"]')).toBeInTheDocument()
    )
  })

  it('describes each step and previews what it was configured to run', async () => {
    stubConnections([{ id: 'conn-1', connectorId: 'github', name: 'GitHub' }])

    const { getByText, findByText } = render(
      <RunStepsList execution={exec} nodes={[triggerNode, scriptNode]} includeTrigger />
    )
    // The trigger names the connector it polls and the event it listens for.
    expect(await findByText('github · prOpened')).toBeInTheDocument()
    // The script names its shell and project.
    expect(getByText('bash · vorn')).toBeInTheDocument()
    // With no output of its own, the trigger falls back to its configured
    // body rather than leaving a blank card.
    expect(getByText('on: prOpened')).toBeInTheDocument()
  })

  it("previews the opening of a step's output and expands to the full log", async () => {
    stubConnections([])
    const noisy = makeExec({
      nodeStates: [makeState({ nodeId: 'scr', logs: 'installing deps\nrunning tests\nall green' })]
    })

    const { getByText, queryByText, getByLabelText, findByText } = render(
      <RunStepsList execution={noisy} nodes={[scriptNode]} />
    )
    // The card opens the log where the log itself opens.
    expect(await findByText('installing deps')).toBeInTheDocument()
    expect(queryByText(/all green/)).not.toBeInTheDocument()

    fireEvent.click(getByLabelText('Show full output of step 1'))
    expect(getByText(/all green/)).toBeInTheDocument()
  })

  it('falls back to the node-type icon for a step with no connection', async () => {
    stubConnections([])

    const { container } = render(
      <RunStepsList execution={exec} nodes={[triggerNode, scriptNode]} includeTrigger />
    )
    await waitFor(() => expect(container.querySelector('svg.lucide-terminal')).toBeInTheDocument())
    // The connector cache resolved to nothing, so the trigger keeps its own
    // node-type glyph rather than borrowing a brand mark.
    expect(container.querySelector('svg[viewBox="0 0 16 16"]')).not.toBeInTheDocument()
  })
})
