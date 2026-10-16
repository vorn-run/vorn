/**
 * Today's Tasks, replayed: a scripted run of every `task:*` call, the
 * projects and the session-event log, and of the MCP task tools, whose
 * answers were recorded from vornd before Tasks became a built-in. Each run
 * must answer the same, call for call, and tell clients the configuration
 * changed after the same calls.
 *
 * Ids, times and the test's directories are normalized before comparing.
 * `VORN_RECORD_TASKS=1` records the fixtures again from the vornd built here.
 *
 * Runs where vornd has been built.
 */
import fs from 'node:fs'
import path from 'node:path'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import { builtVornd, servedDir, startServed, type Served } from './helpers/served'
import { Watcher, until } from './helpers/real-server'

vi.setConfig({ testTimeout: 120_000, hookTimeout: 120_000 })

const CREDENTIAL = 'native-server-tasks-replay-credential'
const FIXTURES = path.join(__dirname, 'fixtures', 'tasks-replay')
const RECORD = process.env.VORN_RECORD_TASKS === '1'

type Step = { call: string; params?: unknown } | { tool: string; arguments?: unknown }
type Recorded = { step: Step; answer: unknown; changed: number }

/** Ids, times and directories as names that are the same on every run. */
function normalizer(dirs: Record<string, string>): (value: unknown) => unknown {
  const ids = new Map<string, string>()
  const text = (s: string): string => {
    let out = s
    for (const [name, dir] of Object.entries(dirs)) out = out.split(dir).join(`<${name}>`)
    out = out.replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g, (id) => {
      if (!ids.has(id)) ids.set(id, `<id${ids.size + 1}>`)
      return ids.get(id)!
    })
    return out.replace(/\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?Z/g, '<time>')
  }
  const walk = (v: unknown): unknown => {
    if (typeof v === 'string') return text(v)
    if (Array.isArray(v)) return v.map(walk)
    if (v && typeof v === 'object') {
      return Object.fromEntries(Object.entries(v).map(([k, x]) => [text(k), walk(x)]))
    }
    return v
  }
  return walk
}

/** The ids the script refers back to as `$1`, `$2`: the tasks it created, in order. */
function resolve(v: unknown, made: string[]): unknown {
  if (typeof v === 'string') {
    return v.replace(/\$(\d+)/g, (_, n: string) => made[Number(n) - 1] ?? `missing-${n}`)
  }
  if (Array.isArray(v)) return v.map((x) => resolve(x, made))
  if (v && typeof v === 'object') {
    return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, resolve(x, made)]))
  }
  return v
}

const IMAGE = Buffer.from('png bytes').toString('base64')

/** The `task:*`, `project:*` and `sessionEvent:*` calls, in the order a person or agent might make them. */
const CALLS: Step[] = [
  { call: 'project:list' },
  {
    call: 'task:create',
    params: { projectName: 'board', title: 'A', description: 'Why it matters.' }
  },
  { call: 'task:create', params: { projectName: 'board', title: 'B', status: 'done' } },
  {
    call: 'task:create',
    params: {
      projectName: 'board',
      title: 'C',
      status: 'in_progress',
      assignedAgent: 'claude',
      branch: 'c-branch',
      useWorktree: true
    }
  },
  { call: 'task:create', params: { projectName: 'nowhere', title: 'Orphan' } },
  { call: 'task:create', params: { projectName: 'board', title: 'Dup', id: '$1' } },
  { call: 'task:list', params: { projectName: 'board' } },
  { call: 'task:list', params: { includeDescription: true } },
  { call: 'task:list', params: { status: 'done' } },
  { call: 'task:list', params: { projectName: 'board', offset: 1, limit: 1 } },
  { call: 'task:list', params: { projectName: '', status: '' } },
  { call: 'task:get', params: { id: '$1' } },
  { call: 'task:get', params: { id: 'ghost' } },
  {
    call: 'task:update',
    params: {
      id: '$1',
      title: 'Renamed',
      archivedAt: '2026-01-01T00:00:00.000Z',
      completedAt: '2026-01-01T00:00:00.000Z',
      order: 999
    }
  },
  { call: 'task:update', params: { id: '$1', projectName: 'other' } },
  { call: 'task:update', params: { id: '$1', projectName: 'nowhere' } },
  { call: 'task:update', params: { id: 'ghost', title: 'x' } },
  { call: 'task:update', params: { id: '$1', projectName: 'board', status: 'in_review' } },
  {
    call: 'task:patch',
    params: { id: '$1', set: { title: 'Patched', branch: 'p', assignedAgent: null } }
  },
  { call: 'task:patch', params: { id: '$1', set: { status: 'done' } } },
  { call: 'task:patch', params: { id: '$1', set: { projectName: 'nowhere' } } },
  { call: 'task:patch', params: { id: '$1' } },
  { call: 'task:patch', params: { id: 'ghost', set: { title: 'x' } } },
  { call: 'task:setStatus', params: { id: '$2', status: 'todo' } },
  { call: 'task:setStatus', params: { id: '$2', status: 'cancelled' } },
  { call: 'task:setStatus', params: { id: 'ghost', status: 'done' } },
  { call: 'task:archive', params: { id: '$2', archived: true } },
  { call: 'task:archive', params: { id: '$2', archived: true } },
  { call: 'task:archive', params: { id: '$3', archived: true } },
  { call: 'task:archive', params: { id: '$2', archived: false } },
  { call: 'task:reorder', params: { ids: ['$3', '$1'] } },
  { call: 'task:reorder', params: { ids: ['$2', 'ghost', '$2'] } },
  { call: 'task:reorder', params: { ids: ['ghost'] } },
  { call: 'task:list', params: { includeDescription: true } },
  {
    call: 'task:imageUpload',
    params: { taskId: '$1', base64: IMAGE, filename: 'shot.PNG' }
  },
  { call: 'task:imageUpload', params: { taskId: '$1', base64: '', filename: 'a.svg' } },
  { call: 'task:imageGetPath', params: { taskId: '../x', filename: 'a.png' } },
  { call: 'task:imageGetPath', params: { taskId: '$1', filename: 'kept.png' } },
  { call: 'task:imageSave', params: { taskId: '$1', sourcePath: '<work>/pic.jpg' } },
  { call: 'task:imageDelete', params: { taskId: '$1', filename: 'missing.jpg' } },
  { call: 'task:imageCleanup', params: '$1' },
  { call: 'task:delete', params: { id: '$2' } },
  { call: 'task:delete', params: { id: '$2' } },
  { call: 'task:list', params: { includeDescription: true } },
  { call: 'sessionEvent:list', params: { limit: 5 } },
  { call: 'sessionEvent:list', params: { eventType: 'renamed' } },
  { call: 'sessionEvent:listBySession', params: { sessionId: 'ghost' } },
  { call: 'project:detectMobile', params: { projectPath: '<work>/app' } },
  { call: 'project:detectMobile', params: { projectPath: '<home>' } }
]

