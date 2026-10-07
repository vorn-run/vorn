import { describe, it, expect, beforeAll, afterAll, afterEach } from 'vitest'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import { runCopilotHook } from './helpers/copilot-hook'
import { normalizePath } from '../packages/server/src/process-utils'

/**
 * The event that took the server down, and why answering it was the fatal part.
 *
 * A `Notification` arrived with `session_id` and `cwd` undefined. `HookEvent`
 * declares both as required and `hook-server` cast the parsed body straight to
 * it, so the status mapper read them, handed `cwd` to `normalizePath`, and threw.
 *
 * That throw alone should have been survivable — the call sat inside a
 * try/catch. The catch answered with `res.writeHead(400)`, on a response
 * `handleEvent` had already ended before emitting. Writing headers to a finished
 * response throws `ERR_HTTP_HEADERS_SENT`, from inside the handler, with nothing
 * above it. The error handler is what ended the process, three milliseconds
 * after the event arrived.
 *
 * So there are three separate things to hold: the payload never reaches the code
 * that throws, a listener that throws anyway cannot escape, and the catch cannot
 * make things worse than what it is catching.
 */

/**
 * A home directory of its own, because a real `HookServer` writes to one.
 *
 * `start()` claims `~/.vorn/{hook-owner,port,token}`, and those paths come from
 * `os.homedir()` — no data directory involved. Left alone, this file would
 * repoint the machine's hook endpoint at a server that exists for one assertion
 * and then exits. It happens to be refused today, because the ownership check
 * sees the running Vorn and declines, but that is the machine's state saving the
 * test rather than the test being safe.
 *
 * Set before the first import of `hook-server`, since it reads `homedir()` once
 * at module scope — which is why the import below is dynamic.
 */
let home: string | null = null
let realHome: string | undefined

let realProfile: string | undefined
let realCopilotHome: string | undefined

beforeAll(() => {
  realHome = process.env.HOME
  realProfile = process.env.USERPROFILE
  realCopilotHome = process.env.COPILOT_HOME
  // Copilot's hooks would otherwise land in a developer's own COPILOT_HOME.
  delete process.env.COPILOT_HOME
  home = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-hooks-'))
  // Both, because `os.homedir()` reads HOME on POSIX and USERPROFILE on Windows.
  // Setting only HOME leaves a developer on Windows running this suite against
  // their real home directory — the isolation would look present and not be.
  process.env.HOME = home
  process.env.USERPROFILE = home
})

/**
 * Put a variable back, including putting it back to not existing.
 *
 * `process.env.X = undefined` does not unset X — it sets it to the *string*
 * "undefined", and every later reader gets a home directory by that name. On a
 * machine where one of these was never set to begin with, restoring it by
 * assignment is how a test leaves the process dirtier than it found it.
 */
function restore(name: 'HOME' | 'USERPROFILE' | 'COPILOT_HOME', value: string | undefined): void {
  if (value === undefined) delete process.env[name]
  else process.env[name] = value
}

afterAll(() => {
  restore('HOME', realHome)
  restore('USERPROFILE', realProfile)
  restore('COPILOT_HOME', realCopilotHome)
  if (home) fs.rmSync(home, { recursive: true, force: true })
})

let server: { stop: () => void; port: number; token: string } | null = null

afterEach(() => {
  server?.stop()
  server = null
})

async function startHookServer() {
  const { HookServer } = await import('../packages/server/src/hook-server')
  const instance = new HookServer()
  // Any free port, never the fixed one. `server-integration.test.ts` starts a
  // server that claims the fixed port too, and vitest runs files in parallel --
  // so on a machine where nothing else holds it the two collide and requests
  // come back as `socket hang up`. Locally that never happened, because the
  // running Vorn already had it and both fell back. CI had no such accident.
  const port = await instance.start(0)
  return {
    instance,
    port,
    token: instance.getAuthToken(),
    stop: () => instance.stop()
  }
}

