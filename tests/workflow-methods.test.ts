import { describe, it, expect, beforeAll, beforeEach, afterAll, vi } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import WebSocket from 'ws'
import type { RpcResponse } from '@vornrun/shared/protocol'

/**
 * Listing workflows, and switching one on.
 *
 * Driven over a real socket rather than by calling the handlers, because the
 * question is whether a client can reach these at all — that was the whole gap.
 * `workflow:get` needs an id and nothing produced one, so a workflow that had
 * never run was unreachable from the phone entirely.
 *
 * What this suite cannot do is prove the write lands. It replaces the database
 * module, so `dbUpdateWorkflow` here is a hand-written stand-in for the real
 * one — and PR #488 is the standing lesson about that: thirty-one tests stayed
 * green against a `dbUpdateTask` that had no `project_name` branch at all,
 * because the mock's idea of the column list was somebody's belief rather than
 * the function's own. `tests/database-workflow-enabled.test.ts` runs against
 * the real table for exactly that reason, and removing the real `enabled`
 * branch turns that one red while leaving this one green.
 */

const TEST_CREDENTIAL = 'workflow-methods-test-credential'

/** Whatever the runner already had, so teardown can put it back. */
const PRIOR_BOOTSTRAP_TOKEN = process.env.SECRET_VORN_BOOTSTRAP_TOKEN

/** The columns `dbUpdateWorkflow` can actually write, in `database.ts` order. */
const WORKFLOW_COLUMNS = vi.hoisted(
  () =>
    new Set([
      'name',
      'nodes',
      'edges',
      'icon',
      'iconColor',
      'enabled',
      'staggerDelayMs',
      'workspaceId'
    ])
)

const store = vi.hoisted(() => ({
  workflows: new Map<string, Record<string, unknown>>(),
  /** Every `config:changed` the server decided to send. */
  notified: 0
}))

vi.mock('node-pty', () => ({ default: { spawn: vi.fn() }, spawn: vi.fn() }))

/**
 * Booting a server probes Tailscale, and the probe is a real process.
 *
 * On a machine with Tailscale.app installed it spawns a GUI helper that does not
 * always die when the timeout fires, so every run of every server-booting test
 * file leaves one behind. Nothing here is about trusted origins.
 */
vi.mock('../packages/server/src/tailscale', () => ({
  getTailscaleStatus: vi.fn(async () => ({ running: false, selfIP: '', selfDNSName: '' })),
  clearBinaryCache: vi.fn()
}))

vi.mock('../packages/server/src/database', () => ({
  getDb: vi.fn(),
  closeDatabase: vi.fn(),
  initDatabase: vi.fn(),
  getDataDir: vi.fn(() => '/tmp/vorn-workflow-methods-test'),
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
  loadFullConfig: vi.fn(() => ({
    version: 1,
    defaults: { shell: '/bin/zsh', fontSize: 14, theme: 'dark' },
    projects: [],
    workflows: [],
    remoteHosts: [],
    tasks: [],
    workspaces: []
  })),
  saveFullConfig: vi.fn(),

  // ── the task table: names only ─────────────────────────────────
  // These exist so the mocked module has the exports `registerAllMethods`
  // imports. Nothing in this file calls them, and a copied implementation would
  // be worse than none: it reached for a `store.tasks` this file does not have
  // and filtered through the workflow column list, which is not the task one.
  // `tests/task-write-methods.test.ts` is where task writes are tested.
  dbListTasks: vi.fn(() => []),
  dbGetTask: vi.fn(() => null),
  dbInsertTask: vi.fn(),
  dbUpdateTask: vi.fn(),
  dbDeleteTask: vi.fn(),
  dbGetMaxTaskOrder: vi.fn(() => -1),
  dbGetProject: vi.fn(() => null),

  dbListProjects: vi.fn(() => []),
  // ── the workflow table, for real ──────────────────────────────
  dbListWorkflows: vi.fn(() => [...store.workflows.values()]),
  dbGetWorkflow: vi.fn((id: string) => store.workflows.get(id) ?? null),
  dbUpdateWorkflow: vi.fn((id: string, updates: Record<string, unknown>) => {
    const row = store.workflows.get(id)
    // The count, as the real one returns: zero rows matched is how an unknown id
    // is answered, and a mock that returned nothing would make every call look
    // like a miss.
    if (!row) return 0
    // The real `dbUpdateWorkflow` builds its SET list from a fixed set of
    // columns and ignores anything else. Mirrored, because a mock that accepts
    // any key lets a handler pass here while writing nothing in production --
    // which is the state `project_name` was in before PR #488.
    for (const [key, value] of Object.entries(updates)) {
      if (!WORKFLOW_COLUMNS.has(key)) continue
      row[key] = value
    }
    return 1
  }),
  dbInsertWorkflow: vi.fn(),
  dbDeleteWorkflow: vi.fn(),
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
  dbGetWorkflowRunByConnectorInboxId: vi.fn(() => null),
  loadWorkspaces: vi.fn(() => [])
}))

let serverPort: number
let serverClose: () => Promise<void>
let ws: WebSocket
let nextId = 1

/** A home of its own: booting a server starts a hook server, which claims files. */
let home: string | null = null
let realHome: string | undefined
let realProfile: string | undefined

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
    const timeout = setTimeout(() => {
      ws.off('message', handler)
      reject(new Error(`Timeout: ${method}`))
    }, 5000)
    ws.on('message', handler)
    ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
  })
}

function seed(workflows: Record<string, unknown>[]): void {
  store.workflows.clear()
  for (const workflow of workflows) {
    store.workflows.set(workflow.id as string, { ...workflow })
  }
}