/** The MCP task tools, called by an agent with no session of its own. */
const TOOLS: Step[] = [
  { tool: 'list_tasks', arguments: {} },
  { tool: 'create_task', arguments: { project_name: 'board', title: 'From an agent' } },
  {
    tool: 'create_task',
    arguments: {
      project_name: 'board',
      title: 'Full',
      description: 'All of it',
      status: 'in_progress',
      branch: 'full',
      use_worktree: true,
      assigned_agent: 'claude'
    }
  },
  { tool: 'create_task', arguments: { project_name: 'nowhere', title: 'Lost' } },
  {
    tool: 'create_task',
    arguments: { project_name: 'board', title: 'Bad', assigned_agent: 'nobody' }
  },
  { tool: 'list_tasks', arguments: { project_name: 'board', status: 'in_progress' } },
  { tool: 'list_tasks', arguments: { assigned_agent: 'claude' } },
  { tool: 'list_tasks', arguments: { workspace_id: 'personal' } },
  { tool: 'get_task', arguments: { id: '$1' } },
  { tool: 'get_task', arguments: { id: 'ghost' } },
  { tool: 'update_task', arguments: { id: '$1', title: 'Retitled', status: 'done' } },
  { tool: 'update_task', arguments: { id: '$1', order: 0 } },
  { tool: 'update_task', arguments: { id: '$2', status: 'todo', use_worktree: false } },
  { tool: 'update_task', arguments: { id: 'ghost', title: 'x' } },
  { tool: 'archive_task', arguments: { id: '$1' } },
  { tool: 'list_tasks', arguments: {} },
  { tool: 'list_tasks', arguments: { include_archived: true } },
  { tool: 'unarchive_task', arguments: { id: '$1' } },
  { tool: 'archive_task', arguments: { id: 'ghost' } },
  { tool: 'get_my_context', arguments: { cwd: '<work>' } },
  { tool: 'get_my_context', arguments: { task_id: '$2' } },
  { tool: 'get_config', arguments: {} },
  { tool: 'delete_task', arguments: { id: '$2' } },
  { tool: 'delete_task', arguments: { id: '$2' } },
  { tool: 'list_tasks', arguments: { include_archived: true } }
]

const TASK_TOOLS = [
  'list_tasks',
  'create_task',
  'get_task',
  'update_task',
  'delete_task',
  'archive_task',
  'unarchive_task',
  'get_my_context'
]

/** An MCP session on vornd's `/mcp`. */
async function mcp(port: number): Promise<(method: string, params?: unknown) => Promise<unknown>> {
  let session = ''
  let next = 1
  const post = async (body: unknown): Promise<unknown> => {
    const res = await fetch(`http://127.0.0.1:${port}/mcp`, {
      method: 'POST',
      headers: {
        Accept: 'application/json, text/event-stream',
        'Content-Type': 'application/json',
        Authorization: `Bearer ${CREDENTIAL}`,
        ...(session ? { 'Mcp-Session-Id': session } : {})
      },
      body: JSON.stringify(body)
    })
    session = res.headers.get('mcp-session-id') ?? session
    const text = await res.text()
    return text ? JSON.parse(text) : null
  }
  await post({
    jsonrpc: '2.0',
    id: next++,
    method: 'initialize',
    params: {
      protocolVersion: '2025-06-18',
      capabilities: {},
      clientInfo: { name: 'replay', version: '1' }
    }
  })
  await post({ jsonrpc: '2.0', method: 'notifications/initialized' })
  return async (method, params) => {
    const answer = (await post({ jsonrpc: '2.0', id: next++, method, params })) as {
      result?: unknown
      error?: unknown
    }
    return answer.error ? { error: answer.error } : answer.result
  }
}

