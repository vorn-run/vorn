import { describe, it, expect, afterEach } from 'vitest'
import { EventEmitter } from 'node:events'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import * as pty from 'node-pty'
import { HANDOFF_MANIFEST_VERSION } from '@vornrun/shared/protocol'
import {
  receiveHandoff,
  announceServing,
  type HandoffChannel
} from '../packages/server/src/handoff/heir'
import { FIRST_PTY_SLOT, writeManifest } from '../packages/server/src/handoff/manifest'
import type { TerminalSession } from '@vornrun/shared/types'

/**
 * The receiving half, driven the way the outgoing server drives it.
 *
 * A replacement is a child with descriptors in its stdio array and a channel to
 * answer on. Both are faked here: `native.open` supplies a real pty to stand in
 * for an inherited one, and `process.send` is replaced so the handshake can be
 * answered on cue. What is under test is the decision -- when this process may
 * serve and when it must not.
 */
const slaves: number[] = []
const dirs: string[] = []

afterEach(() => {
  for (const fd of slaves.splice(0)) {
    try {
      fs.closeSync(fd)
    } catch {
      // Already closed.
    }
  }
  for (const dir of dirs.splice(0)) fs.rmSync(dir, { recursive: true, force: true })
})

/** A real pty master, standing in for one that arrived in the stdio array. */
function inheritedSlot(): number {
  const pair = (
    pty as unknown as {
      native: { open(cols: number, rows: number): { master: number; slave: number } }
    }
  ).native.open(80, 24)
  slaves.push(pair.slave)
  return pair.master
}

function manifestNaming(slot: number): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-heir-'))
  dirs.push(dir)
  const at = path.join(dir, 'handoff.json')
  writeManifest(at, {
    version: HANDOFF_MANIFEST_VERSION,
    donorPid: 4242,
    createdAt: Date.now(),
    panes: [{ slot, session: { id: 'pane-a' } as TerminalSession, pid: 999, cols: 80, rows: 24 }]
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
  it('takes the panes and reports that it has', async () => {
    const slot = inheritedSlot()
    const { channel: parent, sent } = channel('commit')

    const panes = await receiveHandoff(manifestNaming(slot), parent)
    expect(panes).toHaveLength(1)
    expect(panes?.[0]?.session.id).toBe('pane-a')
    expect(panes?.[0]?.cols).toBe(80)
    // Said before the commit is asked for: that word is what releases the endpoint.
    expect(sent).toEqual([{ kind: 'imported' }])
  })

  it('removes the manifest once it has been read', async () => {
    const slot = inheritedSlot()
    const { channel: parent } = channel('commit')
    const at = manifestNaming(slot)

    await receiveHandoff(at, parent)
    expect(fs.existsSync(at)).toBe(false)
  })

  it('refuses to serve when there is no channel to answer on', async () => {
    const mute: HandoffChannel = { on: () => undefined, off: () => undefined }
    expect(await receiveHandoff(manifestNaming(inheritedSlot()), mute)).toBeNull()
  })

  it('refuses to serve on a manifest it cannot read', async () => {
    const { channel: parent } = channel('commit')
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-heir-'))
    dirs.push(dir)
    const at = path.join(dir, 'handoff.json')
    fs.writeFileSync(at, 'half a jso')
    // Null, not "no panes": coming up serving nothing while holding descriptors
    // is how a person watches their terminals disappear.
    expect(await receiveHandoff(at, parent)).toBeNull()
    expect(await receiveHandoff(path.join(dir, 'absent.json'), parent)).toBeNull()
  })

  it('refuses to serve when a slot is not a terminal', async () => {
    const { channel: parent } = channel('commit')
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-heir-'))
    dirs.push(dir)
    const notATerminal = fs.openSync(path.join(dir, 'plain-file'), 'w')
    slaves.push(notATerminal)
    expect(notATerminal).toBeGreaterThanOrEqual(FIRST_PTY_SLOT)

    expect(await receiveHandoff(manifestNaming(notATerminal), parent)).toBeNull()
  })

  it('stands down when the outgoing server never commits', async () => {
    const slot = inheritedSlot()
    const { channel: parent, sent } = channel('disconnect')
    // The channel closing means the donor died mid-handoff: it never released the
    // endpoint, so there is nothing here to take.
    expect(await receiveHandoff(manifestNaming(slot), parent)).toBeNull()
    expect(sent).toEqual([{ kind: 'imported' }])
  })

  it('arrives with its readers paused', async () => {
    const slot = inheritedSlot()
    const { channel: parent } = channel('commit')
    const panes = await receiveHandoff(manifestNaming(slot), parent)

    const seen: string[] = []
    panes?.[0]?.pty.onData((d) => seen.push(d))
    fs.writeSync(slaves[slaves.length - 1] as number, 'written before anyone attached\n')
    await new Promise((r) => setTimeout(r, 150))
    // Nothing yet: the bytes wait in the kernel until `adoptPanes` resumes them,
    // which is the last moment at which they have somewhere to go.
    expect(seen).toEqual([])

    panes?.[0]?.pty.resume()
    await new Promise((r) => setTimeout(r, 150))
    expect(seen.join('')).toContain('written before anyone attached')
    panes?.[0]?.pty.kill()
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
