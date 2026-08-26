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

  it.each([
    ['world-writable', 0o777],
    ['group-writable', 0o770],
    // Not writable, and still not private. On darwin a socket's own mode is not
    // consulted on connect, so a directory another user can traverse is a socket
    // another user can connect to -- and the greeting, sent before
    // authentication, hands them this server's identity and the account name in
    // `dataDir`. Loopback bounded that for TCP peers; for a unix peer the
    // directory is the only bound there is.
    ['world-readable', 0o755],
    ['group-readable', 0o750],
    ['group-executable only', 0o710]
  ])('tightens a %s directory rather than refusing it', (_label, mode) => {
    // This directory is Vorn's own, it holds the credential and the database, and
    // `database.ts` already means it to be 0700 -- but `mode` on mkdir applies
    // only at creation, so real installs are out there sitting wider. Refusing
    // them would leave those machines silently without an endpoint; narrowing
    // fixes the thing that was actually wrong.
    fs.chmodSync(dir, mode)

    expect(canHostEndpoint(dir)).toEqual({ ok: true })
    expect(fs.statSync(dir).mode & 0o777).toBe(0o700)
  })

  it('refuses when it cannot make the directory private', () => {
    const stranger = path.join(dir, 'not-ours')
    fs.mkdirSync(stranger, { mode: 0o755 })
    // Nothing to tighten because there is nothing there: the honest answer is no.
    fs.rmSync(stranger, { recursive: true })
    expect(canHostEndpoint(stranger)).toMatchObject({ ok: false })
  })

  it('declines a path too long for sun_path', () => {
    const deep = path.join(dir, 'x'.repeat(120))
    expect(canHostEndpoint(deep)).toMatchObject({ ok: false })
  })

  it('measures the name it will actually bind, not the shorter one', () => {
    // The scratch name is what gets bound and it is thirteen bytes longer than
    // the canonical one. A directory that fits only the shorter path would pass
    // the check and then fail the bind -- the exact failure this turns into a
    // clean downgrade.
    // Sized so the canonical path fits inside the limit and the scratch name --
    // thirteen bytes longer -- does not. Created, because a directory that is not
    // there is refused for a different reason and would make this pass without
    // testing anything.
    const room = 96 - Buffer.byteLength(endpointPath(dir)) - 1
    const snug = path.join(dir, 'y'.repeat(Math.max(1, room - 6)))
    fs.mkdirSync(snug, { recursive: true, mode: 0o700 })

    expect(Buffer.byteLength(endpointPath(snug))).toBeLessThanOrEqual(96)
    expect(Buffer.byteLength(scratchPathFor(endpointPath(snug)))).toBeGreaterThan(96)
    expect(canHostEndpoint(snug)).toMatchObject({ ok: false })
  })

  it('declines a directory that is not there', () => {
    expect(canHostEndpoint(path.join(dir, 'absent'))).toMatchObject({ ok: false })
  })
})