function post(
  port: number,
  token: string,
  body: string,
  extraHeaders: Record<string, string> = {}
): Promise<number> {
  return new Promise((resolve, reject) => {
    const req = http.request(
      {
        hostname: '127.0.0.1',
        port,
        path: '/hooks',
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Authorization: `Bearer ${token}`,
          ...extraHeaders
        }
      },
      (res) => {
        res.resume()
        resolve(res.statusCode ?? 0)
      }
    )
    req.on('error', reject)
    req.end(body)
  })
}

async function until(check: () => boolean): Promise<void> {
  for (let i = 0; i < 200 && !check(); i++) await new Promise((r) => setTimeout(r, 10))
}

describe('the payload that crashed the server', () => {
  it('is refused, and the server is still there afterwards', async () => {
    const started = await startHookServer()
    server = started

    // Exactly what the log recorded: a name, and nothing else.
    const status = await post(started.port, started.token, '{"hook_event_name":"Notification"}')
    expect(status).toBe(400)

    // The assertion that matters. Before the fix the process was gone by now.
    const after = await post(
      started.port,
      started.token,
      JSON.stringify({ hook_event_name: 'Stop', session_id: 's1', cwd: '/tmp' })
    )
    expect(after).toBe(200)
  })

  it.each([
    ['no session', '{"hook_event_name":"Notification","cwd":"/tmp"}'],
    ['no cwd', '{"hook_event_name":"Notification","session_id":"s1"}'],
    ['empty session', '{"hook_event_name":"Stop","session_id":"","cwd":"/tmp"}'],
    ['a cwd that is not a string', '{"hook_event_name":"Stop","session_id":"s1","cwd":42}'],
    ['no name', '{"session_id":"s1","cwd":"/tmp"}'],
    ['not an object', '"just a string"'],
    ['null', 'null']
  ])('refuses a payload with %s', async (_label, body) => {
    const started = await startHookServer()
    server = started

    expect(await post(started.port, started.token, body)).toBe(400)
  })

  it('accepts an empty cwd, which is what Copilot sends', async () => {
    // `copilot-hook-installer` bakes `cwd: d.cwd||''` into the scripts it writes
    // into another tool's configuration, and those sessions are force-linked, so
    // they resolve from the session map without ever needing a directory. An
    // empty string has always worked; only `undefined` reached the throw. A
    // guard strict enough to reject `''` would 400 every Copilot hook event.
    const started = await startHookServer()
    server = started

    const seen: unknown[] = []
    started.instance.on('hook-event', (event) => seen.push(event))

    const status = await post(
      started.port,
      started.token,
      JSON.stringify({ hook_event_name: 'PostToolUse', session_id: 'copilot-1', cwd: '' })
    )

    expect(status).toBe(200)
    expect(seen).toHaveLength(1)
  })

  it('still accepts a well-formed event', async () => {
    const started = await startHookServer()
    server = started

    const seen: unknown[] = []
    started.instance.on('hook-event', (event) => seen.push(event))

    const status = await post(
      started.port,
      started.token,
      JSON.stringify({ hook_event_name: 'PreToolUse', session_id: 's1', cwd: '/tmp' })
    )

    expect(status).toBe(200)
    expect(seen).toHaveLength(1)
  })

  it('survives a listener that throws, without reaching the top of the process', async () => {
    // `emit` is synchronous, so a listener runs on the request handler's stack,
    // and the response has already been sent by the time it runs. That is what
    // made the throw fatal: the catch above answered it with `writeHead(400)` on
    // a finished response, which threw again with nothing left to catch it.
    //
    // The status codes alone cannot show this. Both requests are answered before
    // anything fails, so they return 200 either way — checked by mutation, and
    // an earlier version of this test passed against the unfixed code for
    // exactly that reason. What has to be watched is whether anything reaches
    // `uncaughtException`, because in the real server that is the process ending.
    const started = await startHookServer()
    server = started
    started.instance.on('hook-event', () => {
      throw new Error('a listener blew up')
    })

    const escaped: Error[] = []
    const watch = (err: Error) => escaped.push(err)
    process.on('uncaughtException', watch)

    try {
      expect(
        await post(
          started.port,
          started.token,
          JSON.stringify({ hook_event_name: 'Stop', session_id: 's1', cwd: '/tmp' })
        )
      ).toBe(200)

      // A tick for anything thrown after the response was flushed.
      await new Promise((resolve) => setTimeout(resolve, 50))

      expect(escaped.map((err) => (err as NodeJS.ErrnoException).code ?? err.message)).toEqual([])
    } finally {
      process.off('uncaughtException', watch)
    }

    // And it is still answering.
    expect(
      await post(
        started.port,
        started.token,
        JSON.stringify({ hook_event_name: 'Stop', session_id: 's2', cwd: '/tmp' })
      )
    ).toBe(200)
  })
})

