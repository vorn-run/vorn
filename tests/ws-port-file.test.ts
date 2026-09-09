import { describe, it, expect, vi, beforeEach } from 'vitest'
import path from 'node:path'
import os from 'node:os'

// Mock ws so importing the rpc client doesn't pull in the real WebSocket
vi.mock('ws', () => ({ WebSocket: vi.fn() }))

// Mock node:fs so readPort() sees our controlled data
const mockReadFileSync = vi.fn()
vi.mock('node:fs', () => ({
  default: {
    readFileSync: (...args: unknown[]) => mockReadFileSync(...args),
    existsSync: () => true,
    mkdirSync: () => undefined,
    writeFileSync: () => undefined
  },
  readFileSync: (...args: unknown[]) => mockReadFileSync(...args),
  existsSync: () => true,
  mkdirSync: () => undefined,
  writeFileSync: () => undefined
}))

// Mock child_process so discoverPort() doesn't find the real running server
vi.mock('node:child_process', () => ({
  execFileSync: () => {
    throw new Error('mocked')
  }
}))

const PORT_FILE = path.join(os.homedir(), '.vorn', 'ws-port')

describe('ws-port file parsing (readPort via isServerRunning)', () => {
  beforeEach(() => {
    vi.resetModules()
    mockReadFileSync.mockReset()
  })

  async function loadIsServerRunning() {
    const mod = await import('../packages/server/src/rpc-client')
    return mod.isServerRunning
  }

  it('returns true for JSON format with port and pid', async () => {
    // Use current PID so the liveness check passes
    mockReadFileSync.mockReturnValue(JSON.stringify({ port: 53829, pid: process.pid }))
    const isServerRunning = await loadIsServerRunning()
    expect(isServerRunning()).toBe(true)
  })

  it('returns true for legacy plain-number format', async () => {
    mockReadFileSync.mockReturnValue('53829')
    const isServerRunning = await loadIsServerRunning()
    expect(isServerRunning()).toBe(true)
  })

  it('returns true for plain number with trailing newline', async () => {
    mockReadFileSync.mockReturnValue('53829\n')
    const isServerRunning = await loadIsServerRunning()
    expect(isServerRunning()).toBe(true)
  })

  it('returns false when file does not exist', async () => {
    mockReadFileSync.mockImplementation(() => {
      throw new Error('ENOENT')
    })
    const isServerRunning = await loadIsServerRunning()
    expect(isServerRunning()).toBe(false)
  })

  it('returns false for empty file', async () => {
    mockReadFileSync.mockReturnValue('')
    const isServerRunning = await loadIsServerRunning()
    expect(isServerRunning()).toBe(false)
  })

  it('returns false for non-numeric content', async () => {
    mockReadFileSync.mockReturnValue('not-a-port')
    const isServerRunning = await loadIsServerRunning()
    expect(isServerRunning()).toBe(false)
  })

  it('returns false for JSON with invalid port', async () => {
    mockReadFileSync.mockReturnValue(JSON.stringify({ port: -1, pid: 1234 }))
    const isServerRunning = await loadIsServerRunning()
    expect(isServerRunning()).toBe(false)
  })

  it('returns false for JSON with zero port', async () => {
    mockReadFileSync.mockReturnValue(JSON.stringify({ port: 0, pid: 1234 }))
    const isServerRunning = await loadIsServerRunning()
    expect(isServerRunning()).toBe(false)
  })

  it('returns false for JSON without port field', async () => {
    mockReadFileSync.mockReturnValue(JSON.stringify({ pid: 1234 }))
    const isServerRunning = await loadIsServerRunning()
    expect(isServerRunning()).toBe(false)
  })

  it('returns false for JSON with dead pid (stale port file)', async () => {
    mockReadFileSync.mockReturnValue(JSON.stringify({ port: 53829, pid: 999999 }))
    const isServerRunning = await loadIsServerRunning()
    expect(isServerRunning()).toBe(false)
  })

  it('returns true for JSON with port but no pid (forward compat)', async () => {
    mockReadFileSync.mockReturnValue(JSON.stringify({ port: 8080 }))
    const isServerRunning = await loadIsServerRunning()
    expect(isServerRunning()).toBe(true)
  })

  it('reads from the correct path', async () => {
    mockReadFileSync.mockReturnValue(JSON.stringify({ port: 53829, pid: process.pid }))
    const isServerRunning = await loadIsServerRunning()
    isServerRunning()
    expect(mockReadFileSync).toHaveBeenCalledWith(PORT_FILE, 'utf-8')
  })
})
