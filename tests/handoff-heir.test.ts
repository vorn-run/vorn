import { describe, it, expect, afterEach } from 'vitest'
import { EventEmitter } from 'node:events'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { HANDOFF_MANIFEST_VERSION } from '@vornrun/shared/protocol'
import {
  receiveHandoff,
  announceServing,
  type HandoffChannel
} from '../packages/server/src/handoff/heir'
import { FIRST_PTY_SLOT, writeManifest } from '../packages/server/src/handoff/manifest'
import type { TerminalSession } from '@vornrun/shared/types'

type HandoffPane = Parameters<typeof writeManifest>[1]['panes'][number]

/**
 * The receiving half, driven the way the outgoing server drives it.
 *
 * `process.send` is replaced with a fake channel so the handshake can be
 * answered on cue. What is under test is the decision -- when this process may
 * serve and when it must not.
 */
const dirs: string[] = []

afterEach(() => {
  for (const dir of dirs.splice(0)) fs.rmSync(dir, { recursive: true, force: true })
})

function manifest(panes: HandoffPane[] = []): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-heir-'))
  dirs.push(dir)
  const at = path.join(dir, 'handoff.json')
  writeManifest(at, {
    version: HANDOFF_MANIFEST_VERSION,
    donorPid: 4242,
    createdAt: Date.now(),
    panes
  })
  return at
}

/** A stand-in for the parent's channel, answering the handshake with `reply`. */
function channel(reply: 'commit' | 'disconnect' | 'silence'): {
  channel: HandoffChannel
  sent: unknown[]
} {
  const sent: unknown[] = []
  const events = new EventEmitter()
  const fake: HandoffChannel = {
    send: (message: unknown) => {
      sent.push(message)
      if ((message as { kind?: string })?.kind !== 'imported') return true
      // Next tick, so the caller is already waiting when the answer arrives.
      setImmediate(() => {
        if (reply === 'commit') events.emit('message', { kind: 'commit' })
        if (reply === 'disconnect') events.emit('disconnect')
      })
      return true
    },
    on: (event, listener) => events.on(event, listener as (...a: unknown[]) => void),
    off: (event, listener) => events.off(event, listener as (...a: unknown[]) => void)
  }
  return { channel: fake, sent }
}

describe('taking a handoff', () => {
  it('serves once the outgoing server commits, having said it is ready first', async () => {
    const { channel: parent, sent } = channel('commit')
    expect(await receiveHandoff(manifest(), parent)).toBe(true)
    // Said before the commit is asked for: that word is what releases the endpoint.
    expect(sent).toEqual([{ kind: 'imported' }])
  })

  it('removes the manifest once it has been read', async () => {
    const { channel: parent } = channel('commit')
    const at = manifest()
    await receiveHandoff(at, parent)
    expect(fs.existsSync(at)).toBe(false)
  })

  it('refuses to serve when there is no channel to answer on', async () => {
    const mute: HandoffChannel = { on: () => undefined, off: () => undefined }
    expect(await receiveHandoff(manifest(), mute)).toBe(false)
  })

  it('refuses to serve on a manifest it cannot read', async () => {
    const { channel: parent } = channel('commit')
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-heir-'))
    dirs.push(dir)
    const at = path.join(dir, 'handoff.json')
    fs.writeFileSync(at, 'half a jso')
    expect(await receiveHandoff(at, parent)).toBe(false)
    expect(await receiveHandoff(path.join(dir, 'absent.json'), parent)).toBe(false)
  })

  it('refuses a server that still holds terminals of its own, which keeps them', async () => {
    const { channel: parent, sent } = channel('commit')
    const at = manifest([
      {
        slot: FIRST_PTY_SLOT,
        session: { id: 'pane-a' } as TerminalSession,
        pid: 999,
        cols: 80,
        rows: 24
      }
    ])
    expect(await receiveHandoff(at, parent)).toBe(false)
    // Nothing said: the outgoing server goes on serving them.
    expect(sent).toEqual([])
  })

  it('stands down when the outgoing server never commits', async () => {
    const { channel: parent, sent } = channel('disconnect')
    // The channel closing means the donor died mid-handoff: it never released the
    // endpoint, so there is nothing here to take.
    expect(await receiveHandoff(manifest(), parent)).toBe(false)
    expect(sent).toEqual([{ kind: 'imported' }])
  })

  it('tells the outgoing server when it is serving', () => {
    const { channel: parent, sent } = channel('silence')
    announceServing(parent)
    expect(sent).toEqual([{ kind: 'serving' }])
  })

  it('survives a channel that has already gone when it announces', () => {
    const gone: HandoffChannel = {
      send: () => {
        throw new Error('channel closed')
      },
      on: () => undefined,
      off: () => undefined
    }
    // The outgoing server leaving early is survivable: this process is serving.
    expect(() => announceServing(gone)).not.toThrow()
  })
})
