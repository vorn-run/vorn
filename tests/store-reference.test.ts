import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

import * as store from '../packages/server/src/database'
import type {
  AppConfig,
  ConnectorItemContext,
  ProjectConfig,
  SourceConnection,
  TaskConfig,
  WorkflowDefinition,
  WorkflowExecution
} from '@vornrun/shared/types'
import { normalizeStoreOutput, withoutMachineValues } from './helpers/store-parity'
import storeReference from './fixtures/js-reference/store.json'

afterEach(() => {
  store.closeDatabase()
})

const project: ProjectConfig = {
  name: 'proj',
  path: '/tmp/proj',
  preferredAgents: ['claude'],
  icon: 'Folder',
  hostIds: ['local'],
  workspaceId: 'personal'
}

const task = (id: string, order: number): TaskConfig => ({
  id,
  projectName: 'proj',
  title: `Task ${id}`,
  description: 'body',
  status: 'todo',
  order,
  createdAt: '2026-10-01T00:00:00.000Z',
  updatedAt: '2026-10-01T00:00:00.000Z'
})

const workflow = (id: string): WorkflowDefinition => ({
  id,
  name: id,
  icon: 'Plug',
  iconColor: '#fff',
  enabled: true,
  nodes: [],
  edges: []
})

const connection: SourceConnection = {
  id: 'conn-1',
  connectorId: 'github',
  name: 'owner/repo',
  filters: { owner: 'owner', repo: 'repo' },
  syncIntervalMinutes: 5,
  statusMapping: {},
  createdAt: '2026-10-01T00:00:00.000Z'
}

const item = (externalId: string): ConnectorItemContext => ({
  connectionId: connection.id,
  connectorId: connection.connectorId,
  externalId,
  title: `Item ${externalId}`,
  raw: { externalId }
})

const run = (runId: string, status: WorkflowExecution['status']): WorkflowExecution => ({
  workflowId: 'wf-1',
  runId,
  startedAt: '2026-10-01T10:00:00.000Z',
  status,
  triggerTaskId: 't1',
  nodeStates: [
    { nodeId: 'n1', status: 'success', output: 'ok', agentType: 'claude' },
    { nodeId: 'gate', status: 'waiting', waitingFor: 'signIn' }
  ]
})

/**
 * Every call the store answers, in an order where each sees what the ones
 * before it wrote. Returns what each call returned, by label.
 */
