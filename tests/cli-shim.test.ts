import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

vi.mock('electron', () => ({
  app: { getPath: () => '/Applications/Vorn.app/Contents/MacOS/Vorn' }
}))

// Which directories exist and are writable is the machine's business, not the test's.
const writable = vi.hoisted(() => new Set<string>())
vi.mock('node:fs', () => ({
  default: {
    accessSync: (dir: string) => {
      if (!writable.has(dir)) throw new Error('not writable')
    },
    constants: { W_OK: 2 },
    existsSync: () => false,
    mkdirSync: () => undefined,
    writeFileSync: () => undefined,
    chmodSync: () => undefined
  }
}))
vi.mock('../src/main/logger', () => ({ default: { warn: () => {}, error: () => {} } }))

import os from 'node:os'
import path from 'node:path'
import { onPath, shimDirectory, shimScript } from '../src/main/cli-shim'

const MAC = {
  exe: '/Applications/Vorn.app/Contents/MacOS/Vorn',
  resources: '/Applications/Vorn.app/Contents/Resources'
}

describe('the vorn command the app writes', () => {
  it('runs the CLI inside the app on macOS, and opens the app when given nothing', () => {
    const script = shimScript(MAC, 'darwin')

    expect(script.startsWith('#!/bin/sh')).toBe(true)
    expect(script).toContain('exec open -a "/Applications/Vorn.app"')
    expect(script).toContain('ELECTRON_RUN_AS_NODE=1')
    expect(script).toContain('RESOURCES="/Applications/Vorn.app/Contents/Resources"')
    expect(script).toContain('${RESOURCES}/server/cli.cjs')
    expect(script).toContain('app.asar.unpacked/node_modules')
  })

  it('names the AppImage rather than the mount it is unpacked into', () => {
    const script = shimScript(
      {
        exe: '/tmp/.mount_Vorn12/vorn',
        resources: '/tmp/.mount_Vorn12/resources',
        appImage: '/home/j/.local/lib/vorn/Vorn.AppImage'
      },
      'linux'
    )

    expect(script).toContain('/home/j/.local/lib/vorn/Vorn.AppImage')
    expect(script).not.toContain('.mount_Vorn12')
    // The entry point is only knowable from inside, so it is resolved there.
    expect(script).toContain('process.env.APPDIR')
  })

  it('writes a batch file on Windows, starting the app when given nothing', () => {
    const script = shimScript(
      {
        exe: 'C:\\Users\\j\\AppData\\Local\\Programs\\Vorn\\Vorn.exe',
        resources: 'C:\\Users\\j\\AppData\\Local\\Programs\\Vorn\\resources'
      },
      'win32'
    )

    expect(script.startsWith('@echo off')).toBe(true)
    expect(script).toContain('if "%~1"=="" (')
    expect(script).toContain('set "ELECTRON_RUN_AS_NODE=1"')
    expect(script).toContain('%*')
    // Written from a Mac in this test, and still a Windows path.
    expect(script).toContain('\\resources\\server\\cli.cjs')
    expect(script).not.toContain('/resources/')
    expect(script.split('\n').every((line) => line === '' || line.endsWith('\r'))).toBe(true)
  })
})

describe('where the command goes', () => {
  const userBin = path.join(os.homedir(), '.local', 'bin')
  const originalPath = process.env.PATH

  beforeEach(() => {
    writable.clear()
  })

  afterEach(() => {
    process.env.PATH = originalPath
  })

  it('prefers a writable directory the shell already searches', () => {
    writable.add('/usr/local/bin')
    writable.add(userBin)
    process.env.PATH = `${userBin}:/usr/bin`

    expect(shimDirectory()).toBe(userBin)
  })

  it('takes a writable directory over none when the shell searches neither', () => {
    writable.add('/usr/local/bin')
    process.env.PATH = '/usr/bin'

    expect(shimDirectory()).toBe('/usr/local/bin')
  })

  it('reads a Windows PATH entry whatever case it was written in', () => {
    process.env.PATH = 'C:\\Users\\J\\AppData\\Local\\Programs\\Vorn'

    expect(onPath('C:\\users\\j\\appdata\\local\\programs\\vorn', 'win32')).toBe(true)
    expect(onPath('C:\\users\\j\\appdata\\local\\programs\\vorn', 'linux')).toBe(false)
  })

  it('falls back to the one directory it may always create', () => {
    process.env.PATH = '/usr/bin'

    expect(shimDirectory()).toBe(userBin)
  })
})
