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

type Row = Record<string, unknown> & { id: string }

const calls: Array<{ method: string; params?: unknown }> = []
let stored: AppConfig
/** The server's tasks, which the configuration does not carry. */
let rows: Row[]

/** A fake server: the configuration over `config:*`, the tasks over `task:*`. */
function answer(method: string, params: Record<string, unknown> = {}): unknown {
  const found = rows.find((t) => t.id === params.id)
  switch (method) {
    case 'config:load':
      return stored
    case 'config:save':
      stored = params as unknown as AppConfig
      return undefined
    case 'task:list':
      return rows.filter(
        (t) =>
          (params.projectName === undefined || t.projectName === params.projectName) &&
          (params.status === undefined || t.status === params.status)
      )
    case 'task:get':
      return found ?? null
    case 'task:create':
      if (found) return { ok: false }
      rows.push(params as Row)
      return { ok: true, task: params }
    case 'task:patch': {
      if (!found) return { ok: false }
      for (const [k, v] of Object.entries(params.set as Record<string, unknown>)) {
        if (v === null) delete found[k]
        else found[k] = v
      }
      return { ok: true, task: found }
    }
    case 'task:delete':
      rows = rows.filter((t) => t.id !== params.id)
      return { ok: found !== undefined }
    default:
      return []
  }
}

vi.mock('../packages/mcp/src/rpc-client', () => ({
  rpcCall: async (method: string, params?: unknown) => {
    calls.push({ method, params })
    return answer(method, params as Record<string, unknown>)
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
    workspaces: [{ id: 'personal', name: 'Personal', order: 0 }]
  } as unknown as AppConfig
  rows = [task('a'), task('b', { status: 'done' })]
})

describe('reading', () => {
  it('goes over the socket rather than opening a database', async () => {
    await dbListTasks()

    expect(calls[0]).toEqual({
      method: 'task:list',
      params: { projectName: undefined, status: undefined, includeDescription: true }
    })
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
    rows = [task('a', { order: 3 }), task('b', { order: 9 })]

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
  it('creates a task through its own call, under its own id', async () => {
    await dbInsertTask(task('c', { assignedSessionId: 's1' }) as never)

    expect(rows.map((t) => t.id)).toEqual(['a', 'b', 'c'])
    expect(rows[2].assignedSessionId).toBe('s1')
    expect(calls.map((c) => c.method)).toEqual(['task:create', 'task:patch'])
  })

  it('updates only the fields it names, and clears one set to undefined', async () => {
    rows[0].archivedAt = '2026-08-18T00:00:00.000Z'
    await dbUpdateTask('a', { title: 'renamed', archivedAt: undefined })

    expect(rows.find((t) => t.id === 'a')?.title).toBe('renamed')
    expect(rows.find((t) => t.id === 'a')).not.toHaveProperty('archivedAt')
    expect(rows).toHaveLength(2)
    await expect(dbUpdateTask('missing', { title: 'x' })).rejects.toThrow('not found')
  })

  it('deletes only what was asked for', async () => {
    await dbDeleteTask('a')

    expect(rows.map((t) => t.id)).toEqual(['b'])
  })

  it('never writes a task through the configuration', async () => {
    await dbInsertTask(task('c') as never)
    await dbUpdateTask('c', { status: 'done' })

    expect(calls.some((c) => c.method === 'config:save')).toBe(false)
  })

  it('leaves other collections untouched when writing one', async () => {
    await dbDeleteWorkspace('personal')

    expect(stored.workspaces).toEqual([])
    expect(stored.projects).toHaveLength(1)
    expect(rows).toHaveLength(2)
  })

  it('carries the revision back, so the server can spot a stale save', async () => {
    // The reason read-modify-write is safe here at all: the server keeps rows
    // added by anyone else since this snapshot rather than pruning them.
    await dbInsertProject({ name: 'other', path: '/other', preferredAgents: [] } as never)

    const save = calls.find((c) => c.method === 'config:save')
    expect((save?.params as AppConfig).revision).toBe(7)
    expect(stored.projects?.map((p) => p.name)).toEqual(['vorn', 'other'])
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
