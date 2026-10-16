/**
 * Today's Workflows, replayed: scripted runs of every way a workflow starts
 * (by hand, by schedule, by webhook, by a task, by a connector's poll), its
 * gates (answered, sent back, rejected, timed out, and the global shortcut),
 * the artifacts a gate keeps, the workflow calls and MCP workflow tools,
 * import and export, and the workflows a new database is seeded with. The
 * answers were recorded from vornd before node types, triggers, the editor
 * and Artifacts became built-ins; each run must answer the same.
 *
 * Ids, times, ports, tokens, process ids and the test's directories are
 * normalized before comparing. `VORN_RECORD_WORKFLOWS=1` records the
 * fixtures again from the vornd built here.
 *
 * Runs where vornd and its session holder have been built.
 */
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import http from 'node:http'
import path from 'node:path'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import { builtSessiond, builtVornd, servedDir, startServed, type Served } from './helpers/served'
import { Watcher, until } from './helpers/real-server'

vi.setConfig({ testTimeout: 180_000, hookTimeout: 120_000 })

const CREDENTIAL = 'native-server-workflows-replay-credential'
const FIXTURES = path.join(__dirname, 'fixtures', 'workflows-replay')
const RECORD = process.env.VORN_RECORD_WORKFLOWS === '1'
const NATIVE_FIXTURE = path.resolve(__dirname, 'fixtures/native-connector.mjs')

type Json = Record<string, unknown>
type Run = {
  runId: string
  workflowId: string
  status: string
  nodeStates: Array<Json & { nodeId: string; status: string }>
}

const pos = { x: 0, y: 0 }

/** An agent step's run, without what the machine's own agent CLI said: whether it started and on which task. */
function agentSteps(run: Run): unknown {
  return {
    status: run.status,
    steps: run.nodeStates.map((n) => ({
      nodeId: n.nodeId,
      status: n.status,
      session: typeof n.sessionId === 'string',
      taskId: n.taskId ?? null,
      agentType: n.agentType ?? null,
      projectName: n.projectName ?? null
    }))
  }
}

/** Ids, times, ports, tokens and directories as names that are the same on every run. */
function normalizer(dirs: () => Record<string, string>, ports: () => number[]) {
  const ids = new Map<string, string>()
  const tokens = new Map<string, string>()
  const text = (s: string): string => {
    let out = s
    for (const [name, dir] of Object.entries(dirs())) out = out.split(dir).join(`<${name}>`)
    for (const port of ports()) out = out.split(`:${port}`).join(':<port>')
    out = out.replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g, (id) => {
      if (!ids.has(id)) ids.set(id, `<id${ids.size + 1}>`)
      return ids.get(id)!
    })
    out = out.replace(/\b[0-9a-f]{32}\b|\b[A-Za-z0-9_-]{43}\b/g, (t) => {
      if (!tokens.has(t)) tokens.set(t, `<token${tokens.size + 1}>`)
      return tokens.get(t)!
    })
    out = out.replace(/\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?(Z|[+-]\d\d:\d\d)/g, '<time>')
    out = out.replace(/\b[0-9a-f]{40}\b/g, '<rev>')
    out = out.replace(/\w{3}, \d\d \w{3} \d{4} \d\d:\d\d:\d\d GMT/g, '<date>')
    out = out.replace(/\[\+\d+\.\ds\]/g, '[+<s>]')
    out = out.replace(/\(pid \d+\)/g, '(pid <pid>)')
    out = out.replace(/"shell": "[^"]*"/g, '"shell": "<shell>"')
    return out
  }
  const walk = (v: unknown): unknown => {
    if (typeof v === 'string') return text(v)
    if (Array.isArray(v)) return v.map(walk)
    if (v && typeof v === 'object') {
      return Object.fromEntries(
        Object.entries(v).map(([k, x]) => {
          // Clock readings: when, never what.
          if (/At$/.test(k) && typeof x === 'number') return [text(k), '<number>']
          // Inbox rows are numbered across every test's webhooks and polls.
          if (/^(connectorInboxId|inboxId)$/.test(k) && typeof x === 'number') {
            return [k, '<inbox>']
          }
          return [text(k), walk(x)]
        })
      )
    }
    return v
  }
  return walk
}

