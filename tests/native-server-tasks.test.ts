/**
 * The task board, a task's images, the projects and the session-event log,
 * answered by vornd through a real server and the vornd it keeps.
 *
 * Each write changes only what it names, keeps the board's orders a
 * permutation, stamps and clears the finished and archived dates, and tells
 * every client the configuration changed only when a row did. A terminal's
 * life is logged as it is created, renamed and ends. None of these calls
 * reaches the server.
 *
 * Runs where vornd and its session holder have been built.
 */
import fs from 'node:fs'
import path from 'node:path'
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import type {
  AppConfig,
  ProjectConfig,
  TaskConfig,
  TerminalSession
} from '../packages/shared/src/types'
import { queryDb } from './helpers/database'
import {
  type RealServer,
  Watcher,
  removeRealServerDirs,
  runnable,
  startRealServer,
  stopRealServer,
  until
} from './helpers/real-server'

vi.setConfig({ testTimeout: 60_000, hookTimeout: 120_000 })

interface Wrote {
  ok: boolean
  task?: TaskConfig
}

describe.runIf(runnable)('the task board in vornd', () => {
  let server: RealServer
  let client: Watcher
  let project = ''

  const call = <T = Wrote>(method: string, params?: unknown): Promise<T> =>
    client.result<T>(method, params)
  const get = (id: string): Promise<TaskConfig | null> =>
    call<TaskConfig | null>('task:get', { id })
  const create = async (fields: Record<string, unknown> = {}): Promise<TaskConfig> => {
    const res = await call('task:create', { projectName: project, title: 'A task', ...fields })
    expect(res.ok).toBe(true)
    return res.task!
  }
  const changes = (): number => client.toldBy('config:changed').length
  /** Lets a broadcast sent before the last answer arrive. */
  const settle = (): Promise<void> => new Promise((r) => setTimeout(r, 100))

  const addProject = async (name: string, dir: string): Promise<void> => {
    const config = await call<AppConfig>('config:load')
    const projects = [
      ...config.projects,
      { name, path: dir, preferredAgents: ['claude'] } as ProjectConfig
    ]
    await call('config:save', { ...config, projects })
  }

  beforeAll(async () => {
    server = await startRealServer()
    client = await Watcher.open(server.vornd)
    project = 'board'
    await addProject(project, server.dirs.work)
    await addProject('other', server.dirs.work)
  })

  afterAll(async () => {
    client?.close()
    if (server) await stopRealServer(server)
    removeRealServerDirs()
  })

  beforeEach(async () => {
    for (const task of await call<TaskConfig[]>('task:list', { includeDescription: true })) {
      await call('task:delete', { id: task.id })
    }
  })

  it('lists the projects', async () => {
    const projects = await call<ProjectConfig[]>('project:list')
    expect(projects.map((p) => p.name)).toEqual(expect.arrayContaining(['board', 'other']))
  })

  it('creates a task after the last one, and refuses a project that does not exist', async () => {
    const first = await create({ description: 'Why it matters.' })
    expect(first).toMatchObject({ status: 'todo', description: 'Why it matters.', order: 0 })
    expect((await create()).order).toBe(1)
    expect((await create({ status: 'done' })).completedAt).toBeTruthy()
    expect((await create({ status: 'in_progress' })).completedAt).toBeUndefined()
    expect(await call('task:create', { projectName: 'nowhere', title: 'Orphan' })).toEqual({
      ok: false
    })
  })

  it('lists a board without descriptions unless asked', async () => {
    await create({ description: 'Long text' })
    const [brief] = await call<TaskConfig[]>('task:list', { projectName: project })
    expect(brief.description).toBe('')
    const [whole] = await call<TaskConfig[]>('task:list', {
      projectName: project,
      includeDescription: true
    })
    expect(whole.description).toBe('Long text')
  })

  it('updates only what it names, and moves a task to the end of another board', async () => {
    const task = await create({ description: 'Original' })
    const res = await call('task:update', {
      id: task.id,
      title: 'Renamed',
      archivedAt: '2026-01-01T00:00:00.000Z',
      completedAt: '2026-01-01T00:00:00.000Z',
      order: 999
    })
    expect(res.ok).toBe(true)
    expect(res.task).toMatchObject({ title: 'Renamed', description: 'Original', order: 0 })
    expect(res.task?.archivedAt).toBeUndefined()
    expect(res.task?.completedAt).toBeUndefined()

    const there = await create({ projectName: 'other' })
    await call('task:update', { id: task.id, projectName: 'other' })
    expect((await get(task.id))?.order).toBe(there.order + 1)
    expect(await call('task:update', { id: task.id, projectName: 'nowhere' })).toEqual({
      ok: false
    })
    expect(await call('task:update', { id: 'ghost', title: 'x' })).toEqual({ ok: false })
  })

  it('stamps a finished task, and clears both dates when it is reopened', async () => {
    const task = await create()
    await call('task:setStatus', { id: task.id, status: 'done' })
    expect((await get(task.id))?.completedAt).toBeTruthy()
    expect(await call('task:archive', { id: task.id, archived: true })).toEqual({ ok: true })
    expect((await get(task.id))?.archivedAt).toBeTruthy()
    await call('task:update', { id: task.id, status: 'in_progress' })
    const reopened = await get(task.id)
    expect(reopened?.completedAt).toBeUndefined()
    expect(reopened?.archivedAt).toBeUndefined()
    expect(await call('task:archive', { id: task.id, archived: true })).toEqual({ ok: false })
  })

  it('marks a moved card as touched', async () => {
    const task = await create()
    const stale = '2020-01-01T00:00:00.000Z'
    queryDb(path.join(server.dirs.data, 'vorn.db'), (d) =>
      d.prepare('UPDATE tasks SET updated_at = ? WHERE id = ?').run(stale, task.id)
    )
    expect(await call('task:setStatus', { id: task.id, status: 'in_progress' })).toEqual({
      ok: true
    })
    expect((await get(task.id))?.updatedAt).not.toBe(stale)
  })

  it('reorders by handing out the places the named tasks held', async () => {
    const [a, b, c, d] = [await create(), await create(), await create(), await create()]
    await call('task:reorder', { ids: [c.id, a.id] })
    const orders = async (): Promise<Array<number | undefined>> =>
      Promise.all([a, b, c, d].map(async (t) => (await get(t.id))?.order))
    expect(await orders()).toEqual([2, 1, 0, 3])
    await call('task:reorder', { ids: [b.id, a.id, 'ghost', b.id] })
    expect(new Set(await orders()).size).toBe(4)
    expect(await call('task:reorder', { ids: ['ghost'] })).toEqual({ ok: false })
  })

  it('tells every client of each change, and of nothing that changed nothing', async () => {
    const task = await create()
    const other = await create()
    await settle()
    const before = changes()
    await call('task:update', { id: task.id, title: 'Renamed' })
    await call('task:setStatus', { id: task.id, status: 'done' })
    await call('task:archive', { id: task.id, archived: true })
    await call('task:reorder', { ids: [other.id, task.id] })
    await call('task:delete', { id: task.id })
    await until('five changes told', () => changes() === before + 5)

    await call('task:reorder', { ids: [other.id] })
    await call('task:update', { id: 'ghost', title: 'x' })
    await call('task:delete', { id: 'ghost' })
    await call('task:create', { projectName: 'nowhere', title: 'Orphan' })
    await settle()
    expect(changes()).toBe(before + 5)
  })

  it('keeps a task’s images under names it chose', async () => {
    const task = await create()
    const name = await call<string>('task:imageUpload', {
      taskId: task.id,
      base64: Buffer.from('png bytes').toString('base64'),
      filename: 'shot.PNG'
    })
    expect(name).toMatch(/^[0-9a-f-]{36}\.png$/)
    const file = await call<string>('task:imageGetPath', { taskId: task.id, filename: name })
    expect(fs.readFileSync(file, 'utf-8')).toBe('png bytes')

    const source = path.join(server.dirs.work, 'pic.jpg')
    fs.writeFileSync(source, 'jpeg')
    const copied = await call<string>('task:imageSave', { taskId: task.id, sourcePath: source })
    await call('task:imageDelete', { taskId: task.id, filename: copied })
    expect(fs.existsSync(path.join(path.dirname(file), copied))).toBe(false)

    const refused = await client.call('task:imageUpload', {
      taskId: task.id,
      base64: '',
      filename: 'a.svg'
    })
    expect(refused.error).toMatchObject({ message: 'Unsupported image type: .svg' })
    const escaped = await client.call('task:imageGetPath', { taskId: '../x', filename: 'a.png' })
    expect(escaped.error).toMatchObject({ message: 'Invalid taskId: ../x' })

    await call('task:imageCleanup', task.id)
    expect(fs.existsSync(path.dirname(file))).toBe(false)
  })

  it('tells a mobile project from any other', async () => {
    const app = path.join(server.dirs.work, 'app')
    fs.mkdirSync(app, { recursive: true })
    fs.writeFileSync(path.join(app, 'package.json'), '{"dependencies":{"expo":"1"}}')
    expect(await call('project:detectMobile', { projectPath: app })).toEqual({
      isMobile: true,
      framework: 'expo',
      needsDevClient: true
    })
    expect(await call('project:detectMobile', { projectPath: server.dirs.home })).toMatchObject({
      isMobile: false
    })
  })

  it('logs a terminal as it is created, renamed and ends', async () => {
    const shell = await call<TerminalSession>('shell:create', server.dirs.work)
    await call('terminal:rename', { id: shell.id, displayName: 'mine' })
    await call('terminal:kill', shell.id)
    type Logged = { eventType: string; metadata?: Record<string, unknown> }
    let events: Logged[] = []
    await until('the exit logged', async () => {
      events = await call<Logged[]>('sessionEvent:listBySession', { sessionId: shell.id })
      return events.some((e) => e.eventType === 'exited')
    })
    const kinds = events.map((e) => e.eventType)
    expect(kinds.filter((k) => k === 'created')).toHaveLength(1)
    expect(kinds.filter((k) => k === 'exited')).toHaveLength(1)
    expect(events.find((e) => e.eventType === 'renamed')?.metadata).toEqual({ displayName: 'mine' })
    expect(events.find((e) => e.eventType === 'created')?.metadata).toMatchObject({
      agentType: 'shell'
    })
    const all = await call<Logged[]>('sessionEvent:list', { eventType: 'renamed', limit: 10 })
    expect(all.length).toBeGreaterThan(0)
  })
})
