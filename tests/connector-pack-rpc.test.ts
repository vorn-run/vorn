import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { IPC } from '@vornrun/shared/types'

const ROOT = resolve(__dirname, '..')
const read = (file: string): string => readFileSync(join(ROOT, file), 'utf8')

/**
 * A pack method has to be wired in five places, and a half-wired one fails at
 * runtime rather than at build: the renderer calls a method the main process
 * never registered a handler for, and the call hangs. Checking each hop here
 * fails on the commit that forgets one.
 */
describe('connector pack RPC wiring', () => {
  const channels = {
    CONNECTOR_INSTALL_PACK: 'connector:installPack',
    CONNECTOR_REMOVE_PACK: 'connector:removePack',
    CONNECTOR_ROLLBACK_PACK: 'connector:rollbackPack',
    CONNECTOR_LIST_PACKS: 'connector:listPacks',
    CONNECTOR_INSTALL_PROGRESS: 'connector:installProgress'
  } as const

  it('names every channel once, in one place', () => {
    for (const [key, channel] of Object.entries(channels)) {
      expect(IPC[key as keyof typeof IPC]).toBe(channel)
    }
  })

  it('declares the request methods in the protocol', () => {
    const protocol = read('packages/shared/src/protocol.ts')
    for (const channel of Object.values(channels)) {
      if (channel === channels.CONNECTOR_INSTALL_PROGRESS) continue
      expect(protocol).toContain(`'${channel}': {`)
    }
  })

  it('declares the progress push as a server notification', () => {
    const protocol = read('packages/shared/src/protocol.ts')
    expect(protocol).toContain(`'${channels.CONNECTOR_INSTALL_PROGRESS}': ConnectorInstallProgress`)
  })

  it('registers a server method for each request', () => {
    const methods = read('packages/server/src/register-methods.ts')
    for (const channel of Object.values(channels)) {
      if (channel === channels.CONNECTOR_INSTALL_PROGRESS) continue
      expect(methods).toContain(`registerMethod('${channel}'`)
    }
    expect(methods).toContain('IPC.CONNECTOR_INSTALL_PROGRESS')
  })

  it('forwards each request from the main process', () => {
    const handlers = read('src/main/ipc-handlers.ts')
    for (const key of Object.keys(channels)) {
      if (key === 'CONNECTOR_INSTALL_PROGRESS') continue
      expect(handlers).toContain(`safeHandle(IPC.${key}`)
    }
  })

  it('exposes each one on the preload, progress included', () => {
    const preload = read('src/preload/index.ts')
    for (const method of [
      'installConnectorPack',
      'removeConnectorPack',
      'rollbackConnectorPack',
      'listConnectorPacks',
      'onConnectorInstallProgress'
    ]) {
      expect(preload).toContain(`${method}:`)
    }
    // The push must hand back an unsubscribe, or a remounting panel leaks a
    // listener per mount and every install renders several times over.
    expect(preload).toContain(
      'ipcRenderer.removeListener(IPC.CONNECTOR_INSTALL_PROGRESS, listener)'
    )
  })
})