function workflow(
  id: string,
  trigger: Json,
  rest: Json[],
  edges: Array<[string, string, string?]>
): Json {
  return {
    id,
    name: id,
    icon: 'Zap',
    iconColor: '#ffffff',
    enabled: true,
    nodes: [
      { id: 't', type: 'trigger', label: 'Trigger', position: pos, config: trigger },
      ...rest
    ],
    edges: edges.map(([source, target, branch]) => ({
      id: `${source}-${target}`,
      source,
      target,
      ...(branch ? { conditionBranch: branch } : {})
    }))
  }
}

const step = (id: string, type: string, config: Json, extra: Json = {}): Json => ({
  id,
  type,
  label: id,
  slug: id,
  position: pos,
  config,
  ...extra
})
const script = (id: string, content: string, extra: Json = {}): Json =>
  step(id, 'script', { scriptType: 'bash', scriptContent: content }, extra)

/** The connector's repository, committed: what `git clone` installs. */
function connectorRepo(dir: string): void {
  fs.mkdirSync(dir, { recursive: true })
  fs.writeFileSync(
    path.join(dir, 'vorn-extension.toml'),
    [
      'id = "tickets"',
      'name = "Tickets"',
      'version = "1.0.0"',
      'api = "vorn:extension@0.9"',
      '',
      '[service]',
      'command = ["node", "index.js"]',
      '',
      '[declares]',
      'network = ["tickets.example.com"]',
      '',
      '[[connector.triggers]]',
      'type = "items"',
      'label = "Items"',
      '',
      '[[connector.actions]]',
      'type = "create"',
      'label = "Create"',
      '',
      '[[connector.actions.inputs]]',
      'key = "title"',
      'label = "Title"',
      ''
    ].join('\n')
  )
  fs.writeFileSync(path.join(dir, 'index.js'), fs.readFileSync(NATIVE_FIXTURE, 'utf-8'))
  fs.writeFileSync(path.join(dir, 'package.json'), JSON.stringify({ type: 'module' }))
  const env = {
    ...process.env,
    GIT_AUTHOR_NAME: 'Vorn',
    GIT_AUTHOR_EMAIL: 'vorn@example.invalid',
    GIT_COMMITTER_NAME: 'Vorn',
    GIT_COMMITTER_EMAIL: 'vorn@example.invalid'
  }
  const git = (...args: string[]): void => {
    execFileSync('git', args, { cwd: dir, env, stdio: 'ignore' })
  }
  git('init', '-q', '-b', 'main')
  git('add', '-A')
  git('commit', '-q', '-m', 'one')
}

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

const WORKFLOW_TOOLS = [
  'list_workflows',
  'create_workflow',
  'update_workflow',
  'delete_workflow',
  'execute_workflow',
  'list_workflow_runs',
  'stop_workflow_run',
  'resolve_gate',
  'get_workflow_schedule',
  'describe_workflow_nodes',
  'export_workflow',
  'import_workflow',
  'list_artifacts',
  'read_artifact',
  'read_artifact_comments',
  'publish_artifact'
]