describe('workflow read and enable methods', () => {
  beforeAll(async () => {
    // `hook-server` claims `~/.vorn/{hook-owner,port,token}` and `hook-installer`
    // writes `~/.claude/settings.json`, both resolved from `os.homedir()` with no
    // data directory involved. A booted server therefore registers itself as the
    // machine's hook endpoint unless HOME is moved first. Both variables, since
    // `homedir()` reads USERPROFILE on Windows.
    realHome = process.env.HOME
    realProfile = process.env.USERPROFILE
    home = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-wf-methods-'))
    process.env.HOME = home
    process.env.USERPROFILE = home

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
    ws.on('message', (raw) => {
      const msg = JSON.parse(raw.toString()) as { method?: string }
      if (msg.method === 'config:changed') store.notified += 1
    })
  }, 15000)

  afterAll(async () => {
    ws?.close()
    if (PRIOR_BOOTSTRAP_TOKEN === undefined) delete process.env.SECRET_VORN_BOOTSTRAP_TOKEN
    else process.env.SECRET_VORN_BOOTSTRAP_TOKEN = PRIOR_BOOTSTRAP_TOKEN
    // Assignment cannot restore an absence: `process.env.X = undefined` stores
    // the string "undefined".
    if (realHome === undefined) delete process.env.HOME
    else process.env.HOME = realHome
    if (realProfile === undefined) delete process.env.USERPROFILE
    else process.env.USERPROFILE = realProfile
    // `app.close()` does not stop the hook server -- that only happens in the
    // signal shutdown path -- so it would keep a listener open on a socket, and
    // its files inside a temp home about to be deleted underneath it.
    const { hookServer } = await import('../packages/server/src/hook-server')
    hookServer.stop()
    await serverClose?.()
    if (home) fs.rmSync(home, { recursive: true, force: true })
  })

  beforeEach(() => {
    store.workflows.clear()
    store.notified = 0
  })

  describe('workflow:list', () => {
    it('hands back a workflow that has never run', async () => {
      // The reason the method exists. The phone lists runs, so a workflow with
      // none behind it appears nowhere -- "clean branches" on the real machine.
      seed([
        { id: 'wf-a', name: 'clean branches', enabled: false },
        { id: 'wf-b', name: 'Simple hello', enabled: true, lastRunStatus: 'success' }
      ])

      const res = await call<{ id: string; name: string }[]>('workflow:list')

      // Order is not asserted: the read is `SELECT * FROM workflows` with no
      // ORDER BY, so there is no order to promise and a test that depended on
      // one would be pinning an accident of insertion.
      expect(res.result?.map((w) => w.name).sort()).toEqual(['Simple hello', 'clean branches'])
    })

    it('is empty rather than absent when there are none', async () => {
      const res = await call<unknown[]>('workflow:list')

      expect(res.result).toEqual([])
      expect(res.error).toBeUndefined()
    })

    it('carries the trigger, because that is where the row gets its words', async () => {
      // Not trimmed, unlike `task:list`. The trigger is a node, so trimming
      // `nodes` would take the one thing the row needs -- and the whole list
      // measured 5.4 KB against 147 KB of task descriptions.
      seed([
        {
          id: 'wf-a',
          name: 'Simple hello',
          icon: 'Cloud',
          iconColor: '#3b82f6',
          enabled: true,
          nodes: [
            {
              id: 't',
              type: 'trigger',
              config: { triggerType: 'manual', inputs: [{ key: 'pr_number' }] }
            }
          ]
        }
      ])

      const res =
        await call<
          { icon: string; iconColor: string; nodes: { type: string; config: unknown }[] }[]
        >('workflow:list')
      const [workflow] = res.result ?? []

      expect(workflow?.icon).toBe('Cloud')
      expect(workflow?.iconColor).toBe('#3b82f6')
      expect(workflow?.nodes?.find((n) => n.type === 'trigger')).toBeTruthy()
    })
  })

  describe('workflow:setEnabled', () => {
    it('switches a schedule on', async () => {
      seed([{ id: 'wf-a', name: 'Default Task Workflow', enabled: false }])

      const res = await call<{ ok: boolean }>('workflow:setEnabled', {
        id: 'wf-a',
        enabled: true
      })

      expect(res.result).toEqual({ ok: true })
      expect(store.workflows.get('wf-a')?.enabled).toBe(true)
    })

    it('switches one off', async () => {
      seed([{ id: 'wf-a', name: 'Default Task Workflow', enabled: true }])

      await call('workflow:setEnabled', { id: 'wf-a', enabled: false })

      expect(store.workflows.get('wf-a')?.enabled).toBe(false)
    })

    it('tells everyone, so the desktop dot changes without a restart', async () => {
      // The desktop holds the configuration in a cache and is drawing this
      // workflow's dot right now. Without the broadcast it shows the old state
      // until something else happens to invalidate it.
      seed([{ id: 'wf-a', name: 'Default Task Workflow', enabled: false }])

      await call('workflow:setEnabled', { id: 'wf-a', enabled: true })

      expect(store.notified).toBe(1)
    })

    it('refuses an id that names no workflow, and says nothing happened', async () => {
      seed([{ id: 'wf-a', name: 'Alpha', enabled: false }])

      const res = await call<{ ok: boolean }>('workflow:setEnabled', {
        id: 'wf-gone',
        enabled: true
      })

      expect(res.result).toEqual({ ok: false })
      // No broadcast either: nothing changed, so telling every client to reload
      // would be a lie about the configuration.
      expect(store.notified).toBe(0)
    })

    it('leaves the others alone', async () => {
      seed([
        { id: 'wf-a', name: 'Alpha', enabled: false },
        { id: 'wf-b', name: 'Beta', enabled: false }
      ])

      await call('workflow:setEnabled', { id: 'wf-a', enabled: true })

      expect(store.workflows.get('wf-b')?.enabled).toBe(false)
    })
  })
})
