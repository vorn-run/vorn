// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, waitFor, act } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import '@testing-library/jest-dom/vitest'
import type { TailscaleStatus, ReachableUrls } from '../packages/shared/src/types'

// ─── Store ───────────────────────────────────────────────────────

const store = {
  config: { defaults: { networkAccessEnabled: false } },
  setConfig: vi.fn()
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) => (selector ? selector(store) : store)
}))

// The QR library draws to a canvas jsdom does not implement, and what it renders is
// not what this file is about.
vi.mock('qrcode', () => ({ default: { toDataURL: vi.fn().mockResolvedValue('data:,') } }))

import { NetworkSettings } from '../src/renderer/components/settings/NetworkSettings'

// ─── Helpers ─────────────────────────────────────────────────────

function tailscale(overrides: Partial<TailscaleStatus> = {}): TailscaleStatus {
  return {
    installed: false,
    running: false,
    backendState: 'NoState',
    selfIP: '',
    selfDNSName: '',
    peers: [],
    ...overrides
  } as TailscaleStatus
}

function reachable(overrides: Partial<ReachableUrls> = {}): ReachableUrls {
  return { urls: ['http://192.168.1.20:4000/app/'], port: 4000, remote: true, ...overrides }
}

const getConnectSettings = vi.fn()
const saveConnectSettings = vi.fn()
const useLocalServer = vi.fn()
const listDeviceTokens = vi.fn()
const createDeviceToken = vi.fn()
const revokeDeviceToken = vi.fn()
const getTailscaleStatus = vi.fn()
const getReachableUrls = vi.fn()
const saveConfig = vi.fn()

/**
 * Render and wait for the panel to settle.
 *
 * Three calls resolve on mount and all of them gate what is on screen, so waiting on
 * the last one to be *called* is not enough — the mode switch only appears once
 * `getConnectSettings` has answered.
 */
async function renderPanel() {
  const result = render(<NetworkSettings />)
  await waitFor(() => expect(getReachableUrls).toHaveBeenCalled())
  await act(async () => {})
  return result
}

/** Move to the other half of the panel, which is a radio rather than a switch. */
async function switchTo(user: ReturnType<typeof userEvent.setup>, label: string) {
  await user.click(screen.getByRole('radio', { name: label }))
}

beforeEach(() => {
  vi.clearAllMocks()
  vi.useRealTimers()
  store.config = { defaults: { networkAccessEnabled: false } }
  getTailscaleStatus.mockResolvedValue(tailscale())
  getReachableUrls.mockResolvedValue(reachable())
  listDeviceTokens.mockResolvedValue([])
  createDeviceToken.mockResolvedValue({
    token: { id: 'tok1', name: 'My phone', createdAt: '', lastSeenAt: null, revokedAt: null },
    plaintext: 'vorn_tok1_secret'
  })
  revokeDeviceToken.mockResolvedValue({ revoked: true })
  getConnectSettings.mockResolvedValue({ mode: 'local', url: '', hasToken: false })
  saveConnectSettings.mockResolvedValue({ ok: true })
  useLocalServer.mockResolvedValue({ ok: true })
  ;(window as unknown as { api: unknown }).api = {
    getTailscaleStatus,
    getReachableUrls,
    saveConfig,
    listDeviceTokens,
    createDeviceToken,
    revokeDeviceToken,
    getConnectSettings,
    saveConnectSettings,
    useLocalServer
  }
})

// ─── Tests ───────────────────────────────────────────────────────

describe('the two directions remote access can point', () => {
  it('asks which one before showing either', async () => {
    // The panel used to stack both at once, which was not merely busy: host mode
    // never starts a local server, so the sharing controls described something that
    // was not running.
    await renderPanel()

    expect(screen.getByRole('radio', { name: 'This machine' })).toBeInTheDocument()
    expect(screen.getByRole('radio', { name: 'Another machine' })).toBeInTheDocument()
  })

  it('shows sharing controls and no host form by default', async () => {
    await renderPanel()

    expect(screen.getByText('Share this machine')).toBeInTheDocument()
    expect(screen.queryByLabelText('Server address')).not.toBeInTheDocument()
  })

  it('shows the host form and no sharing controls on the other side', async () => {
    const user = userEvent.setup()
    await renderPanel()

    await switchTo(user, 'Another machine')

    expect(screen.getByLabelText('Server address')).toBeInTheDocument()
    expect(screen.queryByText('Share this machine')).not.toBeInTheDocument()
    expect(screen.queryByRole('switch')).not.toBeInTheDocument()
  })

  it('opens on the side the app is actually using', async () => {
    // Someone already connected to a host should not have to find the switch to see
    // where they are connected.
    getConnectSettings.mockResolvedValue({
      mode: 'host',
      url: 'ws://box:61601/ws',
      hasToken: true
    })

    await renderPanel()

    expect(screen.getByText('Connected')).toBeInTheDocument()
  })

  it('offers no switch in the browser, which cannot point anywhere else', async () => {
    // `getConnectSettings` answers null in the web build: a browser reached its
    // server by address and has nothing to repoint. Showing a half that cannot work
    // there would be worse than hiding it.
    getConnectSettings.mockResolvedValue(null)

    await renderPanel()

    expect(screen.queryByRole('radio')).not.toBeInTheDocument()
    expect(screen.getByText('Share this machine')).toBeInTheDocument()
  })
})

