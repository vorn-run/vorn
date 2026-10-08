import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { IPC } from '@vornrun/shared/types'

const ROOT = resolve(__dirname, '..')
const read = (file: string): string => readFileSync(join(ROOT, file), 'utf8')

/** A half-wired method hangs at runtime rather than failing at build. */
describe('extension RPC wiring', () => {
  const requests = {
    EXTENSION_LIST: 'extension:list',
    EXTENSION_ACTIVATION: 'extension:activation',
    EXTENSION_FOOTER_ITEMS: 'extension:footerItems',
    EXTENSION_OPEN_PANE: 'extension:openPane',
    EXTENSION_CLOSE_PANE: 'extension:closePane',
    EXTENSION_RUN_HANDLER: 'extension:runHandler',
    EXTENSION_MATCH_LINKS: 'extension:matchLinks'
  } as const

  const pushes = {
    EXTENSION_SELECTION_REQUEST: 'extension:selectionRequest',
    EXTENSION_SELECTION_RESULT: 'extension:selectionResult'
  } as const

  it('names every channel once, in one place', () => {
    for (const [key, channel] of Object.entries({ ...requests, ...pushes })) {
      expect(IPC[key as keyof typeof IPC]).toBe(channel)
    }
  })

  it('declares the request methods in the protocol', () => {
    const protocol = read('packages/shared/src/protocol.ts')
    for (const channel of Object.values(requests)) {
      expect(protocol).toContain(`'${channel}': {`)
    }
  })

  it('declares the pushes each in the direction it travels', () => {
    const protocol = read('packages/shared/src/protocol.ts')
    const server = protocol.slice(protocol.indexOf('export interface ServerNotifications'))
    const client = protocol.slice(protocol.indexOf('export interface ClientNotifications'))
    expect(server).toContain(`'${pushes.EXTENSION_SELECTION_REQUEST}': {`)
    expect(server).toContain(`'${requests.EXTENSION_FOOTER_ITEMS}': {`)
    expect(client).toContain(`'${pushes.EXTENSION_SELECTION_RESULT}': {`)
  })

  it('answers each request in vornd', () => {
    const host = read('packages/core/crates/vornd/src/native/extensions/mod.rs')
    const methods = host.slice(host.indexOf('pub const METHODS'), host.indexOf('];'))
    for (const channel of Object.values(requests)) {
      expect(methods).toContain(`"${channel}"`)
      expect(host).toContain(`"${channel}" =>`)
    }
    expect(host).toContain(`SELECTION_RESULT: &str = "${pushes.EXTENSION_SELECTION_RESULT}"`)
  })

  it('forwards each request from the main process', () => {
    const handlers = read('src/main/ipc-handlers.ts')
    for (const key of Object.keys(requests)) {
      expect(handlers).toContain(`IPC.${key}`)
    }
    expect(handlers).toContain(`IPC.${'EXTENSION_SELECTION_RESULT'}`)
  })

  it('exposes each of them to a window', () => {
    const preload = read('src/preload/index.ts')
    for (const key of Object.keys({ ...requests, ...pushes })) {
      expect(preload).toContain(`IPC.${key}`)
    }
  })

  it('exposes each of them to a browser client', () => {
    const shim = read('packages/web/src/api-shim.ts')
    for (const channel of Object.values({ ...requests, ...pushes })) {
      expect(shim).toContain(`'${channel}'`)
    }
  })
})
