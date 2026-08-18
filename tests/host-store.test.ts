import { describe, it, expect, vi, beforeEach } from 'vitest'

/**
 * Which server this desktop talks to, remembered locally.
 *
 * The first thing Vorn stores outside the server's database, and it has to be:
 * host mode must know which database to open before it has one.
 */

const files = new Map<string, string>()
const encryptionAvailable = { value: true }

vi.mock('node:fs', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return {
    ...actual,
    default: {
      ...(actual.default as Record<string, unknown>),
      readFileSync: (p: string) => {
        const found = files.get(p)
        if (found === undefined) throw new Error('ENOENT')
        return found
      },
      writeFileSync: (p: string, data: string) => files.set(p, data),
      rmSync: (p: string) => files.delete(p)
    }
  }
})

vi.mock('electron', () => ({
  app: { getPath: () => '/userData' },
  safeStorage: {
    isEncryptionAvailable: () => encryptionAvailable.value,
    // Reversible stand-in for the OS keychain: the point under test is that the
    // plaintext never reaches the file, not that AES works.
    encryptString: (s: string) => Buffer.from(`sealed:${s}`),
    decryptString: (b: Buffer) => {
      const raw = b.toString()
      if (!raw.startsWith('sealed:')) throw new Error('not decryptable on this machine')
      return raw.slice('sealed:'.length)
    }
  }
}))
vi.mock('../src/main/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import {
  readHostSettings,
  writeHostSettings,
  clearHostSettings
} from '../src/main/server/host-store'
import { normaliseHostUrl } from '../src/main/server/connect-window'

beforeEach(() => {
  files.clear()
  encryptionAvailable.value = true
})

describe('reading host settings', () => {
  it('runs a local server when nothing has been stored', () => {
    expect(readHostSettings()).toEqual({ mode: 'local', url: '' })
  })

  it('round-trips a host and its token', () => {
    writeHostSettings({ mode: 'host', url: 'ws://box:61601/ws', token: 'vorn_a_b' })

    expect(readHostSettings()).toEqual({
      mode: 'host',
      url: 'ws://box:61601/ws',
      token: 'vorn_a_b'
    })
  })

  it('never writes the token in the clear', () => {
    // It is equivalent to a shell on the host it points at.
    writeHostSettings({ mode: 'host', url: 'ws://box:61601/ws', token: 'vorn_secret_value' })

    expect(files.get('/userData/host.json')).not.toContain('vorn_secret_value')
  })

  it('refuses to store a token when the OS cannot encrypt it', () => {
    // A Linux box with no keyring. Asking again each launch is worse to use and
    // better than leaving a shell credential in a plaintext file.
    encryptionAvailable.value = false
    writeHostSettings({ mode: 'host', url: 'ws://box:61601/ws', token: 'vorn_secret_value' })

    const stored = files.get('/userData/host.json') ?? ''
    expect(stored).not.toContain('vorn_secret_value')
    expect(readHostSettings().token).toBeUndefined()
    // The address is still remembered, so only the credential has to be re-entered.
    expect(readHostSettings().url).toBe('ws://box:61601/ws')
  })

  it('keeps the host but drops a token it cannot decrypt', () => {
    // A keychain that moved, or a file copied between machines.
    files.set(
      '/userData/host.json',
      JSON.stringify({ mode: 'host', url: 'ws://box:61601/ws', encryptedToken: 'Z2FyYmFnZQ==' })
    )

    const settings = readHostSettings()
    expect(settings.mode).toBe('host')
    expect(settings.url).toBe('ws://box:61601/ws')
    expect(settings.token).toBeUndefined()
  })

  it('falls back to local rather than failing to start on a corrupt file', () => {
    files.set('/userData/host.json', '{ not json')

    expect(readHostSettings()).toEqual({ mode: 'local', url: '' })
  })

  it('falls back to local when host mode has no address', () => {
    files.set('/userData/host.json', JSON.stringify({ mode: 'host', url: '' }))

    expect(readHostSettings().mode).toBe('local')
  })

  it('returns to a local server when cleared', () => {
    writeHostSettings({ mode: 'host', url: 'ws://box:61601/ws', token: 'vorn_a_b' })

    clearHostSettings()

    expect(readHostSettings()).toEqual({ mode: 'local', url: '' })
  })
})

describe('normalising what someone pastes', () => {
  // The address people have is the one Settings shows them, which is the web
  // client's. Converting it by hand is a step that exists only because of an
  // implementation detail.
  it.each([
    ['192.168.0.4:61601', 'ws://192.168.0.4:61601/ws'],
    ['http://192.168.0.4:61601/app/', 'ws://192.168.0.4:61601/ws'],
    ['http://192.168.0.4:61601', 'ws://192.168.0.4:61601/ws'],
    ['ws://192.168.0.4:61601/ws', 'ws://192.168.0.4:61601/ws'],
    ['  192.168.0.4:61601  ', 'ws://192.168.0.4:61601/ws']
  ])('%s becomes %s', (input, expected) => {
    expect(normaliseHostUrl(input)).toBe(expected)
  })

  it('keeps TLS when the address has it', () => {
    // Behind a reverse proxy, which is the sane way to expose a host publicly.
    expect(normaliseHostUrl('https://vorn.example.com')).toBe('wss://vorn.example.com/ws')
    expect(normaliseHostUrl('wss://vorn.example.com/ws')).toBe('wss://vorn.example.com/ws')
  })

  it('hands back unparseable input rather than inventing a URL', () => {
    expect(normaliseHostUrl('http://')).toBe('http://')
  })
})
