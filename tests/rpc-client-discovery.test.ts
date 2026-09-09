import { describe, it, expect, vi, beforeEach } from 'vitest'
import path from 'node:path'
import os from 'node:os'

vi.mock('ws', () => ({ WebSocket: vi.fn() }))

const readFileSync = vi.fn()
const writeFileSync = vi.fn()
vi.mock('node:fs', () => {
  const fs = {
    readFileSync: (...args: unknown[]) => readFileSync(...args),
    existsSync: () => true,
    mkdirSync: () => undefined,
    writeFileSync: (...args: unknown[]) => writeFileSync(...args)
  }
  return { default: fs, ...fs }
})

// One Vorn listening on this machine, which is what discovery goes looking for.
const execFileSync = vi.fn(
  (..._args: unknown[]) => 'Vorn  71718 someone  26u  IPv4  0t0  TCP *:50091 (LISTEN)\n'
)
vi.mock('node:child_process', () => ({ execFileSync }))

/**
 * A port file that is not there.
 *
 * The interesting case: with nothing to read, the client either goes looking for
 * a Vorn process or does not, and which it does depends on whether a data
 * directory was named.
 */
function noPortFile(): void {
  readFileSync.mockImplementation(() => {
    throw new Error('ENOENT')
  })
}

async function load() {
  return import('../packages/server/src/rpc-client')
}

beforeEach(() => {
  vi.resetModules()
  readFileSync.mockReset()
  writeFileSync.mockReset()
  execFileSync.mockClear()
  delete process.env.VORN_DATA_DIR
})

describe('finding the server', () => {
  it('reads the port file inside a named data directory', async () => {
    readFileSync.mockReturnValue(JSON.stringify({ port: 4321, pid: process.pid }))
    const { useDataDir, isServerRunning, dataDir } = await load()

    useDataDir('/tmp/elsewhere')
    expect(isServerRunning()).toBe(true)
    expect(dataDir()).toBe('/tmp/elsewhere')
    expect(readFileSync).toHaveBeenCalledWith(path.join('/tmp/elsewhere', 'ws-port'), 'utf-8')
  })

  it('looks for a running Vorn when nothing named a directory', async () => {
    noPortFile()
    const { isServerRunning } = await load()

    expect(isServerRunning()).toBe(true)
    expect(execFileSync).toHaveBeenCalled()
    expect(writeFileSync).toHaveBeenCalledWith(
      path.join(os.homedir(), '.vorn', 'ws-port'),
      JSON.stringify({ port: 50091 }),
      'utf-8'
    )
  })

  it('does not answer with somebody else’s server when a directory was named', async () => {
    noPortFile()
    const { useDataDir, isServerRunning } = await load()

    useDataDir('/tmp/elsewhere')
    expect(isServerRunning()).toBe(false)
    expect(execFileSync).not.toHaveBeenCalled()
    // Nor heal that directory with a port belonging to another server.
    expect(writeFileSync).not.toHaveBeenCalled()
  })

  it('points at the port file it actually looked for, not the default one', async () => {
    noPortFile()
    const { useDataDir, rpcCall } = await load()

    useDataDir('/tmp/elsewhere')
    await expect(rpcCall('terminal:listActive')).rejects.toThrow(
      path.join('/tmp/elsewhere', 'ws-port')
    )
  })

  it('treats an empty override as no override at all', async () => {
    readFileSync.mockReturnValue(JSON.stringify({ port: 4321, pid: process.pid }))
    process.env.VORN_DATA_DIR = '/tmp/env-dir'
    const { useDataDir, dataDir } = await load()

    useDataDir('   ')
    expect(dataDir()).toBe('/tmp/env-dir')
  })

  it('treats VORN_DATA_DIR the same way', async () => {
    noPortFile()
    process.env.VORN_DATA_DIR = '/tmp/env-dir'
    const { isServerRunning } = await load()

    expect(isServerRunning()).toBe(false)
    expect(execFileSync).not.toHaveBeenCalled()
  })
})