describe.runIf(!!builtVornd && !!builtSessiond && process.platform !== 'win32')(
  'Workflows as they answered before node types, triggers, the editor and Artifacts were built-ins',
  () => {
    let served: Served
    let client: Watcher
    let hooks: http.Server
    let hookPort = 0
    let work = ''
    const heard: Array<{ url?: string; method?: string; body: string }> = []
    const dirs = (): Record<string, string> => ({
      data: served.dataDir,
      home: served.home,
      work
    })
    const ports = (): number[] => [served.port, hookPort].filter(Boolean)

    const runs = (workflowId: string): Promise<Run[]> =>
      client.result<Run[]>('workflowRun:list', { workflowId })

    async function settled(workflowId: string, count = 1): Promise<Run[]> {
      let list: Run[] = []
      await until(`${count} finished run(s) of ${workflowId}`, async () => {
        list = await runs(workflowId)
        return list.length >= count && list.every((r) => r.status !== 'running')
      })
      return list
    }

    async function waiting(workflowId: string, nodeId: string, round = 1): Promise<Run> {
      let run!: Run
      await until(`${workflowId} to wait at ${nodeId}, round ${round}`, async () => {
        run = (await runs(workflowId))[0]
        const gate = run?.nodeStates.find((n) => n.nodeId === nodeId)
        return gate?.status === 'waiting' && (gate.round ?? 1) === round
      })
      return run
    }

    const answer = async (method: string, params?: unknown): Promise<unknown> => {
      const frame = await client.call(method, params)
      return frame.error
        ? { error: (frame.error as { message: string }).message }
        : (frame.result ?? null)
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

    /** What a scenario answered, normalized, and what it answered before. */
    function replayed(name: string, recorded: unknown): { got: unknown; want: unknown } {
      const got = normalizer(dirs, ports)(recorded)
      return { got, want: fixture(name, got) }
    }

    beforeAll(async () => {
      hooks = http.createServer((req, res) => {
        let body = ''
        req.on('data', (c) => (body += c))
        req.on('end', () => {
          heard.push({ url: req.url, method: req.method, body })
          res.writeHead(200, { 'content-type': 'application/json' })
          res.end(JSON.stringify({ ok: true, path: req.url }))
        })
      })
      await new Promise<void>((r) => hooks.listen(0, '127.0.0.1', r))
      hookPort = (hooks.address() as { port: number }).port
      // An agent CLI of the test's own, so a step starts the same agent on every machine.
      const bin = servedDir('vorn-wf-replay-bin-')
      fs.writeFileSync(path.join(bin, 'codex'), '#!/bin/sh\necho "fake agent"\nexit 3\n', {
        mode: 0o755
      })
      served = await startServed({
        credential: CREDENTIAL,
        sessiond: true,
        env: {
          PATH: [bin, path.dirname(process.execPath), '/usr/bin', '/bin'].join(path.delimiter)
        }
      })
      client = await Watcher.open(served.port, CREDENTIAL)
      work = servedDir('vorn-wf-replay-work-')
      const config = (await client.result('config:load')) as Json
      await client.result('config:save', {
        ...config,
        projects: [{ name: 'p', path: work, preferredAgents: [], workspaceId: 'personal' }]
      })
    })

    afterAll(async () => {
      client?.close()
      hooks?.close()
      await served?.stop()
    })

    it('seeds a new database with the workflows it always had', async () => {
      const seeded = (await client.result('workflow:list')) as Json[]
      const { got, want } = replayed('seeded', seeded)
      expect(got).toEqual(want)
      // The seeded workflows start real agents on a task's move; none is wanted here.
      for (const wf of seeded) {
        await client.result('workflow:setEnabled', { id: wf.id, enabled: false })
      }
    })

    it('writes, reads and removes workflows as it did', async () => {
      const wf = workflow('crud', { triggerType: 'manual' }, [script('a', 'echo a')], [['t', 'a']])
      const out: unknown[] = []
      for (const [method, params] of [
        ['workflow:create', { workflow: wf }],
        ['workflow:create', {}],
        ['workflow:get', { id: 'crud' }],
        ['workflow:get', { id: 'ghost' }],
        ['workflow:update', { id: 'crud', updates: { name: 'Renamed', staggerDelayMs: 5 } }],
        ['workflow:update', { id: 'crud', updates: {} }],
        ['workflow:update', { id: 'ghost', updates: { name: 'x' } }],
        ['workflow:update', { id: 'crud' }],
        ['workflow:setEnabled', { id: 'crud', enabled: false }],
        ['workflow:setEnabled', { id: 'ghost', enabled: true }],
        ['workflow:get', { id: 'crud' }],
        ['workflow:delete', { id: 'crud' }],
        ['workflow:delete', { id: 'crud' }],
        ['workflow:delete', {}],
        ['workflow:run', { workflowId: 'ghost' }],
        ['workflow:runManual', { workflowId: 'ghost' }],
        ['workflowRun:list', { workflowId: 'ghost' }],
        ['webhook:info']
      ] as Array<[string, unknown]>) {
        out.push({ method, params, answer: await answer(method, params) })
      }
      const { got, want } = replayed('crud', out)
      expect(got).toEqual(want)
    })

    it('runs by hand: inputs, a condition, a loop, an HTTP request, a failure carried on, a retry and a rerun', async () => {
      const wf = workflow(
        'by-hand',
        {
          triggerType: 'manual',
          inputs: [{ key: 'name', label: 'Name', type: 'text', required: true }]
        },
        [
          script('hello', 'echo "hello {{inputs.name}}"'),
          step('check', 'condition', {
            variable: '{{steps.hello.output}}',
            operator: 'contains',
            value: 'hello'
          }),
          script('yes', 'echo yes'),
          script('no', 'echo no'),
          step('each', 'loop', {
            nodeType: 'loop',
            mode: 'forEach',
            items: '["x","y"]',
            bodyNodeIds: ['body'],
            maxIterations: 5
          }),
          script('body', 'echo "item {{loop.item}} {{loop.number}}"'),
          step('call', 'httpRequest', {
            method: 'POST',
            url: `http://127.0.0.1:${hookPort}/step/{{inputs.name}}`,
            headers: { 'x-run': '{{inputs.name}}' },
            body: '{"from":"{{steps.hello.output}}"}'
          }),
          script('fails', 'echo broken; exit 3', { onError: 'continue' }),
          script('after', 'echo "after {{steps.fails.status}} {{steps.call.output}}"')
        ],
        [
          ['t', 'hello'],
          ['hello', 'check'],
          ['check', 'yes', 'true'],
          ['check', 'no', 'false'],
          ['yes', 'each'],
          ['each', 'body'],
          ['each', 'call'],
          ['call', 'fails'],
          ['fails', 'after']
        ]
      )
      await client.result('workflow:create', { workflow: wf })
      const started = await answer('workflow:run', {
        workflowId: 'by-hand',
        context: { inputs: { name: 'Ada' } }
      })
      const [first] = await settled('by-hand')
      const manual = await answer('workflow:runManual', {
        workflowId: 'by-hand',
        inputs: { name: 'Grace' }
      })
      const afterManual = await settled('by-hand', 2)
      const failing = workflow(
        'fails',
        { triggerType: 'manual' },
        [script('ok', 'echo ok'), script('bad', 'echo bad; exit 1'), script('never', 'echo never')],
        [
          ['t', 'ok'],
          ['ok', 'bad'],
          ['bad', 'never']
        ]
      )
      await client.result('workflow:create', { workflow: failing })
      await answer('workflow:run', { workflowId: 'fails' })
      const [failed] = await settled('fails')
      const retried = await answer('workflow:retryRun', { runId: failed.runId })
      await settled('fails', 2)
      const rerun = await answer('workflow:rerun', { runId: failed.runId })
      const all = await settled('fails', 3)
      const target = await answer('workflow:run', { workflowId: 'by-hand', targetNodeId: 'hello' })
      const targeted = await settled('by-hand', 3)
      const { got, want } = replayed('by-hand', {
        started,
        first,
        manual,
        afterManual,
        failed,
        retried,
        rerun,
        all,
        target,
        targeted,
        heard: heard.splice(0),
        noTrigger: await answer('workflow:runManual', { workflowId: 'notrigger' })
      })
      expect(got).toEqual(want)
    })

    it('runs on a schedule that falls due, and says when a recurring one runs next', async () => {
      const runAt = new Date(Date.now() + 3_000).toISOString()
      await client.result('workflow:create', {
        workflow: workflow(
          'once',
          { triggerType: 'once', runAt },
          [script('tick', 'echo tick')],
          [['t', 'tick']]
        )
      })
      await client.result('workflow:create', {
        workflow: workflow(
          'every',
          { triggerType: 'recurring', cron: '0 9 * * 1', timezone: 'UTC' },
          [script('tock', 'echo tock')],
          [['t', 'tock']]
        )
      })
      const [run] = await settled('once')
      const { got, want } = replayed('schedule', {
        run,
        log: await answer('scheduler:getLog', 'once'),
        next: await answer('scheduler:getNextRun', 'every'),
        nextOnce: await answer('scheduler:getNextRun', 'once'),
        workflow: await answer('workflow:get', { id: 'once' })
      })
      expect(got).toEqual(want)
      await client.result('workflow:setEnabled', { id: 'every', enabled: false })
    })

    it('runs a webhook once per delivery', async () => {
      await client.result('workflow:create', {
        workflow: workflow(
          'hook',
          { triggerType: 'webhook', method: 'POST', token: 'tok-1' },
          [script('echo', 'echo "got {{trigger.body.n}} {{trigger.query.q}}"')],
          [['t', 'echo']]
        )
      })
      const post = (key: string, token = 'tok-1'): Promise<number> =>
        fetch(`http://127.0.0.1:${served.port}/wf-hooks/hook/${token}?q=z`, {
          method: 'POST',
          headers: { 'content-type': 'application/json', 'idempotency-key': key },
          body: JSON.stringify({ n: 7 })
        }).then((r) => r.status)
      const statuses = [await post('d1'), await post('d1'), await post('d1', 'wrong')]
      await settled('hook')
      await new Promise((r) => setTimeout(r, 1_000))
      statuses.push(await post('d2'))
      const list = await settled('hook', 2)
      const { got, want } = replayed('webhook', { statuses, runs: list })
      expect(got).toEqual(want)
    })

    it('asks at a gate, keeps its review page as an artifact, and takes every answer', async () => {
      const gated = (id: string, config: Json): Json =>
        workflow(
          id,
          { triggerType: 'manual' },
          [
            script('draft', 'echo "draft {{steps.review.round}}"'),
            step('review', 'approval', config),
            script('ship', 'echo "shipped {{steps.review.text}} {{steps.review.comment}}"')
          ],
          [
            ['t', 'draft'],
            ['draft', 'review'],
            ['review', 'ship']
          ]
        )
      await client.result('workflow:create', {
        workflow: gated('gate', {
          message: 'Look at {{steps.draft.output}}',
          view: '<h1>{{steps.draft.output}}</h1><p>Read me</p>',
          edit: '{{steps.draft.output}}',
          feedback: { from: 'draft', maxRounds: 3 }
        })
      })
      const out: Json = {}
      out.started = await answer('workflow:run', { workflowId: 'gate' })
      let run = await waiting('gate', 'review')
      out.asking = run
      out.waitingList = await answer('workflowRun:listWaiting')
      out.running = await answer('workflowRun:listRunning')
      out.forGate = await answer('artifact:forGate', { runId: run.runId, nodeId: 'review' })
      const artifact = (out.forGate as { artifact?: { id?: string } } | null)?.artifact
      out.artifact = artifact?.id ? await answer('artifact:get', { artifactId: artifact.id }) : null
      // The shortcut that answers an agent's permission request.
      out.shortcut = await answer('permission:resolve-top', { allow: true })
      await new Promise((r) => setTimeout(r, 500))
      out.afterShortcut = (await runs('gate'))[0].nodeStates.find((n) => n.nodeId === 'review')
      out.emptyChanges = await answer('workflow:resolveGate', {
        runId: run.runId,
        nodeId: 'review',
        decision: 'changes',
        comment: ''
      })
      if (artifact?.id) {
        out.comment = await answer('artifact:saveComment', {
          artifactId: artifact.id,
          version: 1,
          anchor: { quote: 'Read me' },
          body: 'Shorter, please'
        })
      }
      out.changes = await answer('workflow:resolveGate', {
        runId: run.runId,
        nodeId: 'review',
        decision: 'changes',
        comment: 'again'
      })
      run = await waiting('gate', 'review', 2)
      out.round2 = run
      out.badEdit = await answer('workflow:resolveGate', {
        runId: run.runId,
        nodeId: 'review',
        decision: 'odd'
      })
      out.approve = await answer('workflow:resolveGate', {
        runId: run.runId,
        nodeId: 'review',
        decision: 'approve',
        edited: 'final words',
        comment: 'fine'
      })
      out.done = (await settled('gate'))[0]
      out.notAsking = await answer('workflow:resolveGate', {
        runId: run.runId,
        nodeId: 'review',
        decision: 'approve'
      })
      out.resolved = client.toldBy('workflow:gateResolved')

      await answer('workflow:run', { workflowId: 'gate' })
      run = await waiting('gate', 'review')
      out.reject = await answer('workflow:resolveGate', {
        runId: run.runId,
        nodeId: 'review',
        decision: 'reject',
        comment: 'no'
      })
      out.rejected = (await settled('gate', 2))[0]

      await client.result('workflow:create', {
        workflow: gated('gate-timeout', { message: 'Quick', timeoutMs: 300 })
      })
      await answer('workflow:run', { workflowId: 'gate-timeout' })
      out.timedOut = (await settled('gate-timeout'))[0]

      await client.result('workflow:create', {
        workflow: gated('gate-stop', { message: 'Stop me' })
      })
      await answer('workflow:run', { workflowId: 'gate-stop' })
      run = await waiting('gate-stop', 'review')
      out.stop = await answer('workflow:stopRun', { runId: run.runId })
      out.stopped = (await settled('gate-stop'))[0]
      out.byTask = await answer('workflowRun:listByTask', { taskId: 'none' })
      const { got, want } = replayed('gates', out)
      expect(got).toEqual(want)
    })

    it('stops a long run', async () => {
      await client.result('workflow:create', {
        workflow: workflow(
          'long',
          { triggerType: 'manual' },
          [script('sleep', 'sleep 30'), script('later', 'echo later')],
          [
            ['t', 'sleep'],
            ['sleep', 'later']
          ]
        )
      })
      const started = (await client.result('workflow:run', { workflowId: 'long' })) as Run
      await until('the script to run', async () =>
        (await runs('long'))[0]?.nodeStates.some((n) => n.status === 'running')
      )
      const stop = await answer('workflow:stopRun', { runId: started.runId })
      const [run] = await settled('long')
      const { got, want } = replayed('stop', {
        stop,
        run,
        all: await answer('workflowRun:listAll')
      })
      expect(got).toEqual(want)
    })

    it('starts the workflows watching tasks, and an agent step takes its task and hands it back', async () => {
      const out: Json = {}
      await client.result('workflow:create', {
        workflow: workflow(
          'created',
          { triggerType: 'taskCreated', projectFilter: 'p' },
          [script('seen', 'echo "created {{task.title}} {{trigger.type}}"')],
          [['t', 'seen']]
        )
      })
      await client.result('workflow:create', {
        workflow: workflow(
          'moved',
          { triggerType: 'taskStatusChanged', fromStatus: 'todo', toStatus: 'in_progress' },
          [
            script(
              'seen',
              'echo "moved {{task.title}} {{trigger.fromStatus}} {{trigger.toStatus}}"'
            )
          ],
          [['t', 'seen']]
        )
      })
      await client.result('workflow:create', {
        workflow: workflow(
          'reopened',
          { triggerType: 'taskStatusChanged', toStatus: 'todo' },
          [script('seen', 'echo "back {{task.title}}"')],
          [['t', 'seen']]
        )
      })
      out.task = await answer('task:create', {
        id: 'task-1',
        projectName: 'p',
        title: 'First',
        description: 'Do the first thing.'
      })
      out.created = (await settled('created'))[0]
      out.move = await answer('task:setStatus', { id: 'task-1', status: 'in_progress' })
      out.moved = (await settled('moved'))[0]
      out.byTask = await answer('workflowRun:listByTask', { taskId: 'task-1' })

      // A terminal step on the queue's first task: the task is taken for its session.
      await answer('task:create', { id: 'task-2', projectName: 'p', title: 'Queued' })
      await client.result('workflow:create', {
        workflow: workflow(
          'taker',
          { triggerType: 'manual' },
          [
            step('agent', 'launchAgent', {
              agentType: 'codex',
              projectName: 'p',
              taskFromQueue: true,
              prompt: ''
            })
          ],
          [['t', 'agent']]
        )
      })
      await answer('workflow:run', { workflowId: 'taker' })
      out.taker = agentSteps((await settled('taker'))[0])
      out.taken = await answer('task:get', { id: 'task-2' })
      // A headless step that cannot start fails without moving its task.
      await client.result('workflow:create', {
        workflow: workflow(
          'headless',
          { triggerType: 'manual' },
          [
            step('agent', 'launchAgent', {
              agentType: 'codex',
              projectName: 'p',
              taskId: 'task-1',
              headless: true,
              prompt: 'Do it'
            })
          ],
          [['t', 'agent']]
        )
      })
      await answer('workflow:run', { workflowId: 'headless' })
      out.headless = agentSteps((await settled('headless'))[0])
      out.task1 = await answer('task:get', { id: 'task-1' })
      out.reopened = await settled('reopened')
      out.movedAll = await settled('moved', 2)
      out.createdAll = await settled('created', 2)
      for (const id of ['created', 'moved', 'reopened', 'taker', 'headless']) {
        await client.result('workflow:setEnabled', { id, enabled: false })
      }
      for (const s of (await client.result('terminal:listActive')) as Array<{ id: string }>) {
        await answer('terminal:kill', s.id)
      }
      const { got, want } = replayed('tasks', out)
      expect(got).toEqual(want)
    })

    it('polls a connector into a workflow whose steps call it and make tasks from its items', async () => {
      const repo = path.join(work, 'tickets')
      connectorRepo(repo)
      const out: Json = {}
      out.install = await answer('connector:installPack', { kind: 'git', url: repo })
      const conn = (await client.result('connection:create', {
        connectorId: 'sdk',
        name: 'Tickets',
        syncIntervalMinutes: 0,
        statusMapping: {},
        filters: { sdkConnectorId: 'tickets', sdkVersion: '1.0.0', sdkTrigger: 'items' }
      })) as { id: string }
      await client.result('workflow:create', {
        workflow: workflow(
          'poll',
          {
            triggerType: 'connectorPoll',
            connectionId: conn.id,
            event: 'mcpPoll',
            cron: '0 0 1 1 *'
          },
          [
            step('make', 'createTaskFromItem', { project: 'p', initialStatus: 'todo' }),
            step('act', 'callConnectorAction', {
              connectionId: conn.id,
              action: 'create',
              args: { title: '{{connectorItem.title}}', n: 5 }
            }),
            step(
              'nope',
              'callConnectorAction',
              {
                connectionId: conn.id,
                action: 'create',
                args: { fail: true }
              },
              { onError: 'continue' }
            ),
            script('item', 'echo "{{connectorItem.title}} {{steps.act.output}}"')
          ],
          [
            ['t', 'make'],
            ['make', 'act'],
            ['act', 'nope'],
            ['nope', 'item']
          ]
        )
      })
      out.poll = await answer('connector:poll', { workflowId: 'poll' })
      out.again = await answer('connector:poll', { workflowId: 'poll' })
      await settled('poll')
      await new Promise((r) => setTimeout(r, 1_500))
      out.runs = await runs('poll')
      out.manual = await answer('workflow:run', { workflowId: 'poll' })
      await new Promise((r) => setTimeout(r, 1_500))
      out.tasks = await answer('task:list', { projectName: 'p', includeDescription: true })
      await client.result('workflow:setEnabled', { id: 'poll', enabled: false })
      const { got, want } = replayed('connector', out)
      expect(got).toEqual(want)
    })

    it('answers every MCP workflow tool as it did, export and import among them', async () => {
      const send = await mcp(served.port)
      const listed = (await send('tools/list')) as { tools: Array<{ name: string }> }
      const tools = listed.tools.filter((t) => WORKFLOW_TOOLS.includes(t.name))
      const call = async (name: string, args: Json): Promise<unknown> =>
        send('tools/call', { name, arguments: args })
      const out: unknown[] = []
      const record = async (name: string, args: Json): Promise<unknown> => {
        const result = await call(name, args)
        out.push({ name, args, result })
        return result
      }
      await record('describe_workflow_nodes', {})
      await record('describe_workflow_nodes', { types: ['script', 'approval', 'nope'] })
      const made = (await record('create_workflow', {
        name: 'From an agent',
        nodes: [
          {
            id: 'go',
            type: 'trigger',
            label: 'Go',
            config: { triggerType: 'manual' },
            position: pos
          },
          {
            id: 'one',
            type: 'script',
            label: 'One',
            slug: 'one',
            config: { scriptType: 'bash', scriptContent: 'echo one' },
            position: pos
          },
          {
            id: 'check',
            type: 'approval',
            label: 'Check',
            slug: 'check',
            config: { message: 'OK {{steps.one.output}}?' },
            position: pos
          }
        ],
        edges: [
          { id: 'e1', source: 'go', target: 'one' },
          { id: 'e2', source: 'one', target: 'check' }
        ]
      })) as { content?: Array<{ text?: string }> }
      await record('create_workflow', {
        name: 'Flat',
        trigger: { triggerType: 'manual' },
        actions: [{ agentType: 'codex', prompt: 'Say hi', projectName: 'p' }]
      })
      const id = /"id": "([^"]+)"/.exec(made?.content?.[0]?.text ?? '')?.[1] ?? 'missing'
      await record('create_workflow', { name: '' })
      await record('list_workflows', {})
      await record('update_workflow', { workflow_id: id, name: 'Renamed by an agent' })
      await record('update_workflow', { workflow_id: 'ghost', name: 'x' })
      await record('get_workflow_schedule', { workflow_id: 'once' })
      await record('get_workflow_schedule', { workflow_id: id })
      await record('execute_workflow', { workflow_id: id })
      await until('the agent workflow to wait', async () =>
        (await runs(id))[0]?.nodeStates.some((n) => n.status === 'waiting')
      )
      await record('list_workflow_runs', { workflow_id: id })
      await record('list_workflow_runs', { status: 'waiting' })
      const [run] = await runs(id)
      await record('resolve_gate', { run_id: run.runId, decision: 'approve', comment: 'ok' })
      await settled(id)
      await record('list_workflow_runs', { workflow_id: id, limit: 1 })
      await record('stop_workflow_run', { run_id: run.runId })
      const exported = (await record('export_workflow', {
        workflow_id: 'by-hand'
      })) as { content?: Array<{ text?: string }> }
      const file = exported?.content?.[0]?.text ?? '{}'
      await record('import_workflow', { workflow: file, project_name: 'p' })
      await record('import_workflow', { workflow: file, project_name: 'p', bundle: 'again' })
      await record('import_workflow', { workflow: '{"name":', project_name: 'p' })
      await record('import_workflow', { workflow: file, project_name: 'nowhere' })
      await record('export_workflow', { workflow_id: 'poll' })
      await record('export_workflow', { workflow_id: 'ghost' })
      await record('delete_workflow', { workflow_id: id })
      await record('delete_workflow', { workflow_id: id })
      await record('list_artifacts', {})
      const { got, want } = replayed('mcp', {
        names: listed.tools.map((t) => t.name),
        tools,
        calls: out
      })
      expect(got).toEqual(want)
    })

    it('leaves the workflows and their runs as they were', async () => {
      await until('quiet', () => true)
      const list = (await client.result('workflow:list')) as Json[]
      const { got, want } = replayed('final', {
        workflows: list.map((w) => ({ ...w, lastRunAt: w.lastRunAt ? '<time>' : null })),
        log: await answer('scheduler:getLog')
      })
      expect(got).toEqual(want)
    })
  }
)