describe('the terminal a hook names', () => {
  async function received(
    body: Record<string, unknown>,
    headers: Record<string, string> = {}
  ): Promise<Record<string, unknown>> {
    const started = await startHookServer()
    server = started
    const seen: Record<string, unknown>[] = []
    started.instance.on('hook-event', (event) => seen.push(event))
    const base = { hook_event_name: 'PreToolUse', session_id: 's1', cwd: '/tmp' }
    expect(
      await post(started.port, started.token, JSON.stringify({ ...base, ...body }), headers)
    ).toBe(200)
    expect(seen).toHaveLength(1)
    return seen[0]
  }

  it('takes it from the header Claude fills from the environment', async () => {
    expect(await received({}, { 'X-Vorn-Terminal': 'term-1' })).toMatchObject({
      vorn_terminal_id: 'term-1'
    })
  })

  it('takes it from the body Copilot posts, the header winning over it', async () => {
    expect(await received({ vorn_terminal_id: 'term-2' })).toMatchObject({
      vorn_terminal_id: 'term-2'
    })
    expect(
      await received({ vorn_terminal_id: 'term-2' }, { 'X-Vorn-Terminal': 'term-1' })
    ).toMatchObject({ vorn_terminal_id: 'term-1' })
  })

  it.each([
    ['an empty header, as from an agent outside Vorn', {}, { 'X-Vorn-Terminal': '' }],
    ['an empty body field', { vorn_terminal_id: '' }, {}],
    ['a body field that is not a string', { vorn_terminal_id: 42 }, {}]
  ])('names none for %s', async (_label, body, headers) => {
    expect(await received(body, headers)).not.toHaveProperty('vorn_terminal_id')
  })

  it('is asked of Claude in the hooks Vorn installs', async () => {
    const { installHooks } = await import('../packages/server/src/hook-installer')
    installHooks(4242, 'tok')
    const settings = JSON.parse(
      fs.readFileSync(path.join(home as string, '.claude', 'settings.json'), 'utf-8')
    )
    const hook = settings.hooks.PreToolUse.at(-1).hooks[0]
    expect(hook.headers['x-vorn-terminal']).toBe('$VORN_SESSION_ID')
    expect(hook.allowedEnvVars).toEqual(['VORN_SESSION_ID'])
  })

  it.skipIf(process.platform === 'win32')(
    'is posted by the script Copilot runs, from its environment',
    async () => {
      const started = await startHookServer()
      server = started
      const seen: Record<string, unknown>[] = []
      started.instance.on('hook-event', (event) => seen.push(event))

      const { installCopilotHooks, uninstallAllCopilotHooks } =
        await import('../packages/server/src/copilot-hook-installer')
      const project = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-copilot-hooks-'))
      const installation = installCopilotHooks('term-copilot')
      try {
        const hooks = JSON.parse(fs.readFileSync(installation.hooksJsonPath, 'utf-8'))
        // Not the sync form: the script posts to this process, which must stay free to answer.
        await runCopilotHook(
          hooks.hooks.preToolUse[0].bash,
          { VORN_SESSION_ID: 'term-copilot' },
          JSON.stringify({ cwd: project, toolName: 'bash' })
        )
        await until(() => seen.length === 1)
        expect(seen[0]).toMatchObject({
          session_id: installation.sessionId,
          vorn_terminal_id: 'term-copilot'
        })
      } finally {
        uninstallAllCopilotHooks()
        fs.rmSync(project, { recursive: true, force: true })
      }
    }
  )
})

