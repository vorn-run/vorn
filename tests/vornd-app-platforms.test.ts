import { describe, it, expect, beforeAll, afterAll } from 'vitest'
import fs from 'node:fs'
import path from 'node:path'
import { IPC, type AppConfig } from '../packages/shared/src/types'
import WebSocket from 'ws'
import { BytesClient, until, home as testHome, killPid } from './helpers/vornd-sessions'
import { builtSessiond, builtVornd, startServed, type Served } from './helpers/served'

/**
 * Sessions through vornd as the server on every platform the app ships on,
 * Windows included, where its sessions run in ConPTY: a client creates a
 * shell and a headless agent as the app does, and is told what they print
 * and how they end. The agent is a Node script, so nothing here depends on a
 * POSIX shell.
 */

/** The desktop's credential, which vornd as the server takes as its own. */
const DESKTOP = 'platforms-desktop-token'

interface Told {
  method: string
  params: Record<string, unknown>
}

/** A client that keeps every notification it is sent. */
async function listen(port: number, told: Told[]): Promise<WebSocket> {
  const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`, {
    headers: { authorization: `Bearer ${DESKTOP}` }
  })
  ws.on('message', (raw, binary) => {
    if (binary) return
    const frame = JSON.parse(String(raw)) as Partial<Told>
    if (frame.method && frame.params) told.push(frame as Told)
  })
  await new Promise<void>((resolve, reject) => {
    ws.once('open', () => resolve())
    ws.once('error', reject)
  })
  return ws
}

describe.skipIf(!builtVornd || !builtSessiond)('sessions through vornd on this platform', () => {
  let h: ReturnType<typeof testHome>
  let vornd: Served
  let listener: WebSocket
  let client: BytesClient
  const told: Told[] = []

  /** A Node script under the test's home. */
  const script = (name: string, body: string): string => {
    const file = path.join(h.dir, `${name}.cjs`)
    fs.writeFileSync(file, body)
    return file
  }

  /** The notifications told about session `id` on `method`. */
  const about = (method: string, id: string): Told[] =>
    told.filter((t) => t.method === method && t.params.id === id)

  const serve = (): Promise<Served> =>
    startServed({
      dataDir: h.dir,
      home: h.dir,
      credential: DESKTOP,
      sessiond: true,
      args: ['--debug-spawn']
    })

  beforeAll(async () => {
    h = testHome()
    const agent = script(
      'fake-agent',
      "process.stdout.write('prompt: ')\n" +
        'process.stdin.pipe(process.stdout)\n' +
        "process.stdin.on('end', () => process.exit(5))\n"
    )
    vornd = await serve()
    listener = await listen(vornd.port, told)
    client = new BytesClient()
    await client.connect(vornd.port, DESKTOP)
    const config = await client.call<AppConfig>('config:load', undefined)
    await client.call('config:save', {
      ...config,
      agentCommands: {
        ...config.agentCommands,
        claude: { command: process.execPath, args: [agent] }
      }
    })
  }, 30_000)

  afterAll(async () => {
    client?.close()
    listener?.close()
    const health = vornd
      ? await fetch(`http://127.0.0.1:${vornd.port}/vornd/health`)
          .then((r) => r.json() as Promise<{ sessiond?: { current?: { pid?: number } } }>)
          .catch(() => null)
      : null
    await vornd?.stop()
    killPid(health?.sessiond?.current?.pid ?? null)
    h?.remove()
  })

  it('runs a shell a client types into, and tells it how the shell ended', async () => {
    const shell = await client.call<{ id: string; agentType: string }>('shell:create', h.dir)
    expect(shell.agentType).toBe('shell')
    await until('the new terminal told', () => about(IPC.SESSION_CREATED, shell.id).length > 0)
    // Attached once its program runs, as a client told to attach again does.
    await until('its program up', () =>
      about(IPC.SESSION_UPDATED, shell.id).some((t) => Number(t.params.pid) > 0)
    )
    await client.attach(shell.id)
    client.notify('terminal:write', { id: shell.id, data: 'echo vorn-shell-ok\r' })
    await until('the echo', async () => (await client.text()).includes('vorn-shell-ok'))
    client.notify('terminal:write', { id: shell.id, data: 'exit 4\r' })
    await until('the exit', () => client.exits.length === 1)
    // Told once, however many paths it could have come by.
    await new Promise((r) => setTimeout(r, 300))
    expect(client.exits).toEqual([4])
  }, 30_000)

  it('runs a headless agent on pipes: its prompt in, its output out, its exit told once', async () => {
    const agent = await client.call<{ id: string }>('headless:create', {
      agentType: 'claude',
      projectName: 'p',
      projectPath: h.dir,
      initialPrompt: 'write the tests'
    })
    await until('the exit', () => about(IPC.HEADLESS_EXIT, agent.id).length > 0)
    const data = about(IPC.HEADLESS_DATA, agent.id)
      .map((t) => t.params.data)
      .join('')
    expect(data).toContain('prompt: write the tests')
    await new Promise((r) => setTimeout(r, 200))
    expect(about(IPC.HEADLESS_EXIT, agent.id).map((t) => t.params)).toEqual([
      { id: agent.id, exitCode: 5 }
    ])
  }, 30_000)

  it('keeps a session when vornd is killed, and carries on with the next vornd', async () => {
    const echo = script(
      'echo-lines',
      "console.log('ready-' + (40 + 2))\n" +
        "require('readline').createInterface({ input: process.stdin })" +
        ".on('line', (l) => console.log('echo:' + l.trim()))\n"
    )
    const id = await client.spawn([process.execPath, echo])
    await client.attach(id)
    await until('the first line', async () => (await client.text()).includes('ready-42'))
    client.close()

    listener.close()
    vornd.child.kill('SIGKILL')
    await new Promise((r) => vornd.child.once('exit', r))
    vornd = await serve()
    listener = await listen(vornd.port, told)
    client = new BytesClient()
    await client.connect(vornd.port, DESKTOP)
    await client.attach(id)
    expect(await client.text()).toContain('ready-42')
    client.notify('terminal:write', { id, data: 'after-restart\r' })
    await until('the echo', async () => (await client.text()).includes('echo:after-restart'))
  }, 30_000)
})
