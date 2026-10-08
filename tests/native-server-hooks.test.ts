/**
 * Agents' hooks, received by vornd through a real server and the vornd it
 * keeps: the endpoint registered in the run's home, refusals of what is not a
 * hook event, a terminal's status and conversation taken from its hooks, a
 * permission request held until a client answers it or the agent moves on,
 * the conversation kept on the terminal's task, Copilot's hooks file and its
 * script, and the registration given up when vornd stops.
 *
 * Runs where vornd and its session holder have been built, on a Unix.
 */
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import type { AppConfig, TaskConfig, TerminalSession } from '../packages/shared/src/types'
import { queryDb } from './helpers/database'
import { hookEndpoint, postHook } from './helpers/hooks'
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

describe.runIf(runnable)('agents’ hooks in vornd', () => {
  let server: RealServer
  let client: Watcher
  let endpoint: { port: number; token: string }
  let claude: TerminalSession
  let home = ''

  const record = async (id: string): Promise<TerminalSession | undefined> =>
    (await client.result<TerminalSession[]>('terminal:listActive')).find((s) => s.id === id)
  const event = (name: string, session: string, extra: Record<string, unknown> = {}) => ({
    hook_event_name: name,
    session_id: session,
    cwd: server.dirs.work,
    ...extra
  })

  beforeAll(async () => {
    server = await startRealServer()
    home = server.dirs.home
    client = await Watcher.open(server.vornd)
    await until('vornd to register its hook endpoint', () => !!hookEndpoint(home))
    endpoint = hookEndpoint(home)!
    const stub = path.join(server.dirs.work, 'stub-agent')
    fs.writeFileSync(stub, '#!/bin/sh\nexec sleep 600\n', { mode: 0o755 })
    const config = await client.result<AppConfig>('config:load')
    await client.result('config:save', {
      ...config,
      defaults: { ...config.defaults, shell: '/bin/sh' },
      agentCommands: { claude: { command: stub, args: [] }, copilot: { command: stub, args: [] } },
      projects: [{ name: 'p', path: server.dirs.work, preferredAgents: ['claude'] }]
    })
    claude = await client.result<TerminalSession>('terminal:create', {
      agentType: 'claude',
      projectName: 'p',
      projectPath: server.dirs.work
    })
    await until('the agent to start', async () => ((await record(claude.id))?.pid ?? 0) > 0)
  })

  afterAll(async () => {
    client?.close()
    if (server) await stopRealServer(server)
    removeRealServerDirs()
  })

  it('registers its endpoint in the home it runs in', () => {
    const vorn = path.join(home, '.vorn')
    expect(fs.statSync(path.join(vorn, 'token')).mode & 0o777).toBe(0o600)
    const owner = JSON.parse(fs.readFileSync(path.join(vorn, 'hook-owner'), 'utf-8'))
    expect(owner.port).toBe(endpoint.port)
    const settings = JSON.parse(
      fs.readFileSync(path.join(home, '.claude', 'settings.json'), 'utf-8')
    )
    expect(Object.keys(settings.hooks)).toHaveLength(8)
    expect(settings.hooks.Stop.at(-1).hooks[0]).toMatchObject({
      type: 'http',
      url: `http://localhost:${endpoint.port}/hooks`,
      headers: { Authorization: `Bearer ${endpoint.token}`, 'x-vorn-terminal': '$VORN_SESSION_ID' }
    })
  })

  it('refuses what is not an agent’s hook event, and goes on', async () => {
    expect((await postHook(endpoint, {}, { method: 'GET' })).status).toBe(404)
    expect((await postHook(endpoint, event('Stop', 's'), { token: 'wrong' })).status).toBe(401)
    const malformed = await postHook(endpoint, { hook_event_name: 'Notification' })
    expect(malformed.status).toBe(400)
    expect(await malformed.json()).toEqual({ error: 'Malformed hook event' })
    expect((await postHook(endpoint, event('Notification', 's', { cwd: '' }))).status).toBe(200)
  })

  it('takes a terminal’s status and conversation from its hooks', async () => {
    const res = await postHook(endpoint, event('SessionStart', 'conv-a'), { terminal: claude.id })
    expect(await res.json()).toEqual({})
    await until('the terminal linked and on hooks', async () => {
      const r = await record(claude.id)
      return r?.hookSessionId === 'conv-a' && r.statusSource === 'hooks' && r.status === 'running'
    })
    await postHook(endpoint, event('Stop', 'conv-a'))
    await until('the terminal idle', async () => (await record(claude.id))?.status === 'idle')
  })

  it('holds a permission request until a client answers it', async () => {
    const asked = postHook(
      endpoint,
      event('PermissionRequest', 'conv-a', {
        tool_name: 'Bash',
        tool_input: { command: 'rm -rf build' }
      }),
      { terminal: claude.id }
    )
    let request: { requestId: string; description?: string; terminalId?: string } | undefined
    await until('clients to be asked', () => {
      request = client.toldBy('widget:permission-request').at(-1) as typeof request
      return !!request
    })
    expect(request).toMatchObject({ description: 'rm -rf build', terminalId: claude.id })
    await until('the terminal waiting', async () => (await record(claude.id))?.status === 'waiting')
    await client.result('permission:resolve', {
      requestId: request!.requestId,
      allow: true,
      updatedInput: { command: 'rm -rf build/tmp' }
    })
    expect(await (await asked).json()).toEqual({
      hookSpecificOutput: {
        hookEventName: 'PermissionRequest',
        decision: { behavior: 'allow', updatedInput: { command: 'rm -rf build/tmp' } }
      }
    })
  })

  it('lets a request go when the agent moves past it', async () => {
    const before = client.toldBy('widget:permission-request').length
    const asked = postHook(endpoint, event('PermissionRequest', 'conv-a', { tool_name: 'Edit' }), {
      terminal: claude.id
    })
    await until(
      'clients to be asked',
      () => client.toldBy('widget:permission-request').length > before
    )
    const { requestId } = client.toldBy('widget:permission-request').at(-1) as { requestId: string }
    await postHook(endpoint, event('PostToolUse', 'conv-a'))
    expect(await (await asked).json()).toEqual({})
    await until('clients to be told it is gone', () =>
      client.toldBy('widget:permission-cancelled').includes(requestId)
    )
    // A conversation no terminal has is the agent's own to decide.
    const loose = await postHook(
      endpoint,
      event('PermissionRequest', 'nobody', { cwd: '/nowhere' })
    )
    expect(await loose.json()).toEqual({})
  })

  it('keeps the conversation a terminal started on the task it was given', async () => {
    const created = await client.result<{ task: TaskConfig }>('task:create', {
      projectName: 'p',
      title: 'With an agent',
      status: 'in_progress'
    })
    const db = path.join(server.dirs.data, 'vorn.db')
    queryDb(db, (d) =>
      d
        .prepare('UPDATE tasks SET assigned_session_id = ? WHERE id = ?')
        .run(claude.id, created.task.id)
    )
    await postHook(endpoint, event('SessionStart', 'conv-b'), { terminal: claude.id })
    await until('the task to carry the conversation', async () => {
      const task = await client.result<TaskConfig>('task:get', { id: created.task.id })
      return task.agentSessionId === 'conv-b'
    })
  })

  it('writes Copilot’s hooks file, whose script posts only inside Vorn', async () => {
    const copilot = await client.result<TerminalSession>('terminal:create', {
      agentType: 'copilot',
      projectName: 'p',
      projectPath: server.dirs.work
    })
    await until('the copilot terminal linked', async () => {
      return (await record(copilot.id))?.hookSessionId === `copilot-${copilot.id}`
    })
    const file = path.join(home, '.copilot', 'hooks', 'vorn.json')
    const hooks = JSON.parse(fs.readFileSync(file, 'utf-8'))
    expect(hooks._vorn).toBe(true)
    const run = (env: Record<string, string>, input: string | Buffer): void => {
      execFileSync('/bin/sh', ['-c', hooks.hooks.sessionStart[0].bash], {
        input,
        env: { PATH: `${path.dirname(process.execPath)}:/usr/bin:/bin`, HOME: home, ...env },
        timeout: 10_000
      })
    }
    // Outside Vorn: the whole event is read and nothing is posted.
    run({}, Buffer.alloc(256 * 1024, ' '))
    run({ VORN_SESSION_ID: copilot.id }, JSON.stringify({ cwd: server.dirs.work }))
    await until(
      'copilot on hooks',
      async () => (await record(copilot.id))?.statusSource === 'hooks'
    )
  })

  it('gives the registration up when it stops', async () => {
    const health = (await (
      await fetch(`http://127.0.0.1:${server.vornd}/vornd/health`)
    ).json()) as {
      unexpectedForwards: Record<string, number>
      hooks: { owner: boolean; pendingPermissions: number }
    }
    expect(health.hooks).toMatchObject({ owner: true, pendingPermissions: 0 })
    expect(health.unexpectedForwards).toEqual({})
    await stopRealServer(server)
    await until('the registration to go', () => !fs.existsSync(path.join(home, '.vorn', 'port')))
    expect(fs.existsSync(path.join(home, '.vorn', 'token'))).toBe(false)
    expect(fs.existsSync(path.join(home, '.copilot', 'hooks', 'vorn.json'))).toBe(false)
    const settings = JSON.parse(
      fs.readFileSync(path.join(home, '.claude', 'settings.json'), 'utf-8')
    )
    expect(settings.hooks).toEqual({})
    server = undefined as unknown as RealServer
  })
})
