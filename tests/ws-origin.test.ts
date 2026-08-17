import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import os from 'node:os'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import {
  isAllowedUpgrade,
  setTrustedOriginHosts,
  resetTrustedOriginHosts
} from '../packages/server/src/ws-origin'

/**
 * The upgrade gate for browsers.
 *
 * Browsers apply neither CORS nor same-origin policy to a WebSocket upgrade and
 * will happily connect a page on any origin to localhost, so this is what stands
 * between a visited website and a shell. Every case below is either a browser
 * behaviour that must keep working, or a way a header can be shaped to look like
 * one.
 */

const PORT = 4400

beforeEach(() => {
  resetTrustedOriginHosts()
})

afterEach(() => {
  resetTrustedOriginHosts()
})

describe('a non-browser client', () => {
  it('is allowed through without an Origin, and held to the credential instead', () => {
    // The desktop bridge and MCP send none. Safe to allow because a browser
    // *cannot* omit it on an upgrade — the spec makes it mandatory.
    expect(isAllowedUpgrade(undefined, `127.0.0.1:${PORT}`)).toBe(true)
  })
})

describe('same-origin', () => {
  it.each([
    ['loopback', `127.0.0.1:${PORT}`],
    ['localhost', `localhost:${PORT}`],
    ['IPv6 loopback', `[::1]:${PORT}`],
    ['a LAN address', `192.168.0.4:${PORT}`],
    ['a tailnet address', `100.64.0.1:${PORT}`]
  ])('allows %s dialled by its own name', (_label, host) => {
    expect(isAllowedUpgrade(`http://${host}`, host)).toBe(true)
  })

  it('allows https, since a proxy may terminate TLS in front', () => {
    expect(isAllowedUpgrade(`https://192.168.0.4:${PORT}`, `192.168.0.4:${PORT}`)).toBe(true)
  })

  it('allows a port the server does not know about, which is how the dev proxy works', () => {
    // The Vite dev server serves the page on 5173 and proxies /ws onward without
    // rewriting Host. Comparing against the bound port instead would refuse it.
    expect(isAllowedUpgrade('http://localhost:5173', 'localhost:5173')).toBe(true)
  })

  it('allows an omitted default port on both sides', () => {
    expect(isAllowedUpgrade('http://192.168.0.4', '192.168.0.4')).toBe(true)
  })

  it('refuses an Origin that does not match the Host that was dialled', () => {
    // The drive-by: a page on some website opening a socket to this machine.
    expect(isAllowedUpgrade('https://evil.example', `127.0.0.1:${PORT}`)).toBe(false)
  })

  it('refuses the right host on the wrong port', () => {
    expect(isAllowedUpgrade(`http://127.0.0.1:9999`, `127.0.0.1:${PORT}`)).toBe(false)
  })
})

describe('a name the attacker chose', () => {
  /**
   * The one hole same-origin cannot see: both headers come from the attacker's own
   * URL, so of course they agree. With DNS rebinding the same trick reaches
   * loopback, needing only the port — which is discoverable. Hence the rule that
   * an origin must be unrebindable, not merely self-consistent.
   */
  it('is refused even though Origin matches Host exactly', () => {
    expect(
      isAllowedUpgrade(`http://vorn.attacker.test:${PORT}`, `vorn.attacker.test:${PORT}`)
    ).toBe(false)
  })

  it('is refused when it resolves to loopback, which is the rebinding case', () => {
    expect(isAllowedUpgrade(`http://rebind.test:${PORT}`, `rebind.test:${PORT}`)).toBe(false)
  })

  it('is allowed once the operator vouches for it', () => {
    setTrustedOriginHosts(['box.tail1234.ts.net'])
    expect(
      isAllowedUpgrade(`http://box.tail1234.ts.net:${PORT}`, `box.tail1234.ts.net:${PORT}`)
    ).toBe(true)
  })

  it('does not vouch for a neighbour of a trusted name', () => {
    setTrustedOriginHosts(['box.tail1234.ts.net'])
    expect(
      isAllowedUpgrade(
        `http://evil.box.tail1234.ts.net:${PORT}`,
        `evil.box.tail1234.ts.net:${PORT}`
      )
    ).toBe(false)
    expect(
      isAllowedUpgrade(
        `http://box.tail1234.ts.net.evil.test:${PORT}`,
        `box.tail1234.ts.net.evil.test:${PORT}`
      )
    ).toBe(false)
  })

  it('tolerates a trailing dot, which is legal in a name and absent from a list', () => {
    setTrustedOriginHosts(['box.tail1234.ts.net'])
    expect(
      isAllowedUpgrade(`http://box.tail1234.ts.net.:${PORT}`, `box.tail1234.ts.net.:${PORT}`)
    ).toBe(true)
  })

  it('trusts this machine’s own hostname without being told', () => {
    setTrustedOriginHosts([])
    const own = os.hostname().toLowerCase()
    expect(isAllowedUpgrade(`http://${own}:${PORT}`, `${own}:${PORT}`)).toBe(true)
    expect(isAllowedUpgrade(`http://${own}.local:${PORT}`, `${own}.local:${PORT}`)).toBe(true)
  })

  it('refuses a trusted name once it is no longer trusted', () => {
    setTrustedOriginHosts(['box.tail1234.ts.net'])
    setTrustedOriginHosts([])
    expect(
      isAllowedUpgrade(`http://box.tail1234.ts.net:${PORT}`, `box.tail1234.ts.net:${PORT}`)
    ).toBe(false)
  })
})

