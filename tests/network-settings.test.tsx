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

/** Render and wait for the two status calls the panel makes on mount. */
async function renderPanel() {
  const result = render(<NetworkSettings />)
  await waitFor(() => expect(getReachableUrls).toHaveBeenCalled())
  return result
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

describe('NetworkSettings', () => {
  it('offers the toggle with Tailscale absent', async () => {
    // It used to render only when Tailscale was running, so there was no way to turn
    // remote access on without it — the gate this pass removes.
    await renderPanel()

    expect(screen.getByText('Enable Remote Access')).toBeInTheDocument()
  })

  it('shows an address with Tailscale absent', async () => {
    // The old panel read `appUrl`, which is only computed when Tailscale runs, so it
    // reported the feature as on and never said where to connect.
    store.config = { defaults: { networkAccessEnabled: true } }

    await renderPanel()

    expect(screen.getByText('http://192.168.1.20:4000/app/')).toBeInTheDocument()
  })

  it('lists every address, since only the user knows which network the phone is on', async () => {
    store.config = { defaults: { networkAccessEnabled: true } }
    getReachableUrls.mockResolvedValue(
      reachable({ urls: ['http://100.1.2.3:4000/app/', 'http://192.168.1.20:4000/app/'] })
    )

    await renderPanel()

    expect(screen.getByText('http://100.1.2.3:4000/app/')).toBeInTheDocument()
    expect(screen.getByText('http://192.168.1.20:4000/app/')).toBeInTheDocument()
  })

  it('manages devices in the app rather than sending you to a terminal', async () => {
    // This used to print `vorn-server token create` and leave you to it, which
    // stopped making sense the moment the server could be another machine.
    store.config = { defaults: { networkAccessEnabled: true } }

    await renderPanel()

    expect(screen.getByText('Add device')).toBeInTheDocument()
    expect(screen.queryByText(/vorn-server token create/)).not.toBeInTheDocument()
  })

  it('shows no address while remote access is off', async () => {
    await renderPanel()

    expect(screen.queryByText('http://192.168.1.20:4000/app/')).not.toBeInTheDocument()
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

  it('presents Tailscale as advice, not as a blocked state', async () => {
    await renderPanel()

    expect(screen.getByText('Tailscale is not installed')).toBeInTheDocument()
    expect(screen.getByText(/Remote access works without it/)).toBeInTheDocument()
  })

  it('drops the Tailscale card once connected', async () => {
    getTailscaleStatus.mockResolvedValue(
      tailscale({ installed: true, running: true, selfIP: '100.1.2.3' })
    )

    await renderPanel()

    expect(screen.queryByText('Tailscale is not installed')).not.toBeInTheDocument()
    expect(screen.queryByText('Tailscale is not connected')).not.toBeInTheDocument()
  })

  it('no longer claims the tailnet is what keeps people out', async () => {
    // The old copy said "Only devices signed into your Tailscale account can reach
    // this address. No passwords" — both clauses are false now that the bind is wide
    // and a token is mandatory.
    await renderPanel()

    expect(screen.queryByText(/No passwords/)).not.toBeInTheDocument()
    expect(screen.getByText(/carries a device token/)).toBeInTheDocument()
  })

  it('still renders when the status calls fail', async () => {
    // A server that is down must not leave the panel stuck on its spinner.
    getTailscaleStatus.mockRejectedValue(new Error('server unreachable'))
    getReachableUrls.mockRejectedValue(new Error('server unreachable'))
    vi.spyOn(console, 'error').mockImplementation(() => {})

    render(<NetworkSettings />)

    await waitFor(() => expect(screen.getByText('Enable Remote Access')).toBeInTheDocument())
  })

  it('renders nothing before the config has loaded', async () => {
    store.config = null as unknown as typeof store.config

    const { container } = render(<NetworkSettings />)

    expect(container).toBeEmptyDOMElement()
  })

  it('drops the addresses when a refresh fails', async () => {
    // Keeping the previous ones would leave the panel advertising a URL while it
    // cannot reach the server to confirm it — which reads as "still reachable" at
    // exactly the moment it is not.
    const user = userEvent.setup()
    store.config = { defaults: { networkAccessEnabled: true } }
    await renderPanel()
    expect(screen.getByText('http://192.168.1.20:4000/app/')).toBeInTheDocument()

    getTailscaleStatus.mockRejectedValue(new Error('server unreachable'))
    getReachableUrls.mockRejectedValue(new Error('server unreachable'))
    vi.spyOn(console, 'error').mockImplementation(() => {})
    await user.click(screen.getByTitle('Check again'))

    await waitFor(() =>
      expect(screen.queryByText('http://192.168.1.20:4000/app/')).not.toBeInTheDocument()
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
    // It is stored as a hash, so there is no second chance to read it. The UI has
    // to say so rather than let someone close the panel and find out.
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

describe('pointing this desktop at another Vorn', () => {
  beforeEach(() => {
    getConnectSettings.mockResolvedValue({ mode: 'local', url: '', hasToken: false })
  })

  it('offers to connect to a server on another machine', async () => {
    await renderPanel()

    await waitFor(() => expect(screen.getByText('Connect to another Vorn')).toBeInTheDocument())
  })

  it('says the app restarts and what still needs a desktop', async () => {
    // Workflow execution lives in the renderer, so a host with nothing attached
    // holds state and runs terminals but fires no schedules. Better said here than
    // discovered when a scheduled workflow silently does not run.
    const user = userEvent.setup()
    await renderPanel()
    await waitFor(() => expect(screen.getByText('Connect')).toBeInTheDocument())

    await user.click(screen.getByText('Connect'))

    expect(screen.getByText(/Vorn restarts to apply this/)).toBeInTheDocument()
    expect(screen.getByText(/only while a desktop is\s+attached/)).toBeInTheDocument()
  })

  it('sends the address and token together', async () => {
    const user = userEvent.setup()
    saveConnectSettings.mockResolvedValue({ ok: true })
    await renderPanel()
    await waitFor(() => expect(screen.getByText('Connect')).toBeInTheDocument())

    await user.click(screen.getByText('Connect'))
    await user.type(screen.getByLabelText('Server address'), '192.168.0.4:61601')
    await user.type(screen.getByLabelText('Device token from that machine'), 'vorn_a_b')
    await user.click(screen.getByText('Connect and restart'))

    expect(saveConnectSettings).toHaveBeenCalledWith({
      url: '192.168.0.4:61601',
      token: 'vorn_a_b'
    })
  })

  it('shows which host it is on, and offers a way back', async () => {
    const user = userEvent.setup()
    getConnectSettings.mockResolvedValue({
      mode: 'host',
      url: 'ws://box:61601/ws',
      hasToken: true
    })
    await renderPanel()

    await waitFor(() => expect(screen.getByText('Connected to another Vorn')).toBeInTheDocument())
    await user.click(screen.getByText('Change'))

    await user.click(screen.getByText('Use this machine'))
    expect(useLocalServer).toHaveBeenCalled()
  })

  it('stays out of the way in the browser, which is already on a host', async () => {
    getConnectSettings.mockResolvedValue(null)

    await renderPanel()

    await waitFor(() => expect(screen.getByText('Enable Remote Access')).toBeInTheDocument())
    expect(screen.queryByText('Connect to another Vorn')).not.toBeInTheDocument()
  })
})
