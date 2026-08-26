import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import {
  claimPublishedFiles,
  writePortFile,
  removePortFile,
  isPidAlive
} from '../packages/server/src/published-files'

/**
 * Who may write the names a machine uses to find its server.
 *
 * These used to test a copy of the server's logic, pasted into this file because
 * the real thing was inline in `startServer` and expensive to reach. The copy
 * drifted the moment the original moved, and a green run proved nothing about
 * the code that ships. It is a module now, and this points at it.
 *
 * "This process" is always `process.pid` here, as it is in production — the
 * functions take no pid argument, because a caller that can name someone else as
 * the owner is a caller that can claim a directory on their behalf. A live rival
 * is played by `process.ppid`, which vitest guarantees is alive and is not us.
 */

let dir: string
let portFile: string

const DEAD_PID = 2147483647 // max pid — nothing is running as this

beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-port-test-'))
  portFile = path.join(dir, 'ws-port')
})

afterEach(() => {
  fs.rmSync(dir, { recursive: true, force: true })
})

const read = (): { port?: number; pid?: number } | null => {
  try {
    return JSON.parse(fs.readFileSync(portFile, 'utf-8'))
  } catch {
    return null
  }
}

describe('claiming the directory', () => {
  it('claims it when nothing is published', () => {
    expect(claimPublishedFiles(dir)).toBe(true)
    writePortFile(dir, 53829, true)
    expect(read()).toEqual({ port: 53829, pid: process.pid })
  })

  it('claims it from a process that has died', () => {
    fs.writeFileSync(portFile, JSON.stringify({ port: 11111, pid: DEAD_PID }))

    expect(claimPublishedFiles(dir)).toBe(true)
    writePortFile(dir, 22222, true)
    expect(read()).toEqual({ port: 22222, pid: process.pid })
  })

  it('refuses it while another process is alive', () => {
    fs.writeFileSync(portFile, JSON.stringify({ port: 11111, pid: process.ppid }))

    expect(claimPublishedFiles(dir)).toBe(false)
  })

  it('claims it from the legacy plain-number format', () => {
    // Written by MCP's own discovery before it learned to record a pid. It names
    // no owner, so it stops nobody.
    fs.writeFileSync(portFile, '53829')

    expect(claimPublishedFiles(dir)).toBe(true)
    writePortFile(dir, 54000, true)
    expect(read()).toEqual({ port: 54000, pid: process.pid })
  })

  it('claims it from a record with no pid', () => {
    // MCP heals a missing port file by discovering the port and writing it back
    // without a pid. `judgeAdoption` is written to tolerate that record, so this
    // must too.
    fs.writeFileSync(portFile, JSON.stringify({ port: 11111 }))

    expect(claimPublishedFiles(dir)).toBe(true)
  })

  it('claims it back from itself, so a restart is not blocked by its own file', () => {
    fs.writeFileSync(portFile, JSON.stringify({ port: 11111, pid: process.pid }))

    expect(claimPublishedFiles(dir)).toBe(true)
    writePortFile(dir, 22222, true)
    expect(read()).toEqual({ port: 22222, pid: process.pid })
  })
})

describe('publishing without the claim', () => {
  it('writes nothing', () => {
    writePortFile(dir, 53829, false)
    expect(fs.existsSync(portFile)).toBe(false)
  })

  it('leaves a live rival file exactly as it found it', () => {
    const theirs = JSON.stringify({ port: 11111, pid: process.ppid })
    fs.writeFileSync(portFile, theirs)

    writePortFile(dir, 53829, false)
    removePortFile(dir, false)

    expect(fs.readFileSync(portFile, 'utf-8')).toBe(theirs)
  })
})

describe('removing it on the way out', () => {
  it('removes the file it published', () => {
    writePortFile(dir, 53829, true)
    removePortFile(dir, true)
    expect(fs.existsSync(portFile)).toBe(false)
  })

  it('leaves a file that has since become somebody elses', () => {
    // The claim was taken minutes ago and the flag still says yes. The file is
    // the thing being removed, so the file gets the last word: another server
    // named there is one this process must not unpublish.
    writePortFile(dir, 53829, true)
    fs.writeFileSync(portFile, JSON.stringify({ port: 54000, pid: process.ppid }))

    removePortFile(dir, true)

    expect(read()).toEqual({ port: 54000, pid: process.ppid })
  })

  it('does not throw when the file is already gone', () => {
    expect(() => removePortFile(dir, true)).not.toThrow()
  })
})

describe('asking whether a process is there', () => {
  it('says yes for this one', () => {
    expect(isPidAlive(process.pid)).toBe(true)
  })

  it('says no for one that has died', () => {
    expect(isPidAlive(DEAD_PID)).toBe(false)
  })

  it.each([0, -1, 1.5, Number.NaN])('says no for %s rather than asking', (pid) => {
    // Signal 0 to pid 0 targets this process group and always succeeds, so an
    // unchecked zero would report every dead server as alive.
    expect(isPidAlive(pid)).toBe(false)
  })
})
