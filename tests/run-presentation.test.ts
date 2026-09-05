import { WORKFLOW_STATUS_DOT } from '../src/renderer/lib/workflow-status'
import { describe, it, expect } from 'vitest'
import {
  bucketOf,
  completedStageCount,
  describeOutcome,
  describeRun,
  ranUninterrupted,
  runStages,
  liveNodeStatus,
  runSummaryText
} from '../src/renderer/lib/run-presentation'
import type {
  ConnectorItemContext,
  NodeExecutionState,
  WorkflowExecution,
  WorkflowNode
} from '../src/shared/types'

function node(id: string, type: WorkflowNode['type'], label: string, config = {}): WorkflowNode {
  return { id, type, label, config, position: { x: 0, y: 0 } } as WorkflowNode
}

function run(overrides: Partial<WorkflowExecution> = {}): WorkflowExecution {
  return {
    runId: 'run-1',
    workflowId: 'wf-1',
    startedAt: '2026-04-20T11:59:00Z',
    status: 'success',
    nodeStates: [],
    ...overrides
  } as WorkflowExecution
}

function githubItem(overrides: Partial<ConnectorItemContext> = {}): ConnectorItemContext {
  return {
    connectionId: 'conn-1',
    connectorId: 'github',
    externalId: '309',
    title: 'refactor: split workflow runs panel into list + detail',
    raw: {},
    ...overrides
  } as ConnectorItemContext
}

describe('describeRun', () => {
  it('titles a GitHub pull-request run as "PR #<n>" and keeps the item title as the subtitle', () => {
    const p = describeRun(
      run({
        connectorItem: githubItem({
          externalUrl: 'https://github.com/vorn-run/vorn/pull/309'
        })
      }),
      { name: 'GitHub: PR Opened', nodes: [] }
    )
    expect(p.title).toBe('PR #309')
    expect(p.subtitle).toBe('refactor: split workflow runs panel into list + detail')
    expect(p.source).toBe('connector')
    expect(p.connectorId).toBe('github')
  })

  it('titles a GitHub issue run as "Issue #<n>"', () => {
    const p = describeRun(
      run({
        connectorItem: githubItem({
          externalId: '84',
          externalUrl: 'https://github.com/vorn-run/vorn/issues/84'
        })
      })
    )
    expect(p.title).toBe('Issue #84')
  })

  // Previously this guessed "Issue #84", inferred from the connector id. The
  // guess is gone with the id test: a packaged connector reports `mcp`, so the
  // inference would have been wrong precisely when it was invisible. Without a
  // url there is nothing that distinguishes an issue from a pull request, and
  // naming it generically is better than naming it confidently wrong.
  it('falls back to the generic form when a GitHub item carries no url', () => {
    const p = describeRun(run({ connectorItem: githubItem({ externalId: '84' }) }))
    expect(p.title).toBe('github 84')
  })

  // The port's whole point: the same GitHub items arriving through a packaged
  // connector, whose connection is stored as `mcp`. Titles must survive that.
  it('still names a PR when the item arrives from a packaged connector', () => {
    const p = describeRun(
      run({
        connectorItem: githubItem({
          connectorId: 'mcp',
          externalUrl: 'https://github.com/vorn-run/vorn/pull/309'
        })
      })
    )
    expect(p.title).toBe('PR #309')
  })

  it('still names an issue when the item arrives from a packaged connector', () => {
    const p = describeRun(
      run({
        connectorItem: githubItem({
          connectorId: 'mcp',
          externalId: '84',
          externalUrl: 'https://github.com/vorn-run/vorn/issues/84'
        })
      })
    )
    expect(p.title).toBe('Issue #84')
  })

  it('falls back to the connector id for connectors with no special-cased title', () => {
    const p = describeRun(
      run({ connectorItem: githubItem({ connectorId: 'linear', externalId: 'ENG-12' }) })
    )
    expect(p.title).toBe('linear ENG-12')
    expect(p.connectorId).toBe('linear')
  })

  // A packaged connector's item only knows itself as `mcp`; the connection it
  // came from is what says which connector really ran and what mark to draw.
  it('takes the connector id and glyph from the connection when one is resolved', () => {
    const icon = { viewBox: '0 0 24 24', paths: ['M2 2h9v9z'] }
    const p = describeRun(
      run({ connectorItem: githubItem({ connectorId: 'mcp', externalId: '7' }) }),
      undefined,
      { connectorId: 'packdemo', icon, packaged: true }
    )
    expect(p.connectorId).toBe('packdemo')
    expect(p.sourceLabel).toBe('packdemo')
    expect(p.connectorIcon).toBe(icon)
    expect(p.title).toBe('packdemo 7')
  })

  it('keeps the item id when no connection resolves, so a deleted one still reads', () => {
    const p = describeRun(run({ connectorItem: githubItem({ externalId: '7' }) }))
    expect(p.connectorId).toBe('github')
    expect(p.connectorIcon).toBeUndefined()
  })

  it('labels a task-triggered run with the workflow name and a short task subtitle', () => {
    const p = describeRun(run({ triggerTaskId: 'fa369a1234' }), {
      name: 'Apply changes',
      nodes: []
    })
    expect(p.title).toBe('Apply changes')
    expect(p.subtitle).toBe('Task fa369a')
    expect(p.source).toBe('task')
  })

  it('names a task-triggered run by its task when the workflow name is gone', () => {
    const p = describeRun(run({ triggerTaskId: 'fa369a1234' }))
    expect(p.title).toBe('Task fa369a')
  })

  it('reads the source from the trigger node for a plain run', () => {
    const nodes = [
      node('t', 'trigger', 'Schedule', { triggerType: 'recurring', cron: '* * * * *' })
    ]
    const p = describeRun(run(), { name: 'clean branches', nodes })
    expect(p.title).toBe('clean branches')
    expect(p.source).toBe('schedule')
    expect(p.sourceLabel).toBe('scheduled')
  })

  it('defaults to manual when the workflow has no trigger node', () => {
    const p = describeRun(run(), { name: 'Simple hello', nodes: [] })
    expect(p.source).toBe('manual')
    expect(p.sourceLabel).toBe('manual')
  })

  it('falls back to the short workflow id when nothing names the run', () => {
    const p = describeRun(run({ workflowId: '407f59ea-1234' }))
    expect(p.title).toBe('407f59ea')
  })

  it("carries the workflow's own icon and colour so a run is recognisable at a glance", () => {
    const p = describeRun(run({ connectorItem: githubItem({}) }), {
      name: 'GitHub: PR Opened',
      icon: 'github',
      iconColor: '#8b5cf6',
      nodes: []
    })
    expect(p.iconName).toBe('github')
    expect(p.iconColor).toBe('#8b5cf6')
  })

  it('leaves the icon unset when the workflow is gone, so a fallback is drawn', () => {
    const p = describeRun(run())
    expect(p.iconName).toBeUndefined()
    expect(p.fallbackIcon).toBeTruthy()
  })
})