function scenario(): Array<[string, unknown]> {
  const out: Array<[string, unknown]> = []
  const step = (label: string, value: unknown): void => {
    out.push([label, value])
  }

  // Config
  const loaded = store.loadConfig()
  step('loadConfig fresh', loaded)
  const config: AppConfig = {
    ...loaded,
    defaults: { ...loaded.defaults, fontSize: 15, experimental: { nativeServer: true } },
    projects: [project]
  }
  store.saveConfig(config)
  step('loadConfig saved', store.loadConfig())

  // Projects
  step('dbListProjects', store.dbListProjects())
  store.dbInsertProject({ ...project, name: 'other', path: '/tmp/other' })
  store.dbUpdateProject('other', { icon: 'Star', hostIds: ['local', 'h1'] })
  step('dbGetProject', store.dbGetProject('other'))
  store.dbDeleteProject('other')
  step('dbGetProject deleted', store.dbGetProject('other'))

  // Tasks
  store.dbInsertTask(task('t1', 0))
  store.dbInsertTask({ ...task('t2', 1.5), branch: 'b', useWorktree: true })
  store.dbUpdateTask('t1', { status: 'done', completedAt: '2026-10-02T00:00:00.000Z' })
  store.dbUpdateTask('t1', { completedAt: undefined })
  step('dbListTasks', store.dbListTasks('proj'))
  step('dbListTasks by status', store.dbListTasks(undefined, 'done'))
  step('dbGetTask', store.dbGetTask('t2'))
  step('dbGetMaxTaskOrder', store.dbGetMaxTaskOrder('proj'))
  step('dbGetMaxTaskOrder none', store.dbGetMaxTaskOrder('nope'))

  // Workflows
  store.dbInsertWorkflow(workflow('wf-1'))
  store.dbInsertWorkflow(workflow('wf-2'))
  step('dbUpdateWorkflow', store.dbUpdateWorkflow('wf-2', { name: 'Two', enabled: false }))
  step('dbUpdateWorkflow none', store.dbUpdateWorkflow('missing', { name: 'x' }))
  store.updateWorkflowRunStatus('wf-1', '2026-10-01T10:00:00.000Z', 'success')
  step('dbGetWorkflow', store.dbGetWorkflow('wf-1'))
  store.dbDeleteWorkflow('wf-2')
  step('dbListWorkflows', store.dbListWorkflows())

  // Identity
  const owner = store.dbGetOwnerUser()
  step('dbGetOwnerUser', owner)
  step('dbHasDeviceTokens empty', store.dbHasDeviceTokens())
  store.dbInsertDeviceToken({
    id: 'dt1',
    userId: owner?.id ?? 'u',
    name: 'phone',
    tokenHash: 'hash',
    createdAt: '2026-10-01T00:00:00.000Z'
  })
  store.dbTouchDeviceToken('dt1', '2026-10-01T01:00:00.000Z')
  step('dbGetDeviceTokenSecret', store.dbGetDeviceTokenSecret('dt1'))
  step('dbRevokeDeviceToken', store.dbRevokeDeviceToken('dt1', '2026-10-01T02:00:00.000Z'))
  step('dbRevokeDeviceToken again', store.dbRevokeDeviceToken('dt1', '2026-10-01T03:00:00.000Z'))
  step('dbListDeviceTokens', store.dbListDeviceTokens())

  // Workspaces and groups
  store.dbInsertWorkspace({ id: 'ws', name: 'Work', order: 1 })
  store.dbUpdateWorkspace('ws', { name: 'Work 2', iconColor: '#000' })
  store.dbInsertSessionGroup({ id: 'g1', name: 'G', order: 0, workspaceId: 'ws' })
  store.dbUpdateSessionGroup('g1', { name: 'G2' })
  step('dbListSessionGroups', store.dbListSessionGroups())
  store.dbInsertSessionGroup({ id: 'g2', name: 'H', order: 1, workspaceId: 'personal' })
  store.dbDeleteSessionGroup('g2')
  store.dbDeleteWorkspace('ws')
  step('dbListWorkspaces', store.dbListWorkspaces())
  step('dbListSessionGroups after', store.dbListSessionGroups())

  // SSH keys
  store.dbSaveSSHKey({
    id: 'k1',
    label: 'key',
    encryptedPrivateKey: 'enc',
    publicKey: 'pub',
    createdAt: '2026-10-01T00:00:00.000Z'
  })
  step('dbListSSHKeys', store.dbListSSHKeys())
  step('dbGetSSHKey', store.dbGetSSHKey('k1'))
  store.dbDeleteSSHKey('k1')
  step('dbGetSSHKey deleted', store.dbGetSSHKey('k1'))

  // Source connections and the connector inbox
  store.dbInsertSourceConnection(connection)
  store.dbUpdateSourceConnection('conn-1', { name: 'renamed', lastSyncAt: undefined })
  store.dbSetConnectionSignIn('conn-1', 'me', '2026-10-01T00:00:00.000Z')
  step('dbListSourceConnections', store.dbListSourceConnections('github'))
  step('dbGetSourceConnection', store.dbGetSourceConnection('conn-1'))
  step('dbGetConnectorPollCursor none', store.dbGetConnectorPollCursor('wf-1', 'conn-1'))
  step(
    'dbRecordConnectorPollPage',
    store.dbRecordConnectorPollPage({
      workflowId: 'wf-1',
      connectionId: 'conn-1',
      connectorId: 'github',
      cursor: 'c1',
      polledAt: '2026-10-01T01:00:00.000Z',
      events: ['a', 'b', 'c'].map((id) => ({
        eventId: id,
        eventType: 'issueCreated',
        eventTimestamp: '2026-10-01T00:30:00.000Z',
        connectorItem: item(id)
      }))
    })
  )
  store.dbRecordConnectorPollError({
    workflowId: 'wf-1',
    connectionId: 'conn-1',
    error: 'boom',
    polledAt: '2026-10-01T01:30:00.000Z'
  })
  step('dbGetConnectorPollCursor', store.dbGetConnectorPollCursor('wf-1', 'conn-1'))
  store.dbEnqueueWebhookEvent({
    workflowId: 'wf-1',
    eventId: 'hook',
    receivedAt: '2026-10-01T01:40:00.000Z',
    item: item('d')
  })
  const claimed = store.dbClaimConnectorInbox({
    now: '2026-10-01T02:00:00.000Z',
    leaseUntil: '2026-10-01T02:05:00.000Z',
    limit: 3
  })
  step('dbClaimConnectorInbox', claimed)
  step(
    'dbCountActiveConnectorInboxLeases',
    store.dbCountActiveConnectorInboxLeases('2026-10-01T02:01:00.000Z')
  )
  const [first, second, third] = claimed
  step(
    'dbCompleteConnectorInbox',
    store.dbCompleteConnectorInbox(first.id, first.leaseToken, '2026-10-01T02:02:00.000Z')
  )
  step(
    'dbCompleteConnectorInbox stale',
    store.dbCompleteConnectorInbox(first.id, 'wrong', '2026-10-01T02:02:00.000Z')
  )
  step(
    'dbRetryConnectorInbox',
    store.dbRetryConnectorInbox({
      id: second.id,
      leaseToken: second.leaseToken,
      error: 'later',
      now: '2026-10-01T02:03:00.000Z'
    })
  )
  step(
    'dbRenewConnectorInboxLease',
    store.dbRenewConnectorInboxLease(third.id, third.leaseToken, '2026-10-01T02:10:00.000Z')
  )
  step(
    'dbDeferConnectorInbox',
    store.dbDeferConnectorInbox(third.id, third.leaseToken, '2026-10-01T03:00:00.000Z')
  )
  store.dbReleaseConnectorInboxLeases('2026-10-01T04:00:00.000Z')
  step(
    'dbCountActiveConnectorInboxLeases after',
    store.dbCountActiveConnectorInboxLeases('2026-10-01T04:00:00.000Z')
  )

  // Task source links
  const link = {
    taskId: 't1',
    connectionId: 'conn-1',
    connectorId: 'github',
    externalId: 'a',
    externalUrl: 'https://example.com/a',
    sourceStatusRaw: 'open',
    sourceUpdatedAt: '2026-10-01T00:00:00.000Z',
    lastSyncedAt: '2026-10-01T00:00:00.000Z',
    conflictState: 'none' as const
  }
  store.dbInsertTaskSourceLink(link)
  store.dbUpdateTaskSourceLink('t1', { conflictState: 'upstream_changed' })
  step('dbGetTaskSourceLink', store.dbGetTaskSourceLink('t1'))
  step('dbGetTaskSourceLinkByExternalId', store.dbGetTaskSourceLinkByExternalId('conn-1', 'a'))
  step('dbListTaskSourceLinks', store.dbListTaskSourceLinks('conn-1'))
  store.dbUpdateTask('t2', { sourceConnectorId: 'github', sourceExternalId: 'z' })
  step('dbFindTaskByConnectorExternalId', store.dbFindTaskByConnectorExternalId('github', 'z'))
  store.dbDeleteTaskSourceLink('t1')
  step('dbGetTaskSourceLink deleted', store.dbGetTaskSourceLink('t1'))

  // Workflow runs
  store.saveWorkflowRun(run('wf-1:r1', 'running'))
  store.saveWorkflowRun({ ...run('wf-1:r2', 'success'), completedAt: '2026-10-01T11:00:00.000Z' })
  store.saveWorkflowRun({ ...run('wf-1:r3', 'running'), connectorInboxId: first.id })
  step('listWorkflowRunIds', store.listWorkflowRunIds())
  step('getWorkflowRun', store.getWorkflowRun('wf-1:r2'))
  step('listWorkflowRuns', store.listWorkflowRuns('wf-1', 2))
  step('listWorkflowRunsByTask', store.listWorkflowRunsByTask('t1'))
  step('listRunningRuns', store.listRunningRuns())
  step('listRunsWithWaitingGates', store.listRunsWithWaitingGates())
  step('listRunsWithWaitingGates signIn', store.listRunsWithWaitingGates('signIn'))
  step('listAllWorkflowRuns', store.listAllWorkflowRuns('personal', 10))
  step('dbGetWorkflowRunByConnectorInboxId', store.dbGetWorkflowRunByConnectorInboxId(first.id))

  // Sessions, schedule log, effects, events
  store.saveSessions([
    {
      id: 's1',
      agentType: 'claude',
      projectName: 'proj',
      projectPath: '/tmp/proj',
      status: 'running',
      createdAt: 1_700_000_000_000,
      pid: 42,
      branch: 'main',
      isWorktree: false
    }
  ])
  step('getPreviousSessions', store.getPreviousSessions())
  store.clearSessions()
  step('getPreviousSessions cleared', store.getPreviousSessions())
  store.addScheduleLogEntry({
    workflowId: 'wf-1',
    workflowName: 'wf-1',
    executedAt: '2026-10-01T00:00:00.000Z',
    status: 'error',
    sessionsLaunched: 0,
    error: 'nope'
  })
  step('getScheduleLogEntries', store.getScheduleLogEntries('wf-1'))
  store.clearScheduleLog()
  step('getScheduleLogEntries cleared', store.getScheduleLogEntries())
  step('claimEffect', store.claimEffect('e1', 'notify', 1_000))
  step('claimEffect again', store.claimEffect('e1', 'notify', 2_000))
  step('pruneEffectReceipts', store.pruneEffectReceipts('notify', 1_500))
  store.insertSessionEvent({
    sessionId: 's1',
    eventType: 'created',
    timestamp: '2026-10-01T00:00:00.000Z',
    metadata: { a: 1 }
  })
  step('listSessionEvents', store.listSessionEvents('created', 5))
  step('listSessionEventsBySession', store.listSessionEventsBySession('s1'))

  // Artifacts
  const { artifact } = store.insertArtifact({
    kind: 'page',
    title: 'Page',
    sessionId: 's1',
    projectName: 'proj',
    gateRunId: 'wf-1:r1',
    gateNodeId: 'gate'
  })
  step('getArtifact', store.getArtifact(artifact.id))
  step('getArtifactToken', typeof store.getArtifactToken(artifact.id))
  step('findGateArtifact', store.findGateArtifact('wf-1:r1', 'gate'))
  store.renameArtifact(artifact.id, 'Renamed')
  step('listArtifacts', store.listArtifacts({ projectName: 'proj' }, 5))
  step('addArtifactVersion', store.addArtifactVersion(artifact.id, 'agent'))
  const comment = store.insertArtifactComment({
    artifactId: artifact.id,
    version: 1,
    anchor: null,
    body: 'first'
  })
  step('insertArtifactComment', comment)
  step('updateArtifactComment', store.updateArtifactComment(comment.id, { body: 'edited' }))
  step('getArtifactComment', store.getArtifactComment(comment.id))
  step('sendArtifactDrafts', store.sendArtifactDrafts(artifact.id))
  step('unansweredBatchId', store.unansweredBatchId(artifact.id))
  step('listArtifactComments', store.listArtifactComments(artifact.id, { state: 'sent' }))
  step('listArtifactVersions', store.listArtifactVersions(artifact.id))
  const draft = store.insertArtifactComment({
    artifactId: artifact.id,
    version: 1,
    anchor: null,
    body: 'second'
  })
  step('deleteArtifactComment', store.deleteArtifactComment(draft.id))
  step('listArtifactIds', store.listArtifactIds())
  step(
    'deleteArtifactsUpdatedBefore',
    store.deleteArtifactsUpdatedBefore('2000-01-01T00:00:00.000Z')
  )

  // A final snapshot of everything, and a second save of it
  const last = store.loadConfig()
  step('loadConfig final', last)
  store.saveConfig(last)
  step('loadConfig resaved', store.loadConfig())
  return out
}

describe('store', () => {
  it('answers every call as the TypeScript store it replaced did', () => {
    const teardown = store.initTestDatabase()
    let actual: Array<[string, unknown]>
    try {
      actual = scenario().map(([label, value]) => [
        label,
        normalizeStoreOutput(withoutMachineValues(value))
      ])
    } finally {
      teardown()
    }
    const reference = storeReference as Array<[string, unknown]>
    expect(actual.map(([label]) => label)).toEqual(reference.map(([label]) => label))
    for (let i = 0; i < reference.length; i++) {
      expect({ [actual[i][0]]: actual[i][1] }).toEqual({ [reference[i][0]]: reference[i][1] })
    }
  })
})
