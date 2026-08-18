import { describe, it, expect, vi, beforeEach } from 'vitest'

const mockNetworkInterfaces = vi.fn()
const mockGetCurrentHost = vi.fn()

vi.mock('node:os', () => ({
  default: { networkInterfaces: (...a: unknown[]) => mockNetworkInterfaces(...a) },
  networkInterfaces: (...a: unknown[]) => mockNetworkInterfaces(...a)
}))
vi.mock('../packages/server/src/server-rebind', () => ({
  getCurrentHost: () => mockGetCurrentHost()
}))

import { reachableUrls } from '../packages/server/src/reachable-urls'

/** Shape of one entry as `os.networkInterfaces()` returns it. */
function iface(address: string, family: 'IPv4' | 'IPv6' = 'IPv4', internal = false) {
  return { address, family, internal, netmask: '', mac: '', cidr: null }
}

describe('reachableUrls', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockGetCurrentHost.mockReturnValue('0.0.0.0')
    mockNetworkInterfaces.mockReturnValue({ en0: [iface('192.168.1.20')] })
  })

  it('reports no address while bound to loopback', () => {
    // Nothing else can reach it, so offering a LAN address would be a lie the user
    // would then try to open on their phone.
    mockGetCurrentHost.mockReturnValue('127.0.0.1')

    const result = reachableUrls(4000, ['100.1.2.3'])

    expect(result).toEqual({ urls: [], port: 4000, remote: false })
  })

  it('builds a URL for each LAN address', () => {
    mockNetworkInterfaces.mockReturnValue({
      en0: [iface('192.168.1.20')],
      en1: [iface('10.0.0.5')]
    })

    expect(reachableUrls(4000).urls).toEqual([
      'http://192.168.1.20:4000/app/',
      'http://10.0.0.5:4000/app/'
    ])
  })

  it('puts the Tailscale address first', () => {
    // It is the only one of these that is encrypted, and the QR code takes the first.
    expect(reachableUrls(4000, ['100.1.2.3']).urls).toEqual([
      'http://100.1.2.3:4000/app/',
      'http://192.168.1.20:4000/app/'
    ])
  })

  it('works with no Tailscale address at all', () => {
    // The case the old UI could not express: it returned nothing without a tailnet.
    const result = reachableUrls(4000, [])

    expect(result.urls).toEqual(['http://192.168.1.20:4000/app/'])
    expect(result.remote).toBe(true)
  })

  it('skips loopback and other internal interfaces', () => {
    mockNetworkInterfaces.mockReturnValue({
      lo0: [iface('127.0.0.1', 'IPv4', true)],
      en0: [iface('192.168.1.20')]
    })

    expect(reachableUrls(4000).urls).toEqual(['http://192.168.1.20:4000/app/'])
  })

  it('skips IPv6, which cannot be offered as a plain URL', () => {
    // Link-local needs a zone id (`%en0`) that no URL can carry, so a link built
    // from one cannot be opened.
    mockNetworkInterfaces.mockReturnValue({
      en0: [iface('fe80::1', 'IPv6'), iface('192.168.1.20')]
    })

    expect(reachableUrls(4000).urls).toEqual(['http://192.168.1.20:4000/app/'])
  })

  it('skips the 169.254 block, which means DHCP failed', () => {
    mockNetworkInterfaces.mockReturnValue({
      en0: [iface('169.254.3.4'), iface('192.168.1.20')]
    })

    expect(reachableUrls(4000).urls).toEqual(['http://192.168.1.20:4000/app/'])
  })

  it('does not repeat an address Tailscale and the interface list both report', () => {
    mockNetworkInterfaces.mockReturnValue({
      utun3: [iface('100.1.2.3')],
      en0: [iface('192.168.1.20')]
    })

    expect(reachableUrls(4000, ['100.1.2.3']).urls).toEqual([
      'http://100.1.2.3:4000/app/',
      'http://192.168.1.20:4000/app/'
    ])
  })

  it('ignores an empty Tailscale address', () => {
    // `selfIP` is the empty string when Tailscale is installed but not connected.
    expect(reachableUrls(4000, ['']).urls).toEqual(['http://192.168.1.20:4000/app/'])
  })

  it('tolerates an interface with no addresses', () => {
    mockNetworkInterfaces.mockReturnValue({ awdl0: undefined, en0: [iface('192.168.1.20')] })

    expect(reachableUrls(4000).urls).toEqual(['http://192.168.1.20:4000/app/'])
  })

  it('returns an empty list rather than throwing when nothing is up', () => {
    mockNetworkInterfaces.mockReturnValue({})

    expect(reachableUrls(4000)).toEqual({ urls: [], port: 4000, remote: true })
  })

  it('carries the port through', () => {
    expect(reachableUrls(59898).urls).toEqual(['http://192.168.1.20:59898/app/'])
    expect(reachableUrls(59898).port).toBe(59898)
  })
})