describe('runStages', () => {
  const nodes = [
    node('t', 'trigger', 'Manual Trigger'),
    node('a', 'script', 'Execute Script'),
    node('b', 'launchAgent', 'Say Hello')
  ]

  it('includes the trigger and orders stages by the workflow definition', () => {
    const stages = runStages(
      run({
        nodeStates: [
          { nodeId: 'b', status: 'success' },
          { nodeId: 't', status: 'success' },
          { nodeId: 'a', status: 'success' }
        ] as NodeExecutionState[]
      }),
      nodes
    )
    expect(stages.map((s) => s.label)).toEqual(['Manual Trigger', 'Execute Script', 'Say Hello'])
  })

  it('falls back to the pending dot for a status it does not know', () => {
    // A status added on the server before the renderer learns about it would
    // otherwise render a segment with no class at all — an invisible stage,
    // which reads as a shorter run rather than an unknown one.
    const stages = runStages(
      run({
        nodeStates: [{ nodeId: 'a', status: 'teleported' }] as unknown as NodeExecutionState[]
      }),
      nodes
    )
    expect(stages[0].dotClass).toBe(WORKFLOW_STATUS_DOT.pending)
  })

  it('falls back to a short node id when the workflow is gone', () => {
    const stages = runStages(
      run({ nodeStates: [{ nodeId: 'abcdef123456', status: 'success' }] as NodeExecutionState[] }),
      []
    )
    expect(stages[0].label).toBe('abcdef12')
  })

  it('counts only terminal stages as complete', () => {
    const stages = runStages(
      run({
        nodeStates: [
          { nodeId: 't', status: 'success' },
          { nodeId: 'a', status: 'skipped' },
          { nodeId: 'b', status: 'running' }
        ] as NodeExecutionState[]
      }),
      nodes
    )
    expect(completedStageCount(stages)).toBe(2)
  })
})

