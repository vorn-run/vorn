import { describe, it, expect, vi, afterEach } from 'vitest'
import { spawn, type ChildProcess } from 'node:child_process'
import { existsSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { findVornd, startVornd, upstreamPort, type Vornd } from '../src/main/server/vornd'

vi.mock('../src/main/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

/**
 * Starting vornd from the app, against a stand-in that behaves the ways vornd
 * can: says where it listens, says nothing, says something else, or dies. The
 * stand-in is a Node script so the test does not need the Rust build; the real
 * binary gets its own test below when it is built.
 */
const STUB = `
const mode = process.env.STUB_MODE
const upstream = process.argv[process.argv.indexOf('--upstream') + 1]
if (!process.argv.includes('--exit-with-stdin')) process.exit(9)
if (mode === 'ok') {
  process.stdout.write(JSON.stringify({ port: 47001, protocol: 1, upstream }) + '\\n')
  process.stdin.resume()
  process.stdin.on('end', () => process.exit(0))
} else if (mode === 'garbage') {
  process.stdout.write('listening!\\n')
  setInterval(() => {}, 1000)
} else if (mode === 'protocol') {
  process.stdout.write('{"port":47001,"protocol":2}\\n')
  setInterval(() => {}, 1000)
} else if (mode === 'dies') {
  process.stderr.write('vornd: --upstream: invalid socket address syntax\\n')
  process.exit(2)
} else {
  setInterval(() => {}, 1000)
}
`

const dir = mkdtempSync(path.join(tmpdir(), 'vornd-launch-'))
const stub = path.join(dir, 'vornd-stub.cjs')
writeFileSync(stub, STUB)

const children: ChildProcess[] = []
const started: Vornd[] = []

function stubSpawn(mode: string): typeof spawn {
  return ((_binary: string, args: string[], options: object) => {
    const child = spawn(process.execPath, [stub, ...args], {
      ...options,
      env: { ...process.env, STUB_MODE: mode }
    })
    children.push(child)
    return child
  }) as unknown as typeof spawn
}

function exited(child: ChildProcess): Promise<number | null> {
  if (child.exitCode !== null || child.signalCode !== null) return Promise.resolve(child.exitCode)
  return new Promise((resolve) => child.once('exit', (code) => resolve(code)))
}

afterEach(() => {
  for (const v of started.splice(0)) v.stop()
  for (const child of children.splice(0)) child.kill()
})

describe('finding vornd', () => {
  const where = { packaged: true, resourcesPath: '/app/Resources', repoRoot: '/repo' }

  it('looks in the app resources when packaged', () => {
    const seen: string[] = []
    const found = findVornd(where, 'darwin', (f) => (seen.push(f), true))
    expect(found).toBe(path.join('/app/Resources', 'vornd', 'vornd'))
    expect(seen).toHaveLength(1)
  })

  it('looks for vornd.exe on Windows', () => {
    expect(findVornd(where, 'win32', () => true)).toBe(
      path.join('/app/Resources', 'vornd', 'vornd.exe')
    )
  })

  it('looks where build:core copies it in dev, then in cargo’s target directory', () => {
    const dev = { ...where, packaged: false }
    const release = path.join('/repo', 'packages', 'core', 'target', 'release', 'vornd')
    expect(findVornd(dev, 'linux', (f) => f === release)).toBe(release)
    expect(findVornd(dev, 'linux', () => true)).toBe(
      path.join('/repo', 'packages', 'core', 'vornd')
    )
  })

  it('answers null when the build has none', () => {
    expect(findVornd(where, 'linux', () => false)).toBeNull()
  })
})

describe('the port vornd forwards to', () => {
  it('is the port in a loopback url', () => {
    expect(upstreamPort('ws://127.0.0.1:50091/ws', null)).toBe(50091)
  })

  it('is the published port for a server reached through its socket', () => {
    expect(upstreamPort('ws+unix:///Users/x/.vorn/server.sock:/ws', 50091)).toBe(50091)
    expect(upstreamPort('ws+unix:///Users/x/.vorn/server.sock:/ws', null)).toBeNull()
  })

  it('is null when there is no port to read', () => {
    expect(upstreamPort('not a url', 50091)).toBeNull()
  })
})

describe('starting vornd', () => {
  it('resolves with the port it reports, forwarding to the server', async () => {
    const vornd = await startVornd('vornd', 50091, { spawnImpl: stubSpawn('ok') })
    started.push(vornd)
    expect(vornd.port).toBe(47001)
    expect(vornd.upstream).toBe(50091)
  })

  it('passes the session holder and its data directory when there is one', async () => {
    let seen: string[] = []
    const spawnImpl = ((binary: string, args: string[], options: object) => {
      seen = args
      return stubSpawn('ok')(binary, args, options as never)
    }) as unknown as typeof spawn
    started.push(await startVornd('vornd', 50091, { spawnImpl }))
    expect(seen).not.toContain('--sessiond')
    started.push(
      await startVornd('vornd', 50091, {
        spawnImpl,
        sessiond: { binary: '/app/vornd/vorn-sessiond', home: '/Users/x/.vorn' }
      })
    )
    expect(seen).toEqual([
      '--upstream',
      '127.0.0.1:50091',
      '--exit-with-stdin',
      '--sessiond',
      '/app/vornd/vorn-sessiond',
      '--home',
      '/Users/x/.vorn'
    ])
  })

  it('stops it by closing its stdin, and does not report that as an exit', async () => {
    const vornd = await startVornd('vornd', 50091, { spawnImpl: stubSpawn('ok') })
    const onExit = vi.fn()
    vornd.onExit(onExit)
    vornd.stop()
    await exited(children[0]!)
    expect(onExit).not.toHaveBeenCalled()
  })

  it('reports an exit nobody asked for', async () => {
    const vornd = await startVornd('vornd', 50091, { spawnImpl: stubSpawn('ok') })
    started.push(vornd)
    const why = new Promise<string>((resolve) => vornd.onExit(resolve))
    children[0]!.kill('SIGKILL')
    expect(await why).toMatch(/signal=SIGKILL/)
  })

  it('fails with what it printed when it exits before listening', async () => {
    await expect(startVornd('vornd', 50091, { spawnImpl: stubSpawn('dies') })).rejects.toThrow(
      /exited before it was listening \(code=2.*invalid socket address/
    )
  })

  it('fails, and ends it, when it says something other than a port', async () => {
    await expect(startVornd('vornd', 50091, { spawnImpl: stubSpawn('garbage') })).rejects.toThrow(
      /something other than where it listens/
    )
    await exited(children[0]!)
  })

  it('fails on a protocol this app does not know', async () => {
    await expect(startVornd('vornd', 50091, { spawnImpl: stubSpawn('protocol') })).rejects.toThrow(
      /protocol 2/
    )
    await exited(children[0]!)
  })

  it('gives up, and ends it, when it never says where it listens', async () => {
    await expect(
      startVornd('vornd', 50091, { spawnImpl: stubSpawn('silent'), timeoutMs: 300 })
    ).rejects.toThrow(/within 300ms/)
    await exited(children[0]!)
  })

  it('fails when the binary is not there', async () => {
    await expect(startVornd(path.join(dir, 'no-such-vornd'), 50091)).rejects.toThrow(
      /could not start vornd/
    )
  })
})

const built = findVornd({
  packaged: false,
  resourcesPath: '',
  repoRoot: path.resolve(__dirname, '..')
})

describe.runIf(built && existsSync(built))('the real vornd', () => {
  it('starts, listens on loopback, and exits when stopped', async () => {
    const children: ChildProcess[] = []
    const vornd = await startVornd(built!, 9, {
      spawnImpl: ((...a: Parameters<typeof spawn>) => {
        const child = spawn(...a)
        children.push(child)
        return child
      }) as typeof spawn
    })
    expect(vornd.port).toBeGreaterThan(0)
    const health = await fetch(`http://127.0.0.1:${vornd.port}/vornd/health`)
    // Nothing listens on port 9, so vornd is up and its server is not.
    expect(health.status).toBe(503)
    vornd.stop()
    expect(await exited(children[0]!)).toBe(0)
  })
})
