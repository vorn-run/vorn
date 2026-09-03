import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

const userData = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-claims-'))

vi.mock('electron', () => ({ app: { getPath: () => userData } }))
vi.mock('../src/main/logger', () => ({ default: { warn: () => {}, error: () => {} } }))

const alive = vi.fn<(pid: number) => boolean>()
vi.mock('../src/main/server/server-adoption', () => ({ isPidAlive: (pid: number) => alive(pid) }))

const FILE = path.join(userData, 'device-claims.json')

const { foreignClaim, recordClaim, dropClaim, dropAllClaimsForThisProcess } =
  await import('../src/main/device-claims-store')

/** Someone else's Vorn, still running. */
const OTHER = process.pid + 1000

beforeEach(() => {
  alive.mockReset()
  alive.mockReturnValue(true)
  fs.rmSync(FILE, { force: true })
})

afterEach(() => {
  fs.rmSync(FILE, { force: true })
})

function writeRecord(value: unknown): void {
  fs.writeFileSync(FILE, JSON.stringify(value))
}

describe('a device another Vorn is driving', () => {
  it('is refused, and names the process holding it', () => {
    writeRecord({ 'udid-1': { pid: OTHER, sessionId: 'sess-a' } })
    expect(foreignClaim('udid-1')).toEqual({ pid: OTHER, sessionId: 'sess-a' })
  })

  it('is free again once that process is gone', () => {
    writeRecord({ 'udid-1': { pid: OTHER, sessionId: 'sess-a' } })
    alive.mockReturnValue(false)
    expect(foreignClaim('udid-1')).toBeNull()
  })

  it('is not foreign when it is our own', () => {
    recordClaim('udid-1', 'sess-a')
    expect(foreignClaim('udid-1')).toBeNull()
  })

  it('is free when nothing has claimed it', () => {
    expect(foreignClaim('udid-1')).toBeNull()
  })
})

describe('reading a record that has gone wrong', () => {
  it('treats an unparseable file as no claims rather than as every device held', () => {
    fs.writeFileSync(FILE, '{ not json')
    expect(foreignClaim('udid-1')).toBeNull()
  })

  it('drops an entry with no usable pid', () => {
    for (const pid of [0, -1, 'x', null, 1.5]) {
      writeRecord({ 'udid-1': { pid, sessionId: 'sess-a' } })
      expect(foreignClaim('udid-1')).toBeNull()
    }
  })

  it('drops an entry with no session', () => {
    writeRecord({ 'udid-1': { pid: OTHER } })
    expect(foreignClaim('udid-1')).toBeNull()
  })

  it('survives a file that is not an object at all', () => {
    for (const raw of ['[]', 'null', '"udid-1"', '7']) {
      fs.writeFileSync(FILE, raw)
      expect(() => foreignClaim('udid-1')).not.toThrow()
      expect(foreignClaim('udid-1')).toBeNull()
    }
  })
})

describe('letting go', () => {
  it('removes our own claim', () => {
    recordClaim('udid-1', 'sess-a')
    dropClaim('udid-1')
    expect(JSON.parse(fs.readFileSync(FILE, 'utf-8'))['udid-1']).toBeUndefined()
  })

  it('leaves a live claim of another Vorn alone', () => {
    writeRecord({ 'udid-1': { pid: OTHER, sessionId: 'sess-a' } })
    recordClaim('udid-2', 'sess-b')
    dropClaim('udid-2')
    expect(foreignClaim('udid-1')).not.toBeNull()
  })

  it('sweeps what a Vorn that crashed left behind', () => {
    writeRecord({
      'udid-1': { pid: OTHER, sessionId: 'sess-a' },
      'udid-2': { pid: process.pid, sessionId: 'sess-b' }
    })
    alive.mockImplementation((pid) => pid === process.pid)
    dropClaim('udid-2')
    expect(JSON.parse(fs.readFileSync(FILE, 'utf-8'))).toEqual({})
  })

  it('gives up everything this process holds on the way out', () => {
    writeRecord({
      'udid-1': { pid: OTHER, sessionId: 'sess-a' },
      'udid-2': { pid: process.pid, sessionId: 'sess-b' }
    })
    dropAllClaimsForThisProcess()
    const after = JSON.parse(fs.readFileSync(FILE, 'utf-8'))
    expect(after['udid-2']).toBeUndefined()
    expect(after['udid-1']).toEqual({ pid: OTHER, sessionId: 'sess-a' })
  })
})

describe('writing the record', () => {
  it('never leaves a half-written file where a reader could see it', () => {
    // `read` answers "no claims" to anything it cannot parse, which is right for
    // a file that was never written and catastrophic for one that was half
    // written: every device reads as free and a second Vorn takes one this
    // process is driving. The swap has to be atomic, so it goes through rename.
    recordClaim('udid-1', 'sess-a')
    const renamed: string[] = []
    const realRename = fs.renameSync
    const spy = vi.spyOn(fs, 'renameSync').mockImplementation((from, to) => {
      renamed.push(String(to))
      return realRename(from, to)
    })
    const written = vi.spyOn(fs, 'writeFileSync')

    recordClaim('udid-2', 'sess-b')

    expect(renamed).toEqual([FILE])
    // Written beside the target, not over it.
    expect(String(written.mock.calls[0][0])).not.toBe(FILE)
    expect(String(written.mock.calls[0][0]).startsWith(FILE)).toBe(true)
    spy.mockRestore()
    written.mockRestore()
  })

  it('leaves no scratch file behind when the swap fails', () => {
    const spy = vi.spyOn(fs, 'renameSync').mockImplementation(() => {
      throw new Error('EXDEV')
    })
    recordClaim('udid-1', 'sess-a')
    spy.mockRestore()

    const leftovers = fs.readdirSync(userData).filter((f) => f.includes('device-claims'))
    expect(leftovers).toEqual([])
  })

  it('keeps the previous record readable when a write fails outright', () => {
    recordClaim('udid-1', 'sess-a')
    const spy = vi.spyOn(fs, 'renameSync').mockImplementation(() => {
      throw new Error('ENOSPC')
    })
    recordClaim('udid-2', 'sess-b')
    spy.mockRestore()

    // The old record survives intact rather than being truncated into nonsense.
    expect(JSON.parse(fs.readFileSync(FILE, 'utf-8'))['udid-1']).toBeDefined()
  })
})
