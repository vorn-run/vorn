import { describe, it, expect } from 'vitest'
import {
  buildConnectorSeededWorkflow,
  cronEveryMinutes
} from '../packages/server/src/default-workflows'
import {
  connectorSeededWorkflowId,
  type SourceConnection,
  type ConnectorManifest,
  type ConnectorPollTriggerConfig,
  type CreateTaskFromItemConfig
} from '../packages/shared/src/types'

const CONN: SourceConnection = {
  id: 'conn-abc',
  connectorId: 'github',
  name: 'owner/repo',
  filters: { owner: 'owner', repo: 'repo' },
  syncIntervalMinutes: 5,
  statusMapping: {},
  createdAt: '2026-04-24T00:00:00Z'
}

const MANIFEST: ConnectorManifest = {
  auth: [],
  triggers: [
    {
      type: 'issueCreated',
      label: 'Issue Created',
      configFields: [],
      defaultIntervalMs: 60_000
    }
  ],
  statusMapping: [{ upstream: 'open', suggestedLocal: 'todo' }],
  defaultWorkflows: [
    {
      name: 'GitHub: Issue Created',
      event: 'issueCreated',
      defaultCronFromMinutes: 5,
      downstream: 'createTaskFromItem'
    }
  ]
}

describe('buildConnectorSeededWorkflow', () => {
  it('builds a stable id tied to the (connection, event) pair', () => {
    const wf = buildConnectorSeededWorkflow(CONN, MANIFEST, MANIFEST.defaultWorkflows![0])
    expect(wf.id).toBe(connectorSeededWorkflowId(CONN.id, 'issueCreated'))
  })

  it('seeds as enabled in the personal workspace', () => {
    const wf = buildConnectorSeededWorkflow(CONN, MANIFEST, MANIFEST.defaultWorkflows![0])
    expect(wf.enabled).toBe(true)
    expect(wf.workspaceId).toBe('personal')
  })

  it('uses the manifest event name for the workflow display name', () => {
    const wf = buildConnectorSeededWorkflow(CONN, MANIFEST, MANIFEST.defaultWorkflows![0])
    expect(wf.name).toBe('GitHub: Issue Created')
  })

  it('derives every-N-minute cron from defaultCronFromMinutes', () => {
    const wf = buildConnectorSeededWorkflow(CONN, MANIFEST, MANIFEST.defaultWorkflows![0])
    const trigger = wf.nodes.find((n) => n.type === 'trigger')!.config as ConnectorPollTriggerConfig
    expect(trigger.cron).toBe('*/5 * * * *')
  })

  it('uses the minute-wildcard cron when interval is 1', () => {
    const event = { ...MANIFEST.defaultWorkflows![0], defaultCronFromMinutes: 1 }
    const wf = buildConnectorSeededWorkflow(CONN, MANIFEST, event)
    const trigger = wf.nodes.find((n) => n.type === 'trigger')!.config as ConnectorPollTriggerConfig
    expect(trigger.cron).toBe('* * * * *')
  })

  it('floors zero / negative intervals to 1 minute rather than emitting invalid cron', () => {
    const event = { ...MANIFEST.defaultWorkflows![0], defaultCronFromMinutes: 0 }
    const wf = buildConnectorSeededWorkflow(CONN, MANIFEST, event)
    const trigger = wf.nodes.find((n) => n.type === 'trigger')!.config as ConnectorPollTriggerConfig
    expect(trigger.cron).toBe('* * * * *')
  })

  it('wires the connectorPoll trigger directly to the createTaskFromItem node', () => {
    const wf = buildConnectorSeededWorkflow(CONN, MANIFEST, MANIFEST.defaultWorkflows![0])
    expect(wf.nodes).toHaveLength(2)
    expect(wf.edges).toHaveLength(1)

    const trigger = wf.nodes.find((n) => n.type === 'trigger')!
    const taskNode = wf.nodes.find((n) => n.type === 'createTaskFromItem')!
    expect(wf.edges[0].source).toBe(trigger.id)
    expect(wf.edges[0].target).toBe(taskNode.id)
  })

  it('seeds the trigger with connectionId + event + triggerType=connectorPoll', () => {
    const wf = buildConnectorSeededWorkflow(CONN, MANIFEST, MANIFEST.defaultWorkflows![0])
    const trigger = wf.nodes.find((n) => n.type === 'trigger')!.config as ConnectorPollTriggerConfig
    expect(trigger.triggerType).toBe('connectorPoll')
    expect(trigger.connectionId).toBe(CONN.id)
    expect(trigger.event).toBe('issueCreated')
  })

  it('initialStatus falls back to todo when manifest has no statusMapping', () => {
    const manifest = { ...MANIFEST, statusMapping: undefined }
    const wf = buildConnectorSeededWorkflow(CONN, manifest, MANIFEST.defaultWorkflows![0])
    const node = wf.nodes.find((n) => n.type === 'createTaskFromItem')!
      .config as CreateTaskFromItemConfig
    expect(node.initialStatus).toBe('todo')
  })

  it('initialStatus uses the first suggestedLocal when statusMapping is present', () => {
    const manifest: ConnectorManifest = {
      ...MANIFEST,
      statusMapping: [
        { upstream: 'in_progress', suggestedLocal: 'in_progress' },
        { upstream: 'open', suggestedLocal: 'todo' }
      ]
    }
    const wf = buildConnectorSeededWorkflow(CONN, manifest, MANIFEST.defaultWorkflows![0])
    const node = wf.nodes.find((n) => n.type === 'createTaskFromItem')!
      .config as CreateTaskFromItemConfig
    expect(node.initialStatus).toBe('in_progress')
  })

  it('createTaskFromItem node defaults project to fromConnection', () => {
    const wf = buildConnectorSeededWorkflow(CONN, MANIFEST, MANIFEST.defaultWorkflows![0])
    const node = wf.nodes.find((n) => n.type === 'createTaskFromItem')!
      .config as CreateTaskFromItemConfig
    expect(node.project).toBe('fromConnection')
  })
})

describe('cronEveryMinutes', () => {
  it('steps the minute field below an hour', () => {
    expect(cronEveryMinutes(1)).toBe('* * * * *')
    expect(cronEveryMinutes(5)).toBe('*/5 * * * *')
    expect(cronEveryMinutes(59)).toBe('*/59 * * * *')
  })

  it('moves to the hour field at an hour, rather than emitting an invalid cron', () => {
    // The minute field only counts to 59, so `*/60 * * * *` is not hourly — it
    // is unschedulable, and a connector suggesting an hour would have been
    // seeded with a workflow that never fires.
    expect(cronEveryMinutes(60)).toBe('0 * * * *')
    expect(cronEveryMinutes(120)).toBe('0 */2 * * *')
    expect(cronEveryMinutes(720)).toBe('0 */12 * * *')
  })

  it('rounds an interval that does not divide into hours', () => {
    expect(cronEveryMinutes(90)).toBe('0 */2 * * *')
    expect(cronEveryMinutes(70)).toBe('0 * * * *')
  })

  it('treats a day as a day rather than every 23 hours', () => {
    // The hour field counts 0-23, so a 24-step never fires. Capping to 23
    // instead would drift a full hour earlier every day, which is not what
    // anybody asking for "daily" means.
    expect(cronEveryMinutes(1440)).toBe('0 0 * * *')
    expect(cronEveryMinutes(1439)).toBe('0 0 * * *')
    expect(cronEveryMinutes(23 * 60)).toBe('0 */23 * * *')
  })
})