describe.runIf(!!builtVornd)('Tasks as they answered before the built-in', () => {
  let served: Served
  let client: Watcher
  let dirs: Record<string, string>

  beforeAll(async () => {
    served = await startServed({ credential: CREDENTIAL })
    client = await Watcher.open(served.port, CREDENTIAL)
    const work = servedDir('vorn-replay-work-')
    dirs = { data: served.dataDir, home: served.home, work }
    fs.writeFileSync(path.join(work, 'pic.jpg'), 'jpeg')
    fs.mkdirSync(path.join(work, 'app'))
    fs.writeFileSync(path.join(work, 'app', 'package.json'), '{"dependencies":{"expo":"1"}}')
    const config = (await client.result('config:load')) as Record<string, unknown>
    await client.result('config:save', {
      ...config,
      projects: [
        { name: 'board', path: work, preferredAgents: ['claude'], workspaceId: 'personal' },
        { name: 'other', path: work, preferredAgents: ['claude'], workspaceId: 'work' }
      ],
      workspaces: [
        { id: 'personal', name: 'Personal', order: 0 },
        { id: 'work', name: 'Work', order: 1 }
      ]
    })
  })

  afterAll(async () => {
    client?.close()
    await served?.stop()
  })

  /** Runs `steps`, each answer normalized, with how many `config:changed` it was followed by. */
  async function run(
    steps: Step[],
    answer: (step: Step, made: string[]) => Promise<unknown>
  ): Promise<Recorded[]> {
    const normalize = normalizer(dirs)
    const made: string[] = []
    const out: Recorded[] = []
    const changes = (): number => client.toldBy('config:changed').length
    for (const step of steps) {
      await new Promise((r) => setTimeout(r, 150))
      const before = changes()
      const got = await answer(step, made)
      await new Promise((r) => setTimeout(r, 150))
      out.push({ step, answer: normalize(got), changed: changes() - before })
    }
    return out
  }

  /** The fixture `name`: written from `recorded` when recording, else read. */
  function fixture(name: string, recorded: unknown): unknown {
    const file = path.join(FIXTURES, `${name}.json`)
    if (RECORD) {
      fs.mkdirSync(FIXTURES, { recursive: true })
      fs.writeFileSync(file, `${JSON.stringify(recorded, null, 2)}\n`)
      return recorded
    }
    return JSON.parse(fs.readFileSync(file, 'utf-8'))
  }

  it('answers every task call as it did', async () => {
    const withDirs = (v: unknown): unknown =>
      JSON.parse(
        Object.entries(dirs).reduce(
          (s, [name, dir]) => s.split(`<${name}>`).join(dir),
          JSON.stringify(v ?? null)
        )
      )
    const recorded = await run(CALLS, async (step, made) => {
      if (!('call' in step)) return null
      const frame = await client.call(step.call, withDirs(resolve(step.params, made)))
      const result = frame.result as { task?: { id?: string } } | undefined
      if (step.call === 'task:create' && result?.task?.id) made.push(result.task.id)
      return frame.error
        ? { error: (frame.error as { message: string }).message }
        : (frame.result ?? null)
    })
    expect(recorded).toEqual(fixture('calls', recorded))
  })

  it('answers every MCP task tool as it did', async () => {
    for (const task of (await client.result('task:list', {})) as Array<{ id: string }>) {
      await client.result('task:delete', { id: task.id })
    }
    const send = await mcp(served.port)
    const listed = (await send('tools/list')) as { tools: Array<{ name: string }> }
    const names = listed.tools.map((t) => t.name)
    const tools = listed.tools.filter((t) => TASK_TOOLS.includes(t.name))
    const withDirs = (v: unknown): unknown =>
      JSON.parse(
        Object.entries(dirs).reduce(
          (s, [name, dir]) => s.split(`<${name}>`).join(dir),
          JSON.stringify(v ?? {})
        )
      )
    const recorded = await run(TOOLS, async (step, made) => {
      if (!('tool' in step)) return null
      const result = (await send('tools/call', {
        name: step.tool,
        arguments: withDirs(resolve(step.arguments, made))
      })) as { content?: Array<{ text?: string }> }
      const said = result?.content?.[0]?.text ?? ''
      const created = /"id": "([0-9a-f-]{36})"/.exec(said)
      if (step.tool === 'create_task' && created) made.push(created[1])
      return result
    })
    const answered = { names, tools, calls: recorded }
    expect(answered).toEqual(fixture('mcp', answered))
  })

  it('leaves the board as it was', async () => {
    await until('quiet', () => true)
    const normalize = normalizer(dirs)
    const board = normalize(await client.result('task:list', { includeDescription: true }))
    expect(board).toEqual(fixture('board', board))
  })
})