describe('describeOutcome', () => {
  const approval = node('gate', 'approval', 'Review', { message: 'recommends merge' })

  it('prefers a waiting gate over the run status and uses the gate message', () => {
    const outcome = describeOutcome(
      run({
        status: 'running',
        nodeStates: [{ nodeId: 'gate', status: 'waiting' }] as NodeExecutionState[]
      }),
      [approval]
    )
    expect(outcome).toEqual({ label: 'recommends merge', tone: 'waiting' })
  })

  it('falls back to "needs review" for a gate with no message', () => {
    const outcome = describeOutcome(
      run({
        status: 'running',
        nodeStates: [{ nodeId: 'gate', status: 'waiting' }] as NodeExecutionState[]
      }),
      [node('gate', 'approval', 'Review')]
    )
    expect(outcome.label).toBe('needs review')
  })

  it('maps run statuses to human labels', () => {
    expect(describeOutcome(run({ status: 'running' }), []).label).toBe('in progress')
    expect(describeOutcome(run({ status: 'error' }), []).label).toBe('run failed')
    expect(describeOutcome(run({ status: 'cancelled' }), []).label).toBe('stopped')
    expect(describeOutcome(run({ status: 'success' }), []).label).toBe('completed')
  })

  it("prefers a finished run's structured verdict over the generic label", () => {
    const outcome = describeOutcome(
      run({
        nodeStates: [
          { nodeId: 'a', status: 'success' },
          { nodeId: 'b', status: 'success', structuredOutput: { verdict: 'recommends merge' } }
        ] as NodeExecutionState[]
      }),
      []
    )
    expect(outcome).toEqual({ label: 'recommends merge', tone: 'success' })
  })

  it('ignores a structured field too long to be a verdict', () => {
    const outcome = describeOutcome(
      run({
        nodeStates: [
          { nodeId: 'b', status: 'success', structuredOutput: { summary: 'x'.repeat(200) } }
        ] as NodeExecutionState[]
      }),
      []
    )
    expect(outcome.label).toBe('completed')
  })
})

describe('ranUninterrupted', () => {
  it('is true when no step waited or was approved', () => {
    expect(
      ranUninterrupted(
        run({ nodeStates: [{ nodeId: 'a', status: 'success' }] as NodeExecutionState[] })
      )
    ).toBe(true)
  })

  it('is false once a gate was approved', () => {
    expect(
      ranUninterrupted(
        run({
          nodeStates: [
            { nodeId: 'a', status: 'success', approvedAt: '2026-04-20T12:00:00Z' }
          ] as NodeExecutionState[]
        })
      )
    ).toBe(false)
  })
})

describe('runSummaryText', () => {
  it('prefers the logs of the step that is still running', () => {
    const text = runSummaryText(
      run({
        nodeStates: [
          { nodeId: 'a', status: 'success', logs: 'old output' },
          { nodeId: 'b', status: 'running', logs: 'scanning 14 local branches…' }
        ] as NodeExecutionState[]
      })
    )
    expect(text).toBe('scanning 14 local branches…')
  })

  it('falls back to the last step that produced anything', () => {
    const text = runSummaryText(
      run({
        nodeStates: [
          { nodeId: 'a', status: 'success', logs: 'first' },
          { nodeId: 'b', status: 'success', logs: 'last' }
        ] as NodeExecutionState[]
      })
    )
    expect(text).toBe('last')
  })

  it('uses the error when a failed step logged nothing', () => {
    const text = runSummaryText(
      run({ nodeStates: [{ nodeId: 'a', status: 'error', error: 'boom' }] as NodeExecutionState[] })
    )
    expect(text).toBe('boom')
  })

  it('clips a long log to its tail', () => {
    const text = runSummaryText(
      run({
        nodeStates: [
          { nodeId: 'a', status: 'success', logs: 'y'.repeat(2000) }
        ] as NodeExecutionState[]
      })
    )
    expect(text?.startsWith('…')).toBe(true)
    expect(text!.length).toBeLessThan(700)
  })

  it('returns undefined when nothing produced output', () => {
    expect(
      runSummaryText(
        run({ nodeStates: [{ nodeId: 'a', status: 'pending' }] as NodeExecutionState[] })
      )
    ).toBeUndefined()
  })
})

describe('bucketOf', () => {
  it('buckets a paused run as waiting rather than running', () => {
    expect(
      bucketOf(
        run({
          status: 'running',
          nodeStates: [
            { nodeId: 'a', status: 'success' },
            { nodeId: 'gate', status: 'waiting' }
          ] as NodeExecutionState[]
        })
      )
    ).toBe('waiting')
  })

  it('buckets an unpaused in-flight run as running', () => {
    expect(
      bucketOf(
        run({
          status: 'running',
          nodeStates: [{ nodeId: 'a', status: 'running' }] as NodeExecutionState[]
        })
      )
    ).toBe('running')
  })

  it('buckets success as success and everything else terminal as error', () => {
    expect(bucketOf(run({ status: 'success' }))).toBe('success')
    expect(bucketOf(run({ status: 'error' }))).toBe('error')
    expect(bucketOf(run({ status: 'cancelled' }))).toBe('error')
  })
})

