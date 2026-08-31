import { describe, it, expect, beforeAll, beforeEach, afterAll, vi } from 'vitest'
import WebSocket from 'ws'
import type { RpcResponse } from '@vornrun/shared/protocol'
import type { TaskConfig } from '@vornrun/shared/types'

/**
 * Writing a task over the socket.
 *
 * Until these methods existed the only way to write one was `config:save` with
 * the whole configuration attached, which is the round trip `task:list` was
 * added to avoid. These are the five that reach the row functions directly.
 *
 * Driven over a real WebSocket rather than by calling the handlers, because the
 * phone's path is the socket and the typed registry in `protocol.ts` is half of
 * what makes a method exist at all.
 *
 * The task table is a real store rather than a bag of `vi.fn()`s: these methods
 * are almost entirely about what they write, so a mock that records calls and
 * returns nothing would pass while writing the wrong thing.
 */

const TEST_CREDENTIAL = 'task-write-test-credential'

/**
 * Whatever the runner already had, so teardown can put it back.
 *
 * Deleting the variable outright would take it from a run that had set one
 * before this file was imported.
 */
const PRIOR_BOOTSTRAP_TOKEN = process.env.SECRET_VORN_BOOTSTRAP_TOKEN

/**
 * The columns `dbUpdateTask` can actually write, in `database.ts` order.
 *
 * Hoisted like `store` below it. The mock reads this from inside a function
 * body, so a plain `const` happens to be safe -- but a `vi.mock` factory is
 * lifted above the imports, and anything it touches while it is being built
 * would find this still in its dead zone. Not worth leaving as the one binding
 * in this file that depends on where it is read from.
 */
const WRITABLE = vi.hoisted(
  () =>
    new Set([
      'projectName',
      'title',
      'description',
      'status',
      'order',
      'branch',
      'useWorktree',
      'assignedAgent',
      'assignedSessionId',
      'agentSessionId',
      'updatedAt',
      'completedAt',
      'archivedAt',
      'sourceConnectorId',
      'sourceExternalUrl',
      'sourceExternalId'
    ])
)

const store = vi.hoisted(() => ({
  tasks: new Map<string, Record<string, unknown>>(),
  projects: new Set<string>(),
  /** Every `config:changed` the server decided to send. */
  notified: 0
}))

vi.mock('node-pty', () => ({ default: { spawn: vi.fn() }, spawn: vi.fn() }))

/**
 * Booting a server probes Tailscale, and the probe is a real process.
 *
 * `startServer` calls `refreshTrustedOrigins`, which shells out to the
 * Tailscale binary with `execFile`. On a machine where Tailscale.app is
 * installed that spawns a GUI helper which does not always die when the 10s
 * timeout fires, so every run of every server-booting test file leaves one
 * behind. Nothing here is about trusted origins; mock it and spawn nothing.
 */
vi.mock('../packages/server/src/tailscale', () => ({
  getTailscaleStatus: vi.fn(async () => ({ running: false, selfIP: '', selfDNSName: '' })),
  clearBinaryCache: vi.fn()
}))

