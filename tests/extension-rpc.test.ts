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
    EXTENSION_OPEN_PANE: 'extension:openPane',
    EXTENSION_CLOSE_PANE: 'extension:closePane'
  } as const

  it('names every channel once, in one place', () => {
    for (const [key, channel] of Object.entries(requests)) {
      expect(IPC[key as keyof typeof IPC]).toBe(channel)
    }
  })

  it('declares the request methods in the protocol', () => {
    const protocol = read('packages/shared/src/protocol.ts')
    for (const channel of Object.values(requests)) {
      expect(protocol).toContain(`'${channel}': {`)
    }
  })

  it('answers each request in vornd', () => {
    const host = read('packages/core/crates/vornd/src/native/extensions/mod.rs')
    const methods = host.slice(host.indexOf('pub const METHODS'), host.indexOf('];'))
    for (const channel of Object.values(requests)) {
      expect(methods).toContain(`"${channel}"`)
      expect(host).toContain(`"${channel}" =>`)
    }
  })
})
