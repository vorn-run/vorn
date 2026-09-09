import { describe, it, expect, vi } from 'vitest'

vi.mock('electron', () => ({
  app: { getPath: () => '/Applications/Vorn.app/Contents/MacOS/Vorn' }
}))
vi.mock('../src/main/logger', () => ({ default: { warn: () => {}, error: () => {} } }))

import { shimScript } from '../src/main/cli-shim'

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
  })
})