describe('headers shaped to look like a browser', () => {
  it('refuses Origin: null, which arrives as a string and not as absent', () => {
    // A sandboxed iframe, a file:// page or a cross-origin redirect all produce
    // this. Treating an unparseable value as "no origin" would let it straight in.
    expect(isAllowedUpgrade('null', `127.0.0.1:${PORT}`)).toBe(false)
  })

  it('refuses userinfo smuggled in front of the host', () => {
    // `new URL('http://evil.example@127.0.0.1:4400').host` is `127.0.0.1:4400`, so
    // a bare host comparison would say yes to this.
    expect(isAllowedUpgrade(`http://evil.example@127.0.0.1:${PORT}`, `127.0.0.1:${PORT}`)).toBe(
      false
    )
  })

  it('refuses duplicate Origin headers, which arrive joined rather than as a list', () => {
    // `origin` is not de-duplicated by Node the way `host` is, so two headers
    // become "a, b". This must never be split on commas.
    expect(
      isAllowedUpgrade(`http://127.0.0.1:${PORT}, http://evil.example`, `127.0.0.1:${PORT}`)
    ).toBe(false)
  })

  it.each([
    ['a trailing slash', `http://127.0.0.1:${PORT}/`],
    ['a path', `http://127.0.0.1:${PORT}/app`],
    ['an uppercase scheme', `HTTP://127.0.0.1:${PORT}`],
    ['a non-http scheme', `ftp://127.0.0.1:${PORT}`],
    ['a file origin', 'file://'],
    ['empty', '']
  ])('refuses %s', (_label, origin) => {
    expect(isAllowedUpgrade(origin, `127.0.0.1:${PORT}`)).toBe(false)
  })

  it('refuses an uppercase host that a browser would have lowercased', () => {
    // Node lowercases header names, never values, so only a non-browser can send
    // this — and both sides go through the same parser so it cannot pass by
    // normalising one of them.
    expect(isAllowedUpgrade(`http://127.0.0.1:${PORT}`, `LOCALHOST:${PORT}`)).toBe(false)
  })

  it('refuses when the Host header is missing or unusable', () => {
    expect(isAllowedUpgrade(`http://127.0.0.1:${PORT}`, undefined)).toBe(false)
    expect(isAllowedUpgrade(`http://127.0.0.1:${PORT}`, 'not a host: :: ::')).toBe(false)
  })

  it('refuses a spelled-out default port, which a browser never sends', () => {
    expect(isAllowedUpgrade('http://192.168.0.4:80', '192.168.0.4:80')).toBe(false)
  })
})

describe('IPv6 and IPv4 canonical forms', () => {
  it('matches a bracketed IPv6 origin against a bracketed Host', () => {
    expect(isAllowedUpgrade(`http://[fd00::1]:${PORT}`, `[fd00::1]:${PORT}`)).toBe(true)
  })

  it('refuses a non-canonical IPv6 spelling, which a browser would have collapsed', () => {
    expect(isAllowedUpgrade(`http://[0:0:0:0:0:0:0:1]:${PORT}`, `[0:0:0:0:0:0:0:1]:${PORT}`)).toBe(
      false
    )
  })

  it('refuses an IPv4 shorthand, for the same reason', () => {
    expect(isAllowedUpgrade(`http://127.1:${PORT}`, `127.1:${PORT}`)).toBe(false)
  })

  it('still allows the canonical form the parser produces from those', () => {
    expect(isAllowedUpgrade(`http://127.0.0.1:${PORT}`, `127.0.0.1:${PORT}`)).toBe(true)
  })
})