describe('sharing this machine', () => {
  it('offers the toggle with Tailscale absent', async () => {
    // It used to render only when Tailscale was running, so there was no way to turn
    // remote access on without it.
    await renderPanel()

    expect(screen.getByText('Share this machine')).toBeInTheDocument()
    expect(screen.getByRole('switch')).toBeInTheDocument()
  })

  it('shows an address with Tailscale absent', async () => {
    // The old panel read `appUrl`, which is only computed when Tailscale runs, so it
    // reported the feature as on and never said where to connect.
    store.config = { defaults: { networkAccessEnabled: true } }

    await renderPanel()

    expect(screen.getByText('http://192.168.1.20:4000/app/')).toBeInTheDocument()
  })

  it('shows no address while remote access is off', async () => {
    await renderPanel()

    expect(screen.queryByText('http://192.168.1.20:4000/app/')).not.toBeInTheDocument()
  })

  it('leads with one address and folds the rest away', async () => {
    // A machine answers on several, but they are not equally useful: the server
    // returns them tailnet first, and that one is encrypted. Three identical boxes
    // made the reader choose before anything had told them how to.
    store.config = { defaults: { networkAccessEnabled: true } }
    getReachableUrls.mockResolvedValue(
      reachable({ urls: ['http://100.1.2.3:4000/app/', 'http://192.168.1.20:4000/app/'] })
    )

    await renderPanel()

    expect(screen.getByText('http://100.1.2.3:4000/app/')).toBeInTheDocument()
    expect(screen.queryByText('http://192.168.1.20:4000/app/')).not.toBeInTheDocument()
  })

  it('still gives every address to whoever asks', async () => {
    // Only the person looking at the screen knows which network the other device is
    // on, so the others stay reachable rather than being dropped.
    const user = userEvent.setup()
    store.config = { defaults: { networkAccessEnabled: true } }
    getReachableUrls.mockResolvedValue(
      reachable({ urls: ['http://100.1.2.3:4000/app/', 'http://192.168.1.20:4000/app/'] })
    )
    await renderPanel()

    await user.click(screen.getByText('1 other address'))

    expect(screen.getByText('http://192.168.1.20:4000/app/')).toBeInTheDocument()
  })

  it('says the connection is encrypted on a tailnet', async () => {
    store.config = { defaults: { networkAccessEnabled: true } }
    getTailscaleStatus.mockResolvedValue(
      tailscale({ installed: true, running: true, selfIP: '100.1.2.3' })
    )

    await renderPanel()

    expect(screen.getByText(/Encrypted via Tailscale/)).toBeInTheDocument()
  })

  it('says it is not, and offers Tailscale, when there is no tailnet', async () => {
    // The card that used to say this at length is gone; the qualifier under the
    // address is the honest difference between the two cases.
    store.config = { defaults: { networkAccessEnabled: true } }

    await renderPanel()

    expect(screen.getByText(/This network only, unencrypted/)).toBeInTheDocument()
    expect(screen.getByText('Add Tailscale')).toBeInTheDocument()
  })

  it('manages devices in the app rather than sending you to a terminal', async () => {
    store.config = { defaults: { networkAccessEnabled: true } }

    await renderPanel()

    expect(screen.getByText('Add device')).toBeInTheDocument()
    expect(screen.queryByText(/vorn-server token create/)).not.toBeInTheDocument()
  })

  it('asks before enabling without a tailnet, and does not save until confirmed', async () => {
    const user = userEvent.setup()
    await renderPanel()

    await user.click(screen.getByRole('switch'))

    expect(screen.getByText('Enable without Tailscale?')).toBeInTheDocument()
    // Naming the cost is the point: unencrypted, and the token runs commands.
    expect(screen.getByText(/travels unencrypted/)).toBeInTheDocument()
    expect(saveConfig).not.toHaveBeenCalled()
  })

  it('saves once the acknowledgement is accepted', async () => {
    const user = userEvent.setup()
    await renderPanel()

    await user.click(screen.getByRole('switch'))
    await user.click(screen.getByText('Enable anyway'))

    expect(saveConfig).toHaveBeenCalledWith(
      expect.objectContaining({ defaults: expect.objectContaining({ networkAccessEnabled: true }) })
    )
  })

  it('leaves the setting alone when the acknowledgement is cancelled', async () => {
    const user = userEvent.setup()
    await renderPanel()

    await user.click(screen.getByRole('switch'))
    await user.click(screen.getByText('Cancel'))

    expect(screen.queryByText('Enable without Tailscale?')).not.toBeInTheDocument()
    expect(saveConfig).not.toHaveBeenCalled()
  })

  it('enables straight away on a tailnet, where the traffic is encrypted', async () => {
    const user = userEvent.setup()
    getTailscaleStatus.mockResolvedValue(
      tailscale({ installed: true, running: true, selfIP: '100.1.2.3' })
    )
    await renderPanel()

    await user.click(screen.getByRole('switch'))

    expect(screen.queryByText('Enable without Tailscale?')).not.toBeInTheDocument()
    expect(saveConfig).toHaveBeenCalled()
  })

  it('turns off without asking', async () => {
    const user = userEvent.setup()
    store.config = { defaults: { networkAccessEnabled: true } }
    await renderPanel()

    await user.click(screen.getByRole('switch'))

    expect(screen.queryByText('Enable without Tailscale?')).not.toBeInTheDocument()
    expect(saveConfig).toHaveBeenCalledWith(
      expect.objectContaining({
        defaults: expect.objectContaining({ networkAccessEnabled: false })
      })
    )
  })

  it('re-reads the addresses after the setting changes', async () => {
    // The server rebinds on the config change, so the addresses it reports change too.
    vi.useFakeTimers({ shouldAdvanceTime: true })
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime })
    await renderPanel()
    getReachableUrls.mockClear()

    await user.click(screen.getByRole('switch'))
    await user.click(screen.getByText('Enable anyway'))
    await act(async () => {
      vi.advanceTimersByTime(600)
    })

    await waitFor(() => expect(getReachableUrls).toHaveBeenCalled())
  })

  it('drops the addresses when the status calls fail', async () => {
    // Keeping the previous ones would leave the panel advertising a URL while it
    // cannot reach the server to confirm it, which reads as "still reachable" at
    // exactly the moment it is not.
    store.config = { defaults: { networkAccessEnabled: true } }
    getTailscaleStatus.mockRejectedValue(new Error('server unreachable'))
    getReachableUrls.mockRejectedValue(new Error('server unreachable'))
    vi.spyOn(console, 'error').mockImplementation(() => {})

    await renderPanel()

    await waitFor(() => expect(screen.getByText('Share this machine')).toBeInTheDocument())
    expect(screen.queryByText('http://192.168.1.20:4000/app/')).not.toBeInTheDocument()
  })

  it('renders nothing before the config has loaded', async () => {
    store.config = null as unknown as typeof store.config

    const { container } = render(<NetworkSettings />)

    expect(container).toBeEmptyDOMElement()
  })
})

