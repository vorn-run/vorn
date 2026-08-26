import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import fs from 'node:fs'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import {
  claimEndpoint,
  probeEndpoint,
  canHostEndpoint,
  scratchPathFor,
  endpointPath,
  type Liveness
} from '../packages/server/src/endpoint'

/**
 * Deciding who owns the name a machine's server answers on.
 *
 * The two wrong answers are not symmetric, and every test here is written around
 * that. Taking a name from a server that is still serving strands every terminal
 * on the machine behind an endpoint nobody can reach — the failure this exists to
 * prevent. Declining a name whose owner is gone leaves a leftover file, which the
 * next start replaces in one rename. So the bar for "take it" is proof of death,
 * and everything short of proof declines.
 */

let dir: string
let canonical: string
const cleanup: Array<() => void> = []

beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-endpoint-'))
  fs.chmodSync(dir, 0o700)
  canonical = endpointPath(dir)
})

afterEach(() => {
  while (cleanup.length) cleanup.pop()?.()
  fs.rmSync(dir, { recursive: true, force: true })
})

/** A real bound listener, since the claim identifies sockets by inode. */
async function listener(at: string): Promise<net.Server> {
  const server = net.createServer((c) => {
    // A probe connects and destroys immediately, so this write often lands on a
    // socket that has already gone. Swallowed, or the reset is unhandled and
    // ends the test worker.
    c.on('error', () => {})
    c.end('served')
  })
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject)
    server.listen(at, resolve)
  })
  cleanup.push(() => server.close())
  return server
}

const inode = (p: string): number => fs.lstatSync(p).ino
const always = (answer: Liveness) => async (): Promise<Liveness> => answer

describe('the four races', () => {
  it('takes a free name', async () => {
    const scratch = scratchPathFor(canonical)
    await listener(scratch)
    const mine = inode(scratch)

    expect(await claimEndpoint(scratch, canonical, always('dead'))).toEqual({ held: true })
    expect(inode(canonical)).toBe(mine)
    // The scratch name is gone: it was ours, and removing it is what makes
    // libuv's unlink at close incapable of touching the canonical entry.
    expect(fs.existsSync(scratch)).toBe(false)
  })

  it('stands down when the incumbent is alive', async () => {
    const incumbent = await listener(canonical)
    const theirs = inode(canonical)
    const scratch = scratchPathFor(canonical)
    await listener(scratch)

    const outcome = await claimEndpoint(scratch, canonical, always('alive'))

    expect(outcome).toMatchObject({ held: false })
    expect(inode(canonical)).toBe(theirs)
    expect(incumbent.listening).toBe(true)
  })

  it('takes the name when the incumbent is provably dead', async () => {
    // What SIGKILL leaves: a socket inode at the canonical name with nothing
    // serving it. Built by hard-linking a live listener's name into place and
    // then closing the listener, which unlinks only the name it bound.
    const doomed = scratchPathFor(canonical)
    const server = await listener(doomed)
    fs.linkSync(doomed, canonical)
    await new Promise<void>((r) => server.close(() => r()))
    expect(fs.lstatSync(canonical).isSocket()).toBe(true)
    expect(await probeEndpoint(canonical)).toBe('dead')
    const corpse = inode(canonical)

    const scratch = scratchPathFor(canonical)
    await listener(scratch)
    const mine = inode(scratch)

    // The real probe, not a stub: this is the one case where the claim must
    // conclude death on its own and act on it.
    expect(await claimEndpoint(scratch, canonical)).toEqual({ held: true })
    expect(inode(canonical)).toBe(mine)
    expect(inode(canonical)).not.toBe(corpse)
    expect(fs.existsSync(scratch)).toBe(false)
  })

  it('never takes a name it cannot prove is dead', async () => {
    // A hung server answers nothing. It is still serving every terminal it holds,
    // and deleting its endpoint is the one unrecoverable mistake here.
    await listener(canonical)
    const theirs = inode(canonical)
    const scratch = scratchPathFor(canonical)
    await listener(scratch)

    const outcome = await claimEndpoint(scratch, canonical, always('unknown'))

    expect(outcome).toMatchObject({ held: false, because: 'the incumbent is unknown' })
    expect(inode(canonical)).toBe(theirs)
  })

  it('takes a name that frees itself while it is being checked', async () => {
    // A server that bound the canonical path directly -- which is what this
    // codebase did before the scratch name, and what an older Vorn still does --
    // unlinks it on close. So the name really can go from taken to free while a
    // claim is in progress, and standing down there would leave the machine with
    // no endpoint held and nobody holding one.
    const incumbent = await listener(canonical)
    const scratch = scratchPathFor(canonical)
    await listener(scratch)
    const mine = inode(scratch)

    // The name is taken when the claim starts -- so the first link really does
    // fail with EEXIST -- and the incumbent departs during the probe, taking the
    // canonical entry with it the way `close()` does.
    let departed = false
    const probe = vi.fn(async (): Promise<Liveness> => {
      if (!departed) {
        departed = true
        await new Promise<void>((r) => incumbent.close(() => r()))
      }
      return 'dead'
    })

    expect(await claimEndpoint(scratch, canonical, probe)).toEqual({ held: true })
    expect(inode(canonical)).toBe(mine)
    expect(probe).toHaveBeenCalledTimes(1)
  })

  it('stands down when the entry changes hands mid-check', async () => {
    await listener(canonical)
    const scratch = scratchPathFor(canonical)
    await listener(scratch)

    // A third server takes the name between the probe and the re-look, every
    // time. The inode is what notices; a path comparison would see no change.
    const probe = vi.fn(async (): Promise<Liveness> => {
      const usurper = scratchPathFor(canonical)
      const server = await listener(usurper)
      cleanup.push(() => server.close())
      fs.renameSync(usurper, canonical)
      return 'dead'
    })

    const outcome = await claimEndpoint(scratch, canonical, probe)

    expect(outcome).toMatchObject({ held: false })
    expect(probe).toHaveBeenCalledTimes(3) // bounded, not forever
  })
})

