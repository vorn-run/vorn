import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
vi.mock('node:fs', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return { ...actual, existsSync: vi.fn(() => true), mkdirSync: vi.fn() }
})

import {
  initTestDatabase,
  dbListSessionGroups,
  dbInsertSessionGroup,
  dbUpdateSessionGroup,
  dbDeleteSessionGroup,
  saveSessions,
  getPreviousSessions
} from '../packages/server/src/database'
import type { SessionGroupConfig, TerminalSession } from '../packages/shared/src/types'

let close: (() => void) | null = null

beforeEach(() => {
  close = initTestDatabase()
})

afterEach(() => {
  close?.()
  close = null
})

const group = (over: Partial<SessionGroupConfig> = {}): SessionGroupConfig => ({
  id: 'g1',
  name: 'Sidebar work',
  order: 0,
  workspaceId: 'personal',
  ...over
})

const session = (id: string, groupId?: string) =>
  ({
    id,
    agentType: 'claude',
    projectName: 'vorn',
    projectPath: '/tmp/vorn',
    status: 'idle',
    createdAt: 1,
    pid: 1,
    ...(groupId && { groupId })
  }) as TerminalSession

describe('the group rows themselves', () => {
  it('comes back with everything it was given', () => {
    dbInsertSessionGroup(group({ icon: 'Terminal', iconColor: '#eab308' }))

    expect(dbListSessionGroups()).toEqual([
      {
        id: 'g1',
        name: 'Sidebar work',
        order: 0,
        workspaceId: 'personal',
        icon: 'Terminal',
        iconColor: '#eab308'
      }
    ])
  })

  it('drops the fields it was not given rather than storing nulls', () => {
    dbInsertSessionGroup(group())

    const [loaded] = dbListSessionGroups()
    expect(loaded).not.toHaveProperty('icon')
    expect(loaded).not.toHaveProperty('iconColor')
  })

  it('lists in the order the person arranged, not insertion order', () => {
    dbInsertSessionGroup(group({ id: 'b', name: 'Second', order: 2 }))
    dbInsertSessionGroup(group({ id: 'a', name: 'First', order: 1 }))

    expect(dbListSessionGroups().map((g) => g.name)).toEqual(['First', 'Second'])
  })

  it('updates one field without disturbing the rest', () => {
    dbInsertSessionGroup(group({ icon: 'Terminal', iconColor: '#eab308' }))
    dbUpdateSessionGroup('g1', { name: 'Renamed' })

    expect(dbListSessionGroups()[0]).toMatchObject({
      name: 'Renamed',
      icon: 'Terminal',
      iconColor: '#eab308',
      order: 0
    })
  })

  it('can be moved, recoloured and re-homed', () => {
    dbInsertSessionGroup(group())
    dbUpdateSessionGroup('g1', {
      order: 5,
      icon: 'Rocket',
      iconColor: '#3b82f6',
      workspaceId: 'work'
    })

    expect(dbListSessionGroups()[0]).toMatchObject({
      order: 5,
      icon: 'Rocket',
      iconColor: '#3b82f6',
      workspaceId: 'work'
    })
  })

  it('writes nothing when asked to change nothing', () => {
    dbInsertSessionGroup(group())
    dbUpdateSessionGroup('g1', {})

    expect(dbListSessionGroups()[0].name).toBe('Sidebar work')
  })
})

describe('deleting a group', () => {
  it('lets its sessions go without ending them', () => {
    dbInsertSessionGroup(group())
    saveSessions([session('s1', 'g1'), session('s2', 'g1'), session('s3')])

    dbDeleteSessionGroup('g1')

    expect(dbListSessionGroups()).toEqual([])
    const left = getPreviousSessions()
    expect(left.map((s) => s.id).sort()).toEqual(['s1', 's2', 's3'])
    expect(left.every((s) => s.groupId === undefined)).toBe(true)
  })

  it("leaves another group's sessions filed where they were", () => {
    dbInsertSessionGroup(group())
    dbInsertSessionGroup(group({ id: 'g2', name: 'Release' }))
    saveSessions([session('s1', 'g1'), session('s2', 'g2')])

    dbDeleteSessionGroup('g1')

    expect(getPreviousSessions().find((s) => s.id === 's2')?.groupId).toBe('g2')
  })
})