describe('the device list', () => {
  beforeEach(() => {
    store.config = { defaults: { networkAccessEnabled: true } }
  })

  it('lists the devices allowed to connect, with when each was last seen', async () => {
    listDeviceTokens.mockResolvedValue([
      {
        id: 'a',
        name: 'My phone',
        createdAt: '',
        lastSeenAt: '2026-08-17T10:00:00Z',
        revokedAt: null
      },
      { id: 'b', name: 'Work laptop', createdAt: '', lastSeenAt: null, revokedAt: null }
    ])

    await renderPanel()

    await waitFor(() => expect(screen.getByText('My phone')).toBeInTheDocument())
    expect(screen.getByText('Work laptop')).toBeInTheDocument()
    expect(screen.getByText('Never connected')).toBeInTheDocument()
  })

  it('hides a revoked device instead of listing it as usable', async () => {
    listDeviceTokens.mockResolvedValue([
      {
        id: 'a',
        name: 'Lost phone',
        createdAt: '',
        lastSeenAt: null,
        revokedAt: '2026-08-17T10:00:00Z'
      }
    ])

    await renderPanel()

    await waitFor(() => expect(screen.getByText('Add device')).toBeInTheDocument())
    expect(screen.queryByText('Lost phone')).not.toBeInTheDocument()
  })

  it('shows the token once, and says that is the only time', async () => {
    // It is stored as a hash, so there is no second chance to read it.
    const user = userEvent.setup()
    await renderPanel()

    await user.click(screen.getByText('Add device'))
    await user.type(screen.getByLabelText('What is this device?'), 'My phone')
    await user.click(screen.getByText('Create token'))

    await waitFor(() => expect(screen.getByText('vorn_tok1_secret')).toBeInTheDocument())
    expect(screen.getByText(/only time it can be shown/)).toBeInTheDocument()
    expect(createDeviceToken).toHaveBeenCalledWith('My phone')
  })

  it('asks before revoking, since a revoked device stops working at once', async () => {
    const user = userEvent.setup()
    listDeviceTokens.mockResolvedValue([
      { id: 'a', name: 'My phone', createdAt: '', lastSeenAt: null, revokedAt: null }
    ])
    await renderPanel()
    await waitFor(() => expect(screen.getByText('My phone')).toBeInTheDocument())

    await user.click(screen.getByText('Remove'))
    expect(revokeDeviceToken).not.toHaveBeenCalled()

    await user.click(screen.getByText('Revoke'))
    expect(revokeDeviceToken).toHaveBeenCalledWith('a')
  })

  it('says so when the list cannot be read, rather than looking empty', async () => {
    listDeviceTokens.mockRejectedValue(new Error('server unreachable'))
    vi.spyOn(console, 'error').mockImplementation(() => {})

    await renderPanel()

    await waitFor(() =>
      expect(screen.getByText('Could not read the device list.')).toBeInTheDocument()
    )
  })
})