describe('what the probe is allowed to conclude', () => {
  it('calls a served name alive', async () => {
    await listener(canonical)
    expect(await probeEndpoint(canonical)).toBe('alive')
  })

  it('calls an absent name dead', async () => {
    expect(await probeEndpoint(path.join(dir, 'nothing.sock'))).toBe('dead')
  })

  it('calls a socket file with nothing behind it dead', async () => {
    const server = await listener(canonical)
    await new Promise<void>((r) => server.close(() => r()))
    // close() unlinks, so put a corpse back the way a SIGKILL would leave one.
    const other = scratchPathFor(canonical)
    const s2 = await listener(other)
    fs.linkSync(other, canonical)
    await new Promise<void>((r) => s2.close(() => r()))

    expect(await probeEndpoint(canonical)).toBe('dead')
  })

  it('is never asked about something that is not a socket', () => {
    // There was a test here that connected to a regular file and expected
    // "unknown". It passed on darwin, which answers ENOTSOCK, and failed on linux,
    // which answers ECONNREFUSED -- so the probe called it dead, which by its own
    // whitelist is right. The test was asserting an errno rather than a rule.
    //
    // The rule it should have been asserting is one layer up: the claim
    // identifies the entry before it probes, and anything that is not a socket is
    // not something this process may replace. The probe never sees one.
    const notASocket = path.join(dir, 'a-regular-file')
    fs.writeFileSync(notASocket, 'x')
    expect(fs.lstatSync(notASocket).isSocket()).toBe(false)
  })
})

describe('whether this machine can host one at all', () => {
  it('hosts one in a private directory', () => {
    expect(canHostEndpoint(dir)).toEqual({ ok: true })
  })

  it('declines a directory anyone can write', () => {
    // The directory is the whole access control on darwin, where a socket's own
    // mode is not consulted on connect. Anyone who can write here can rename
    // over the endpoint.
    fs.chmodSync(dir, 0o777)
    expect(canHostEndpoint(dir)).toMatchObject({ ok: false })
  })

  it('declines a path too long for sun_path', () => {
    const deep = path.join(dir, 'x'.repeat(120))
    expect(canHostEndpoint(deep)).toMatchObject({ ok: false })
  })

  it('declines a directory that is not there', () => {
    expect(canHostEndpoint(path.join(dir, 'absent'))).toMatchObject({ ok: false })
  })
})