// The keys are constrained to names the module actually exports. A mock naming a
// function the module does not have is a mock that can never be wrong: it stands
// in for nothing, the real import stays undefined, and whatever calls it fails
// into somebody's catch. Values are deliberately unconstrained -- these are
// stubs, not implementations.
vi.mock(
  '../packages/server/src/database',
  () =>
    ({
      closeDatabase: vi.fn(),
      initDatabase: vi.fn(),
      getDataDir: vi.fn(() => '/tmp/vorn-task-write-test'),
      dbGetOwnerUser: vi.fn(() => ({
        id: 'owner-1',
        name: 'test',
        role: 'owner' as const,
        createdAt: new Date().toISOString()
      })),
      dbInsertDeviceToken: vi.fn(),
      dbListDeviceTokens: vi.fn(() => []),
      dbGetDeviceTokenSecret: vi.fn(),
      dbRevokeDeviceToken: vi.fn(() => true),
      dbTouchDeviceToken: vi.fn(),
      loadConfig: vi.fn(() => ({
        version: 1,
        defaults: { shell: '/bin/zsh', fontSize: 14, theme: 'dark' },
        projects: [],
        workflows: [],
        remoteHosts: [],
        tasks: [],
        workspaces: []
      })),
      saveConfig: vi.fn(),

      // ── the task table, for real ──────────────────────────────────
      dbListTasks: vi.fn((projectName?: string) =>
        [...store.tasks.values()].filter((t) => !projectName || t.projectName === projectName)
      ),
      dbGetTask: vi.fn((id: string) => store.tasks.get(id) ?? null),
      dbInsertTask: vi.fn((task: TaskConfig) => {
        store.tasks.set(task.id, { ...task })
      }),
      dbUpdateTask: vi.fn((id: string, updates: Record<string, unknown>) => {
        const row = store.tasks.get(id)
        if (!row) return
        for (const [key, value] of Object.entries(updates)) {
          // Mirrors the real `dbUpdateTask`: an absent key leaves the column alone,
          // and only `completedAt` and `archivedAt` treat an explicit `undefined`
          // as "clear it". Getting this wrong here would hide the bug it exists to
          // catch — a reopened task keeping the date it was finished.
          if (value === undefined && key !== 'completedAt' && key !== 'archivedAt') continue
          // And the column whitelist, which this mock used not to have. Without it a
          // test could hand `dbUpdateTask` a field the real one silently drops and
          // watch it pass — which is exactly the state `projectName` was in before
          // it was added to the real function.
          if (!WRITABLE.has(key)) continue
          row[key] = value
        }
      }),
      dbDeleteTask: vi.fn((id: string) => {
        store.tasks.delete(id)
      }),
      dbGetMaxTaskOrder: vi.fn((projectName: string) =>
        [...store.tasks.values()]
          .filter((t) => t.projectName === projectName)
          .reduce((max, t) => Math.max(max, (t.order as number) ?? -1), -1)
      ),
      dbGetProject: vi.fn((name: string) =>
        store.projects.has(name) ? { name, path: '/tmp' } : null
      ),

      dbListProjects: vi.fn(() => []),
      dbListWorkflows: vi.fn(() => []),
      dbInsertWorkflow: vi.fn(),
      dbUpdateWorkflow: vi.fn(),
      dbDeleteWorkflow: vi.fn(),
      dbGetWorkflow: vi.fn(),
      dbSignalChange: vi.fn(),
      saveWorkflowRun: vi.fn(),
      listWorkflowRuns: vi.fn(() => []),
      listWorkflowRunsByTask: vi.fn(() => []),
      updateWorkflowRunStatus: vi.fn(),
      dbListSourceConnections: vi.fn(() => []),
      dbGetSourceConnection: vi.fn(),
      dbInsertSourceConnection: vi.fn(),
      dbUpdateSourceConnection: vi.fn(),
      dbDeleteSourceConnection: vi.fn(),
      dbGetTaskSourceLink: vi.fn(),
      dbGetTaskSourceLinkByExternalId: vi.fn(),
      dbFindTaskByConnectorExternalId: vi.fn(),
      dbInsertTaskSourceLink: vi.fn(),
      dbUpdateTaskSourceLink: vi.fn(),
      dbReleaseConnectorInboxLeases: vi.fn(),
      dbCountActiveConnectorInboxLeases: vi.fn(() => 0),
      dbClaimConnectorInbox: vi.fn(() => []),
      dbGetWorkflowRunByConnectorInboxId: vi.fn(() => null)
    }) satisfies Partial<Record<keyof typeof import('../packages/server/src/database'), unknown>>
)

let serverPort: number
let serverClose: () => Promise<void>
let ws: WebSocket
let nextId = 1

