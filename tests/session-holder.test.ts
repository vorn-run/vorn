import { describe, it, expect, vi } from 'vitest'
import path from 'node:path'
import {
  endOlderHolder,
  readAnnouncement,
  readSessionHolders
} from '../src/main/server/session-holder'
import type { SessionHolders } from '@vornrun/shared/types'

const holder = {
  pid: 4242,
  instance: '1a2b',
  build: '0.8.0',
  proto: 1,
  sessions: 2,
  compatible: true
}

function answering(body: unknown, status = 200): typeof fetch {
  return (async () => new Response(JSON.stringify(body), { status })) as typeof fetch
}

describe('reading the session holders from vornd', () => {
  it('reads current, older and error from the health check, even a 503', async () => {
    const reported = await readSessionHolders(
      47001,
      answering(
        {
          status: 'down',
          sessiond: {
            current: holder,
            older: [{ ...holder, instance: 'ff', sessions: null }],
            error: null
          }
        },
        503
      )
    )
    expect(reported).toEqual({
      current: holder,
      older: [{ ...holder, instance: 'ff', sessions: null }],
      error: null
    })
  })

  it('is null for a vornd that keeps no holder, or cannot be asked', async () => {
    expect(await readSessionHolders(47001, answering({ sessiond: null }))).toBeNull()
    const refused = (async () => {
      throw new Error('ECONNREFUSED')
    }) as typeof fetch
    expect(await readSessionHolders(47001, refused)).toBeNull()
  })

  it('drops entries it cannot use', async () => {
    const reported = await readSessionHolders(
      47001,
      answering({ sessiond: { current: { pid: 'x' }, older: [holder, 7], error: 'boom' } })
    )
    expect(reported).toEqual({ current: null, older: [holder], error: 'boom' })
  })
})

const home = path.join('/Users', 'x', '.vorn')
const infoFile = path.join(home, 'run', 'sessiond-1a2b.info')
const announced =
  (pid: number, instance = '1a2b'): ((file: string) => string) =>
  (file) => {
    if (file !== infoFile) throw new Error('ENOENT')
    return `endpoint=/tmp/s.sock\npid=${pid}\nproto=1\nbuild=0.7.5\ninstance=${instance}\n`
  }

describe('reading an announcement', () => {
  it('reads the pid of the instance it names', () => {
    expect(readAnnouncement(home, '1a2b', announced(4242))).toEqual({ pid: 4242, instance: '1a2b' })
  })

  it('refuses a name that is not an instance id, and a file naming another', () => {
    expect(readAnnouncement(home, '../../etc/passwd', announced(4242))).toBeNull()
    expect(readAnnouncement(home, '1a2b', announced(4242, 'ffff'))).toBeNull()
    expect(readAnnouncement(home, 'abcd', announced(4242))).toBeNull()
  })
})

describe('ending an older session holder', () => {
  const reported: SessionHolders = {
    current: { ...holder, pid: 1, instance: '99' },
    older: [{ ...holder, build: '0.7.5' }],
    error: null
  }

  it('kills the process vornd reports as older and its announcement names', () => {
    const kill = vi.fn()
    const remove = vi.fn()
    expect(
      endOlderHolder(home, reported, '1a2b', { readFile: announced(4242), kill, remove })
    ).toEqual({
      ok: true
    })
    expect(kill).toHaveBeenCalledWith(4242)
    expect(remove).toHaveBeenCalledWith(infoFile)
  })

  it('never ends the current holder, or one vornd does not report', () => {
    const kill = vi.fn()
    expect(
      endOlderHolder(home, reported, '99', { readFile: announced(1, '99'), kill })
    ).toMatchObject({
      ok: false
    })
    expect(endOlderHolder(home, null, '1a2b', { readFile: announced(4242), kill })).toMatchObject({
      ok: false
    })
    expect(kill).not.toHaveBeenCalled()
  })

  it('leaves a pid alone once the announcement no longer names it', () => {
    const kill = vi.fn()
    expect(endOlderHolder(home, reported, '1a2b', { readFile: announced(5555), kill })).toEqual({
      ok: false,
      detail: 'that session holder is no longer running'
    })
    expect(kill).not.toHaveBeenCalled()
  })

  it('says why when the kill fails', () => {
    const kill = (): void => {
      throw new Error('EPERM')
    }
    expect(
      endOlderHolder(home, reported, '1a2b', { readFile: announced(4242), kill, remove: vi.fn() })
    ).toEqual({ ok: false, detail: 'EPERM' })
  })
})