describe('using another machine', () => {
  it('takes an address and a token together', async () => {
    const user = userEvent.setup()
    await renderPanel()
    await switchTo(user, 'Another machine')

    await user.type(screen.getByLabelText('Server address'), '192.168.0.4:61601')
    await user.type(screen.getByLabelText('Device token from that machine'), 'vorn_a_b')
    await user.click(screen.getByText('Connect and restart'))

    expect(saveConnectSettings).toHaveBeenCalledWith({
      url: '192.168.0.4:61601',
      token: 'vorn_a_b'
    })
  })

  it('says the app restarts and what still needs a desktop', async () => {
    // Workflow execution lives in the renderer, so a host with nothing attached holds
    // state and runs terminals but fires no schedules. Better said here than
    // discovered when a scheduled workflow silently does not run.
    const user = userEvent.setup()
    await renderPanel()

    await switchTo(user, 'Another machine')

    expect(screen.getByText(/Vorn restarts to apply this/)).toBeInTheDocument()
    expect(screen.getByText(/fires no schedules/)).toBeInTheDocument()
  })

  it('reports a rejected address instead of appearing to succeed', async () => {
    const user = userEvent.setup()
    saveConnectSettings.mockResolvedValue({ ok: false, error: 'Both an address and a token.' })
    await renderPanel()
    await switchTo(user, 'Another machine')

    await user.click(screen.getByText('Connect and restart'))

    await waitFor(() =>
      expect(screen.getByText('Both an address and a token.')).toBeInTheDocument()
    )
  })

  it('shows which host it is on, and the way back', async () => {
    const user = userEvent.setup()
    getConnectSettings.mockResolvedValue({
      mode: 'host',
      url: 'ws://box:61601/ws',
      hasToken: true
    })
    await renderPanel()

    expect(screen.getByText('Connected')).toBeInTheDocument()
    expect(screen.getByText('ws://box:61601/ws')).toBeInTheDocument()

    await user.click(screen.getByText('Disconnect'))
    expect(useLocalServer).toHaveBeenCalled()
  })

  it('lets a connected host be changed without disconnecting first', async () => {
    const user = userEvent.setup()
    getConnectSettings.mockResolvedValue({
      mode: 'host',
      url: 'ws://box:61601/ws',
      hasToken: true
    })
    await renderPanel()

    await user.click(screen.getByText('Change'))

    expect(screen.getByLabelText('Server address')).toHaveValue('ws://box:61601/ws')
  })
})
