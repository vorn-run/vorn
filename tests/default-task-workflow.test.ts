import fs from 'node:fs'
import path from 'node:path'
import { describe, it, expect } from 'vitest'
import type {
  LaunchAgentConfig,
  TaskStatusChangedTriggerConfig,
  WorkflowDefinition
} from '../packages/shared/src/types'

/**
 * The workflows a new database is seeded with, as vornd answered `workflow:list`
 * (the replay records it; the Workflows built-in ships them as files).
 */
const SEEDS = JSON.parse(
  fs.readFileSync(path.resolve(__dirname, 'fixtures/workflows-replay/seeded.json'), 'utf-8')
) as WorkflowDefinition[]

const seeded = (): WorkflowDefinition => {
  const seed = SEEDS.find((s) => s.id === 'system:default-task-workflow')
  expect(seed).toBeDefined()
  return seed!
}

describe('the seeded default task workflow', () => {
  it('has the stable system id', () => {
    expect(seeded().id).toBe('system:default-task-workflow')
  })

  it('is enabled and scoped to the personal workspace', () => {
    const wf = seeded()
    expect(wf.enabled).toBe(true)
    expect(wf.workspaceId).toBe('personal')
  })

  it('triggers on todo → in_progress with no project filter', () => {
    const triggerNode = seeded().nodes.find((n) => n.type === 'trigger')
    expect(triggerNode).toBeDefined()
    const trigger = triggerNode!.config as TaskStatusChangedTriggerConfig
    expect(trigger.triggerType).toBe('taskStatusChanged')
    expect(trigger.fromStatus).toBe('todo')
    expect(trigger.toStatus).toBe('in_progress')
    expect(trigger.projectFilter).toBeUndefined()
  })

  it('has exactly one headless launchAgent node using fromTask', () => {
    const launchNodes = seeded().nodes.filter((n) => n.type === 'launchAgent')
    expect(launchNodes).toHaveLength(1)
    const launch = launchNodes[0].config as LaunchAgentConfig
    expect(launch.agentType).toBe('fromTask')
    expect(launch.headless).toBe(true)
  })

  it('connects the trigger to the launchAgent with one edge', () => {
    const wf = seeded()
    expect(wf.edges).toHaveLength(1)
    const [edge] = wf.edges
    const trigger = wf.nodes.find((n) => n.type === 'trigger')!
    const launch = wf.nodes.find((n) => n.type === 'launchAgent')!
    expect(edge.source).toBe(trigger.id)
    expect(edge.target).toBe(launch.id)
  })

  it('seeds the dev server workflow switched off', () => {
    const seed = SEEDS.find((s) => s.id === 'system:dev-server-on-restore')
    expect(seed?.enabled).toBe(false)
  })
})
