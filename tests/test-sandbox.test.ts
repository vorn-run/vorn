import { randomUUID } from 'node:crypto'
import fs, { writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import electronLog from 'electron-log/main'
import { describe, expect, inject, it, vi } from 'vitest'
import log from '../src/main/logger'
import { dataDir } from '../packages/mcp/src/rpc-client'
import { appLogFiles, linesSince, snapshotLogs } from './helpers/app-log'
import { assertSandboxed, isInside } from './helpers/sandbox'

const realHome = inject('realHome')
const sandboxRoot = inject('sandboxRoot')
const repoRoot = inject('repoRoot')

describe('the test sandbox', () => {
  it('gives the file a temporary home and drops the live app data dir', () => {
    expect(os.homedir()).not.toBe(realHome)
    expect(isInside(os.homedir(), sandboxRoot)).toBe(true)
    expect(isInside(sandboxRoot, os.tmpdir())).toBe(true)
    expect(process.env.VORN_DATA_DIR).toBeUndefined()
    expect(process.env.VORN_SESSION_ID).toBeUndefined()
    expect(dataDir()).toBe(path.join(os.homedir(), '.vorn'))
  })

  it('writes the main-process log to the temporary home, never the real main.log', async () => {
    const real = snapshotLogs(appLogFiles(realHome))
    const token = `sandbox-${randomUUID()}`
    log.warn(token)
    log.error(new Error(token))

    const file = electronLog.transports.file.getFile().path
    expect(isInside(file, os.homedir())).toBe(true)
    await vi.waitFor(() => expect(fs.readFileSync(file, 'utf8')).toContain(token))
    expect(linesSince(real, token)).toEqual([])
  })

  it('refuses writes anywhere in the real home', () => {
    const target = path.join(realHome, '.vorn', `sandbox-${randomUUID()}`)
    const refused = /Test sandbox: refused to write/
    expect(() => fs.writeFileSync(target, 'x')).toThrow(refused)
    expect(() => writeFileSync(target, 'x')).toThrow(refused)
    expect(() => fs.appendFile(target, 'x', () => {})).toThrow(refused)
    expect(() => fs.promises.mkdir(path.join(realHome, 'Library', 'Logs', 'Vorn'))).toThrow(refused)
    expect(() => fs.openSync(pathToFileURL(target), 'a')).toThrow(refused)
    expect(() => fs.openSync(Buffer.from(target), fs.constants.O_WRONLY)).toThrow(refused)
    expect(() => fs.renameSync(path.join(os.tmpdir(), 'a'), target)).toThrow(refused)
    expect(() => fs.createWriteStream(target)).toThrow(refused)
  })

  it('refuses reads of the real ~/.vorn', () => {
    const target = path.join(realHome, '.vorn', 'port')
    const refused = /Test sandbox: refused to read/
    expect(() => fs.readFileSync(target)).toThrow(refused)
    expect(() => fs.existsSync(target)).toThrow(refused)
    expect(() => fs.openSync(target, 'r')).toThrow(refused)
    expect(() => fs.createReadStream(target)).toThrow(refused)
  })

  it('allows the temporary directory, the checkout and descriptors', () => {
    expect(() => assertSandboxed(path.join(os.tmpdir(), 'x'), 'write')).not.toThrow()
    expect(() => assertSandboxed(path.join(repoRoot, 'coverage', 'x'), 'write')).not.toThrow()
    expect(() => assertSandboxed(path.join(realHome, 'Library', 'Logs'), 'read')).not.toThrow()
    expect(() => assertSandboxed(3, 'write')).not.toThrow()
    expect(() => assertSandboxed(new URL('https://example.com/'), 'write')).not.toThrow()
  })
})

describe('the real app log guard', () => {
  it('reports only appended lines that mention the marker, rereading a rotated file', () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-log-guard-'))
    const file = path.join(dir, 'main.log')
    fs.writeFileSync(file, 'old line from /checkout/tests/a.test.ts\n')
    const missing = path.join(dir, 'missing.log')
    const snapshot = snapshotLogs([file, missing])
    expect(linesSince(snapshot, '/checkout')).toEqual([])

    fs.appendFileSync(file, 'app says hello\n  at /checkout/src/main/x.ts:1:1\n')
    expect(linesSince(snapshot, '/checkout')).toEqual([`${file}: at /checkout/src/main/x.ts:1:1`])

    fs.writeFileSync(file, '/checkout\n')
    expect(linesSince(snapshot, '/checkout')).toEqual([`${file}: /checkout`])
    fs.rmSync(dir, { recursive: true, force: true })
  })

  it('names main.log where electron-log keeps it on this platform', () => {
    const home = path.join(os.tmpdir(), 'no-such-home')
    const expected =
      process.platform === 'darwin'
        ? path.join(home, 'Library', 'Logs', 'Vorn', 'main.log')
        : process.platform === 'win32'
          ? path.join('C:\\AppData', 'Vorn', 'logs', 'main.log')
          : path.join(home, '.config', 'Vorn', 'logs', 'main.log')
    expect(appLogFiles(home, { APPDATA: 'C:\\AppData' })).toContain(expected)
  })
})
