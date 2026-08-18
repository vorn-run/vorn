import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { AppConfig } from '../packages/shared/src/types'

/**
 * Tasks, projects, workspaces and workflows over the socket instead of straight
 * into this machine's SQLite.
 *
 * Reading the local file was fine while the only server was this machine's, and
 * wrong once the desktop could be pointed at a host: the data is over there, and
 * the local copy is stale. These go to whichever server MCP is talking to without
 * knowing which one that is.
 */

const calls: Array<{ method: string; params?: unknown }> = []
let stored: AppConfig

vi.mock('../packages/mcp/src/ws-client', () => ({
  rpcCall: async (method: string, params?: unknown) => {
    calls.push({ method, params })
    if (method === 'config:load') return stored
    if (method === 'config:save') {
      stored = params as AppConfig
      return undefined
    }
    return []
  }
}))

import {
  dbListTasks,
  dbGetTask,
  dbInsertTask,
  dbUpdateTask,
  dbDeleteTask,
  dbGetMaxTaskOrder,
  dbListProjects,
  dbGetProject,
  dbInsertProject,
  dbListWorkspaces,
  dbDeleteWorkspace,
  listAllWorkflowRuns
} from '../packages/mcp/src/data-access'

const task = (id: string, over: Record<string, unknown> = {}) => ({
  id,
  projectName: 'vorn',
  title: id,
  description: '',
  status: 'todo',
  order: 0,
  createdAt: '2026-08-17T00:00:00.000Z',
  updatedAt: '2026-08-17T00:00:00.000Z',
  ...over
})

beforeEach(() => {
  calls.length = 0
  stored = {
    version: 1,
    revision: 7,
    defaults: { shell: '/bin/zsh', fontSize: 13, theme: 'dark' },
    projects: [{ name: 'vorn', path: '/repo', preferredAgents: [] }],
    tasks: [task('a'), task('b', { status: 'done' })],
    workspaces: [{ id: 'personal', name: 'Personal', order: 0 }],
    workflows: []
  } as unknown as AppConfig
})

describe('reading', () => {
  it('goes over the socket rather than opening a database', async () => {
    await dbListTasks()

    expect(calls[0].method).toBe('config:load')
  })

  it('filters tasks by project and status the way the query did', async () => {
    expect((await dbListTasks('vorn')).map((t) => t.id)).toEqual(['a', 'b'])
    expect((await dbListTasks('vorn', 'done')).map((t) => t.id)).toEqual(['b'])
    expect(await dbListTasks('other')).toEqual([])
  })

  it('finds one by id, and answers null rather than throwing', async () => {
    expect((await dbGetTask('a'))?.id).toBe('a')
    expect(await dbGetTask('missing')).toBeNull()
  })

  it('reports the highest order in a project', async () => {
    stored.tasks = [task('a', { order: 3 }), task('b', { order: 9 })] as never

    expect(await dbGetMaxTaskOrder('vorn')).toBe(9)
  })

  it('returns 0 for a project with no tasks, so the first insert sorts sanely', async () => {
    expect(await dbGetMaxTaskOrder('empty')).toBe(0)
  })

  it('reads projects and workspaces the same way', async () => {
    expect((await dbListProjects()).map((p) => p.name)).toEqual(['vorn'])
    expect((await dbGetProject('vorn'))?.path).toBe('/repo')
    expect((await dbListWorkspaces()).map((w) => w.id)).toEqual(['personal'])
  })
})

describe('writing', () => {
  it('adds without disturbing what is already there', async () => {
    await dbInsertTask(task('c') as never)

    expect(stored.tasks?.map((t) => t.id)).toEqual(['a', 'b', 'c'])
  })

  it('updates in place', async () => {
    await dbUpdateTask('a', { title: 'renamed' })

    expect(stored.tasks?.find((t) => t.id === 'a')?.title).toBe('renamed')
    expect(stored.tasks).toHaveLength(2)
  })

  it('deletes only what was asked for', async () => {
    await dbDeleteTask('a')

    expect(stored.tasks?.map((t) => t.id)).toEqual(['b'])
  })

  it('carries the revision back, so the server can spot a stale save', async () => {
    // The reason read-modify-write is safe here at all: the server keeps rows
    // added by anyone else since this snapshot rather than pruning them.
    await dbInsertTask(task('c') as never)

    const save = calls.find((c) => c.method === 'config:save')
    expect((save?.params as AppConfig).revision).toBe(7)
  })

  it('leaves other collections untouched when writing one', async () => {
    await dbDeleteWorkspace('personal')

    expect(stored.workspaces).toEqual([])
    expect(stored.projects).toHaveLength(1)
    expect(stored.tasks).toHaveLength(2)
  })

  it('inserts a project without dropping the tasks', async () => {
    await dbInsertProject({ name: 'other', path: '/other', preferredAgents: [] } as never)

    expect(stored.projects?.map((p) => p.name)).toEqual(['vorn', 'other'])
    expect(stored.tasks).toHaveLength(2)
  })
})

describe('workflow runs', () => {
  it('uses the RPC that serves them, since they are not in the config blob', async () => {
    await listAllWorkflowRuns('personal', 10)

    expect(calls[0]).toEqual({
      method: 'workflowRun:listAll',
      params: { workspaceId: 'personal', limit: 10 }
    })
  })
})