describe('this test file itself', () => {
  it('names a malformed event even when the name itself is empty', async () => {
    // `??` treats '' as present, so the log line began with a space and said
    // nothing about what had arrived — the one job of that message.
    const started = await startHookServer()
    server = started

    expect(await post(started.port, started.token, '{"hook_event_name":"","cwd":"/tmp"}')).toBe(400)
  })

  it('writes its claim inside its own home directory', async () => {
    // Behavioural on purpose. Asserting the environment variables only proves
    // they are set now; `hook-server` resolves `~/.vorn` into module-level
    // constants at import time, so what matters is where the constants landed —
    // and if some future import graph or an `isolate: false` in the vitest config
    // loaded that module before `beforeAll` ran, they would point at the real
    // home while every env check still passed.
    //
    // So: start a server, let it claim, and look for the evidence here rather
    // than there. This failing is the sandbox failing, which is the only warning
    // anyone gets before a test repoints a real machine's hook registration.
    const started = await startHookServer()
    server = started

    expect(os.homedir()).toBe(home)
    expect(fs.existsSync(path.join(home as string, '.vorn', 'hook-owner'))).toBe(true)
    expect(fs.existsSync(path.join(home as string, '.vorn', 'port'))).toBe(true)
  })
})

describe('the utility underneath it', () => {
  it('still refuses a path that is not one', () => {
    // Pinned deliberately. The guard belongs at the boundary, where an untrusted
    // body is turned into a typed event — not in a general path helper, which
    // would then quietly absorb the next caller's mistake instead of this one.
    expect(() => normalizePath(undefined as unknown as string)).toThrow(TypeError)
  })
})

describe('what the idle clock counts as an agent being alive', () => {
  // `msSinceHookActivity` is the only trace an agent started outside Vorn leaves
  // on this server, and the idle watch stays up for it. That makes advancing it
  // a way to keep the server alive, so it must take the same proof every other
  // hook request takes.
  it('advances on an authenticated hook post', async () => {
    const started = await startHookServer()
    server = started
    await new Promise((r) => setTimeout(r, 30))
    const before = started.instance.msSinceHookActivity()
    expect(before).toBeGreaterThan(0)

    await post(
      started.port,
      started.token,
      '{"hook_event_name":"Stop","session_id":"s","cwd":"/tmp"}'
    )
    expect(started.instance.msSinceHookActivity()).toBeLessThan(before)
  })

  it('does not advance for a request with no credential', async () => {
    const started = await startHookServer()
    server = started
    await new Promise((r) => setTimeout(r, 30))
    const before = started.instance.msSinceHookActivity()

    const status = await post(started.port, 'not-the-token', '{"hook_event_name":"Stop"}')
    expect(status).toBe(401)
    // Anything on this machine can reach loopback. If a refused request counted,
    // a port scan would be enough to keep a server nobody is using alive for
    // ever -- a worse hole than the one this clock closes.
    expect(started.instance.msSinceHookActivity()).toBeGreaterThanOrEqual(before)
  })

  it('does not advance for a request that is not a POST', async () => {
    const started = await startHookServer()
    server = started
    await new Promise((r) => setTimeout(r, 30))
    const before = started.instance.msSinceHookActivity()

    await new Promise<void>((resolve, reject) => {
      const req = http.request(
        { hostname: '127.0.0.1', port: started.port, path: '/hooks', method: 'GET' },
        (res) => {
          res.resume()
          expect(res.statusCode).toBe(404)
          resolve()
        }
      )
      req.on('error', reject)
      req.end()
    })
    expect(started.instance.msSinceHookActivity()).toBeGreaterThanOrEqual(before)
  })
})