function call<T = unknown>(
  method: string,
  params?: unknown
): Promise<RpcResponse & { result?: T }> {
  const id = nextId++
  return new Promise((resolve, reject) => {
    const handler = (raw: WebSocket.RawData) => {
      const msg = JSON.parse(raw.toString()) as RpcResponse
      if (msg.id !== id) return
      ws.off('message', handler)
      clearTimeout(timeout)
      resolve(msg as RpcResponse & { result?: T })
    }
    // Detached on the way out as well as on the way in: a call that times out
    // and leaves its listener behind makes every later call in a failing run
    // answer to a socket nobody is reading for.
    const timeout = setTimeout(() => {
      ws.off('message', handler)
      reject(new Error(`Timeout: ${method}`))
    }, 5000)
    ws.on('message', handler)
    ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
  })
}

/** Create a task and return it, failing loudly rather than returning undefined. */
async function create(fields: Record<string, unknown> = {}): Promise<TaskConfig> {
  const res = await call<{ ok: boolean; task?: TaskConfig }>('task:create', {
    projectName: 'vorn',
    title: 'Fix SSH prompt listener double-dispose bug',
    ...fields
  })
  expect(res.result?.ok).toBe(true)
  return res.result!.task!
}

describe('task write methods', () => {
  beforeAll(async () => {
    process.env.SECRET_VORN_BOOTSTRAP_TOKEN = TEST_CREDENTIAL
    const { startServer } = await import('../packages/server/src/index')

    const origWrite = process.stdout.write.bind(process.stdout)
    process.stdout.write = (() => true) as typeof process.stdout.write
    try {
      const started = await startServer({ port: 0 })
      serverPort = started.port
      serverClose = async () => {
        await started.app.close()
      }
    } finally {
      process.stdout.write = origWrite
    }

    ws = new WebSocket(`ws://127.0.0.1:${serverPort}/ws`, {
      headers: { Authorization: `Bearer ${TEST_CREDENTIAL}` }
    })
    await new Promise<void>((resolve, reject) => {
      ws.on('open', () => resolve())
      ws.on('error', reject)
    })
    // No `subscribe:set`, so this client is unfiltered and hears everything —
    // which is what lets the broadcast assertions below be about the server's
    // decision to send rather than about a subscription.
    ws.on('message', (raw) => {
      const msg = JSON.parse(raw.toString()) as { method?: string }
      if (msg.method === 'config:changed') store.notified += 1
    })
  }, 15000)

  afterAll(async () => {
    ws?.close()
    if (PRIOR_BOOTSTRAP_TOKEN === undefined) delete process.env.SECRET_VORN_BOOTSTRAP_TOKEN
    else process.env.SECRET_VORN_BOOTSTRAP_TOKEN = PRIOR_BOOTSTRAP_TOKEN
    await serverClose?.()
  })

  beforeEach(() => {
    store.tasks.clear()
    store.projects.clear()
    store.projects.add('vorn')
    store.notified = 0
  })

  describe('create', () => {
    it('writes the task and hands back the row', async () => {
      const task = await create({ description: 'Add a disposed flag.' })

      expect(task.id).toBeTruthy()
      expect(task.status).toBe('todo')
      expect(task.description).toBe('Add a disposed flag.')
      expect(store.tasks.get(task.id)).toBeDefined()
    })

    it('refuses a project that does not exist', async () => {
      const res = await call<{ ok: boolean }>('task:create', {
        projectName: 'not-a-project',
        title: 'Orphan'
      })
      expect(res.result?.ok).toBe(false)
      expect(store.tasks.size).toBe(0)
    })

    it('puts each new task after the last one in its project', async () => {
      const first = await create({ title: 'First' })
      const second = await create({ title: 'Second' })
      expect(second.order).toBe(first.order + 1)
    })

    it('stamps completedAt when it is created already finished', async () => {
      const task = await create({ status: 'done' })
      expect(task.completedAt).toBeTruthy()
    })

    it('leaves completedAt off an unfinished task', async () => {
      const task = await create({ status: 'in_progress' })
      expect(task.completedAt).toBeUndefined()
    })
  })

  describe('moving a task to another project', () => {
    it('writes the project, which the row function could not do at all before', async () => {
      store.projects.add('dev')
      const task = await create()

      const res = await call<{ ok: boolean }>('task:update', {
        id: task.id,
        projectName: 'dev'
      })

      expect(res.result?.ok).toBe(true)
      expect(store.tasks.get(task.id)?.projectName).toBe('dev')
    })

    it('puts it at the end of the board it arrives on', async () => {
      // `order` is per project. Carried across it keeps a place that means
      // nothing where it lands -- and from 0 into a board that already has a 0
      // it lands on top of something, which no index forbids and `task:reorder`
      // would preserve rather than repair.
      store.projects.add('dev')
      const moving = await create({ title: 'Moving' })
      const settled = await create({ projectName: 'dev', title: 'Already there' })

      expect(moving.order).toBe(0)
      expect(settled.order).toBe(0)

      await call('task:update', { id: moving.id, projectName: 'dev' })

      expect(store.tasks.get(moving.id)?.order).toBe(1)
      expect(store.tasks.get(settled.id)?.order).toBe(0)
    })

    it('refuses a project that does not exist', async () => {
      const task = await create()

      const res = await call<{ ok: boolean }>('task:update', {
        id: task.id,
        projectName: 'not-a-project'
      })

      // A task in a project nothing can run is a task that has been lost.
      expect(res.result?.ok).toBe(false)
      expect(store.tasks.get(task.id)?.projectName).toBe('vorn')
    })

    it('leaves the project alone when it is not asked about', async () => {
      const task = await create()
      await call('task:update', { id: task.id, title: 'Renamed' })

      expect(store.tasks.get(task.id)?.projectName).toBe('vorn')
      expect(store.tasks.get(task.id)?.order).toBe(0)
    })

    it('does not renumber when the project sent is the one it is already in', async () => {
      const first = await create({ title: 'First' })
      await create({ title: 'Second' })

      await call('task:update', { id: first.id, projectName: 'vorn' })

      expect(store.tasks.get(first.id)?.order).toBe(0)
    })
  })

  describe('update', () => {
    it('changes only what was sent', async () => {
      const task = await create({ description: 'Original' })
      const res = await call<{ ok: boolean; task?: TaskConfig }>('task:update', {
        id: task.id,
        title: 'Renamed'
      })

      expect(res.result?.ok).toBe(true)
      expect(res.result?.task?.title).toBe('Renamed')
      expect(res.result?.task?.description).toBe('Original')
    })

    it('refuses an id that names nothing', async () => {
      const res = await call<{ ok: boolean }>('task:update', { id: 'nope', title: 'x' })
      expect(res.result?.ok).toBe(false)
    })

    it('ignores columns it does not advertise', async () => {
      // The params type is erased at run time, so a client can send anything.
      // `dbUpdateTask` writes `archivedAt` on mere presence, which would file
      // away an open task and walk straight past the rule `task:archive`
      // enforces — so the handler names its fields rather than spreading.
      const task = await create({ status: 'todo' })
      await call('task:update', {
        id: task.id,
        title: 'Renamed',
        archivedAt: '2026-01-01T00:00:00.000Z',
        completedAt: '2026-01-01T00:00:00.000Z',
        assignedSessionId: 'someone-elses-session',
        order: 999
      })

      const row = store.tasks.get(task.id)!
      expect(row.title).toBe('Renamed')
      expect(row.archivedAt).toBeUndefined()
      expect(row.completedAt).toBeUndefined()
      expect(row.assignedSessionId).toBeUndefined()
      expect(row.order).toBe(task.order)
    })

    it('leaves archiving to the method that checks whether it is allowed', async () => {
      const task = await create({ status: 'todo' })
      await call('task:update', { id: task.id, archivedAt: '2026-01-01T00:00:00.000Z' })
      expect(store.tasks.get(task.id)?.archivedAt).toBeUndefined()

      // And that method still refuses, which is the rule being protected.
      const res = await call<{ ok: boolean }>('task:archive', { id: task.id, archived: true })
      expect(res.result?.ok).toBe(false)
    })

    it('clears completedAt and archivedAt when a finished task is reopened', async () => {
      const task = await create({ status: 'done' })
      await call('task:archive', { id: task.id, archived: true })
      expect(store.tasks.get(task.id)?.archivedAt).toBeTruthy()

      await call('task:update', { id: task.id, status: 'in_progress' })

      const row = store.tasks.get(task.id)!
      expect(row.completedAt).toBeUndefined()
      expect(row.archivedAt).toBeUndefined()
    })
  })

  describe('setStatus', () => {
    it('stamps completedAt on the way into a finished state', async () => {
      const task = await create()
      await call('task:setStatus', { id: task.id, status: 'done' })
      expect(store.tasks.get(task.id)?.completedAt).toBeTruthy()
    })

    it('clears it again on the way out', async () => {
      const task = await create({ status: 'done' })
      await call('task:setStatus', { id: task.id, status: 'todo' })
      expect(store.tasks.get(task.id)?.completedAt).toBeUndefined()
    })

    it('marks the row as touched, like every other write does', async () => {
      const task = await create()
      // Aged by hand rather than by the clock: creating and moving a task can
      // land in the same millisecond, and an assertion that depends on them not
      // doing so is one that goes red on a machine faster than this one.
      const stale = '2020-01-01T00:00:00.000Z'
      store.tasks.set(task.id, { ...task, updatedAt: stale })

      await call('task:setStatus', { id: task.id, status: 'in_progress' })

      expect(store.tasks.get(task.id)?.updatedAt).not.toBe(stale)
    })
  })

  describe('archive', () => {
    it('files away a finished task', async () => {
      const task = await create({ status: 'done' })
      const res = await call<{ ok: boolean }>('task:archive', { id: task.id, archived: true })

      expect(res.result?.ok).toBe(true)
      expect(store.tasks.get(task.id)?.archivedAt).toBeTruthy()
    })

    it('refuses one that is still open', async () => {
      const task = await create({ status: 'todo' })
      const res = await call<{ ok: boolean }>('task:archive', { id: task.id, archived: true })

      // The same rule `archive_task` enforces on the MCP side. A todo hidden
      // from the board is a todo that is lost.
      expect(res.result?.ok).toBe(false)
      expect(store.tasks.get(task.id)?.archivedAt).toBeUndefined()
    })

    it('restores one, whatever its status', async () => {
      const task = await create({ status: 'cancelled' })
      await call('task:archive', { id: task.id, archived: true })

      const res = await call<{ ok: boolean }>('task:archive', { id: task.id, archived: false })
      expect(res.result?.ok).toBe(true)
      expect(store.tasks.get(task.id)?.archivedAt).toBeUndefined()
    })
  })

  describe('reorder', () => {
    it('rewrites the order of everything named', async () => {
      const a = await create({ title: 'A' })
      const b = await create({ title: 'B' })
      const c = await create({ title: 'C' })

      const res = await call<{ ok: boolean }>('task:reorder', { ids: [c.id, a.id, b.id] })

      expect(res.result?.ok).toBe(true)
      expect(store.tasks.get(c.id)?.order).toBe(0)
      expect(store.tasks.get(a.id)?.order).toBe(1)
      expect(store.tasks.get(b.id)?.order).toBe(2)
    })

    it('skips ids that name nothing without failing the rest', async () => {
      const a = await create({ title: 'A' })
      const res = await call<{ ok: boolean }>('task:reorder', { ids: ['ghost', a.id] })

      expect(res.result?.ok).toBe(true)
      // The ghost does not take a place with it. Numbering the list would have
      // pushed A to 1 on account of a task that does not exist.
      expect(store.tasks.get(a.id)?.order).toBe(0)
    })

    it('leaves the tasks it was not given where they were', async () => {
      const a = await create({ title: 'A' })
      const b = await create({ title: 'B' })
      const c = await create({ title: 'C' })
      const d = await create({ title: 'D' })

      // Half the board, reversed. The other half must not be landed on.
      await call('task:reorder', { ids: [c.id, a.id] })

      expect(store.tasks.get(c.id)?.order).toBe(0)
      expect(store.tasks.get(a.id)?.order).toBe(2)
      expect(store.tasks.get(b.id)?.order).toBe(1)
      expect(store.tasks.get(d.id)?.order).toBe(3)

      const orders = [a, b, c, d].map((t) => store.tasks.get(t.id)?.order)
      expect(new Set(orders).size).toBe(4)
    })

    it('reads an id sent twice as the once it can mean', async () => {
      const a = await create({ title: 'A' })
      const b = await create({ title: 'B' })
      const c = await create({ title: 'C' })
      const d = await create({ title: 'D' })

      // A repeat puts its order into the slots twice, and the second copy is a
      // place no task can occupy -- the spare pushed a later task onto an order
      // another one already held. Here that used to land A and C both on 1.
      await call('task:reorder', { ids: [b.id, a.id, c.id, b.id] })

      const orders = [a, b, c, d].map((t) => store.tasks.get(t.id)?.order)
      expect(new Set(orders).size).toBe(4)
      // Same answer as the list with the repeat taken out.
      expect(store.tasks.get(b.id)?.order).toBe(0)
      expect(store.tasks.get(a.id)?.order).toBe(1)
      expect(store.tasks.get(c.id)?.order).toBe(2)
      expect(store.tasks.get(d.id)?.order).toBe(3)
    })

    it('reports failure when nothing named exists', async () => {
      const res = await call<{ ok: boolean }>('task:reorder', { ids: ['ghost'] })
      expect(res.result?.ok).toBe(false)
    })

    it('says nothing when the order asked for is the order already there', async () => {
      const a = await create({ title: 'A' })
      const b = await create({ title: 'B' })
      const before = store.notified

      const res = await call<{ ok: boolean }>('task:reorder', { ids: [a.id, b.id] })

      await new Promise((resolve) => setTimeout(resolve, 50))
      // It succeeded -- the board is in the state asked for -- and wrote
      // nothing, so there is nothing for anyone to rebuild.
      expect(res.result?.ok).toBe(true)
      expect(store.notified).toBe(before)
    })
  })

  describe('delete', () => {
    it('removes the task', async () => {
      const task = await create()
      const res = await call<{ ok: boolean }>('task:delete', { id: task.id })

      expect(res.result?.ok).toBe(true)
      expect(store.tasks.has(task.id)).toBe(false)
    })

    it('refuses an id that names nothing', async () => {
      const res = await call<{ ok: boolean }>('task:delete', { id: 'ghost' })
      expect(res.result?.ok).toBe(false)
    })
  })

  /**
   * The half a handler test cannot see.
   *
   * Every one of these writes a row directly, so the cached configuration every
   * other client reads is stale until `notifyChanged` invalidates it — and that
   * same call is what broadcasts `config:changed`. A method that skips it looks
   * correct in a unit test and is invisible on every other screen.
   */
  describe('config:changed', () => {
    it('fires on every mutation', async () => {
      const task = await create()
      // A second task so the reorder below has something to swap with. Given one
      // id there is no order it could change, and a no-op proves nothing about
      // whether a real reorder is heard.
      const other = await create({ title: 'Second' })
      await call('task:update', { id: task.id, title: 'Renamed' })
      await call('task:setStatus', { id: task.id, status: 'done' })
      await call('task:archive', { id: task.id, archived: true })
      await call('task:reorder', { ids: [other.id, task.id] })
      await call('task:delete', { id: task.id })

      // The broadcast is a notification, so it arrives independently of the
      // replies awaited above.
      await new Promise((resolve) => setTimeout(resolve, 50))
      expect(store.notified).toBe(7)
    })

    it('stays quiet when nothing changed', async () => {
      await call('task:update', { id: 'ghost', title: 'x' })
      await call('task:delete', { id: 'ghost' })
      await call('task:create', { projectName: 'not-a-project', title: 'Orphan' })

      await new Promise((resolve) => setTimeout(resolve, 50))
      expect(store.notified).toBe(0)
    })
  })
})
