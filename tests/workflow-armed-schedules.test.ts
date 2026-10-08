import { describe, it, expect } from 'vitest'
import type { WorkflowDefinition } from '../packages/shared/src/types'
import { armedScheduleCount } from '../packages/server/src/workflow-triggers'

describe('the schedules that keep an idle server up', () => {
  const wf = (config: Record<string, unknown>, enabled = true): WorkflowDefinition =>
    ({
      id: 'w',
      name: 'w',
      enabled,
      nodes: [{ id: 't', type: 'trigger', config }],
      edges: []
    }) as unknown as WorkflowDefinition

  it('counts enabled crons, polls and once triggers still ahead', () => {
    const now = Date.parse('2030-01-01T00:00:00Z')
    expect(
      armedScheduleCount(
        [
          wf({ triggerType: 'recurring', cron: '* * * * *' }),
          wf({ triggerType: 'connectorPoll', cron: '*/5 * * * *' }),
          wf({ triggerType: 'once', runAt: '2030-01-02T00:00:00Z' }),
          wf({ triggerType: 'once', runAt: '2029-01-01T00:00:00Z' }),
          wf({ triggerType: 'recurring', cron: ' ' }),
          wf({ triggerType: 'recurring', cron: '* * * * *' }, false),
          wf({ triggerType: 'manual' })
        ],
        now
      )
    ).toBe(3)
  })
})