describe('liveNodeStatus', () => {
  const exec = (over: Partial<WorkflowExecution>): WorkflowExecution =>
    run({ workflowId: 'wf-1', status: 'running', ...over })

  it('reports what each node of a running workflow is doing', () => {
    const map = liveNodeStatus(
      [
        exec({
          nodeStates: [
            { nodeId: 'a', status: 'success' },
            { nodeId: 'b', status: 'running' }
          ] as NodeExecutionState[]
        })
      ],
      'wf-1'
    )
    expect(map).toEqual({ a: 'success', b: 'running' })
  })

  it('lets the gate win when two runs disagree about a node', () => {
    // Runs go in parallel, so one node can be finished in one and parked on an
    // approval in another. The one that needs a person is the one worth showing.
    const map = liveNodeStatus(
      [
        exec({
          runId: 'r1',
          nodeStates: [{ nodeId: 'a', status: 'success' }] as NodeExecutionState[]
        }),
        exec({
          runId: 'r2',
          nodeStates: [{ nodeId: 'a', status: 'waiting' }] as NodeExecutionState[]
        })
      ],
      'wf-1'
    )
    expect(map).toEqual({ a: 'waiting' })
  })

  it('ranks running above error above success', () => {
    const map = liveNodeStatus(
      [
        exec({
          runId: 'r1',
          nodeStates: [{ nodeId: 'a', status: 'success' }] as NodeExecutionState[]
        }),
        exec({
          runId: 'r2',
          nodeStates: [{ nodeId: 'a', status: 'error' }] as NodeExecutionState[]
        }),
        exec({
          runId: 'r3',
          nodeStates: [{ nodeId: 'a', status: 'running' }] as NodeExecutionState[]
        })
      ],
      'wf-1'
    )
    expect(map).toEqual({ a: 'running' })
  })

  it('ignores runs of other workflows', () => {
    const map = liveNodeStatus(
      [
        exec({
          workflowId: 'wf-2',
          nodeStates: [{ nodeId: 'a', status: 'running' }] as NodeExecutionState[]
        })
      ],
      'wf-1'
    )
    expect(map).toBeUndefined()
  })

  it('forgets a run once it finishes', () => {
    // Finished runs stay in the store to back the history list. Reading their
    // node states too would leave the last run's dots on the canvas forever,
    // and the canvas could never go back to showing a plain definition.
    const stillReported = (['success', 'error', 'cancelled'] as const).filter(
      (status) =>
        liveNodeStatus(
          [
            exec({
              status,
              nodeStates: [{ nodeId: 'a', status: 'success' }] as NodeExecutionState[]
            })
          ],
          'wf-1'
        ) !== undefined
    )

    expect(stillReported).toEqual([])
  })

  it('says nothing rather than an empty map when nothing is live', () => {
    // The canvas takes undefined to mean "this is a definition, not a run", so
    // an empty object here would still be a truthy claim that a run exists.
    expect(liveNodeStatus([], 'wf-1')).toBeUndefined()
    expect(
      liveNodeStatus(
        [exec({ nodeStates: [{ nodeId: 'a', status: 'pending' }] as NodeExecutionState[] })],
        'wf-1'
      )
    ).toBeUndefined()
  })
})

describe('a run started by a session coming back', () => {
  it('names restore, whether it was cold or warm, and the session', () => {
    const nodes = [node('t', 'trigger', 'Restored', { triggerType: 'sessionRestored' })]
    const p = describeRun(
      run({
        triggerSession: { id: 'abc123def', label: 'attach what you can see', restore: 'cold' }
      }),
      { name: 'Bring the dev server back', nodes }
    )
    expect(p.title).toBe('Bring the dev server back')
    expect(p.subtitle).toBe('restore · cold · attach what you can see')
    expect(p.source).toBe('restore')
    expect(p.sourceLabel).toBe('restore')
  })

  it('reads the source from the trigger node when the run predates the session field', () => {
    const nodes = [node('t', 'trigger', 'Restored', { triggerType: 'sessionRestored' })]
    expect(describeRun(run(), { name: 'x', nodes }).source).toBe('restore')
  })
})
