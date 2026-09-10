// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { AppConfig, UpdateStatus, ServerRuntimeStatus } from '../src/shared/types'

const mockStore = {
  config: null as AppConfig | null,
  setConfig: vi.fn(),
  appUpdateStatus: { kind: 'idle', lastCheckedAt: null } as UpdateStatus,
  /** The panel says what restarting costs, so it reads the board. */
  terminals: new Map<string, { status: string; ended?: unknown }>()
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) =>
    selector ? selector(mockStore) : mockStore
}))

const installUpdate = vi.fn()
const downloadUpdate = vi.fn()
const checkForUpdates = vi.fn()
const setUpdateChannel = vi.fn()
const setUpdateAutoDownload = vi.fn()
const saveConfig = vi.fn()
const upgradeServer = vi.fn(async () => runtimeStatus)

/**
 * Which build is holding the terminals.
 *
 * The server outlives the app, so after an update these are briefly different
 * builds — and the panel is where somebody looks to find that out.
 */
let runtimeStatus: ServerRuntimeStatus = {
  serverVersion: '0.6.0-beta.4',
  appVersion: '0.6.0-beta.4',
  serverPid: 4242,
  adopted: true,
  canUpgrade: false,
  sessionsSurviveUpdate: true,
  sessions: null,
  lastUpgrade: null
}

Object.defineProperty(window, 'api', {
  value: {
    installUpdate,
    downloadUpdate,
    checkForUpdates,
    setUpdateChannel,
    setUpdateAutoDownload,
    saveConfig,
    getAppVersion: () => '0.6.0-beta.4',
    getServerRuntimeStatus: () => runtimeStatus,
    onServerRuntimeStatus: () => () => {},
    upgradeServer
  },
  writable: true
})

const { UpdatesSettings } = await import('../src/renderer/components/settings/UpdatesSettings')

function makeConfig(defaults: Partial<AppConfig['defaults']> = {}): AppConfig {
  return {
    version: 1,
    defaults: { shell: '/bin/zsh', fontSize: 13, theme: 'dark', ...defaults }
  } as AppConfig
}

beforeEach(() => {
  vi.clearAllMocks()
  mockStore.config = makeConfig()
  mockStore.appUpdateStatus = { kind: 'idle', lastCheckedAt: null }
  mockStore.terminals = new Map()
  runtimeStatus = { ...runtimeStatus, sessionsSurviveUpdate: true }
})

describe('UpdatesSettings', () => {
  it('renders nothing until config has loaded', () => {
    mockStore.config = null
    const { container } = render(<UpdatesSettings />)
    expect(container).toBeEmptyDOMElement()
  })

  it('shows the current version from the app rather than from config', () => {
    render(<UpdatesSettings />)
    expect(screen.getByText('Vorn 0.6.0-beta.4')).toBeInTheDocument()
  })

  describe('the status block', () => {
    it('reports being up to date without offering an action', () => {
      render(<UpdatesSettings />)
      expect(screen.getByText('Up to date')).toBeInTheDocument()
      expect(screen.queryByRole('button', { name: 'Restart Now' })).not.toBeInTheDocument()
    })

    it('offers a restart, and only a restart, once an update is staged', () => {
      mockStore.appUpdateStatus = { kind: 'ready', version: '0.7.0' }
      render(<UpdatesSettings />)

      expect(screen.getByText('Version 0.7.0 is ready to install')).toBeInTheDocument()
      fireEvent.click(screen.getByRole('button', { name: 'Restart Now' }))
      expect(installUpdate).toHaveBeenCalledTimes(1)
      expect(screen.queryByRole('button', { name: 'Download' })).not.toBeInTheDocument()
    })

    it('offers the deferred download when auto-download is off', () => {
      mockStore.appUpdateStatus = { kind: 'available', version: '0.7.0' }
      render(<UpdatesSettings />)

      fireEvent.click(screen.getByRole('button', { name: 'Download' }))
      expect(downloadUpdate).toHaveBeenCalledTimes(1)
    })

    it('surfaces a failure with a retry instead of staying silent', () => {
      mockStore.appUpdateStatus = { kind: 'error', message: 'ENOTFOUND update.vorn.run' }
      render(<UpdatesSettings />)

      // The label stays general: a failed download lands in this same state,
      // so naming the check would misreport what went wrong.
      expect(screen.getByText("Couldn't update")).toBeInTheDocument()
      expect(screen.getByText('ENOTFOUND update.vorn.run')).toBeInTheDocument()
      fireEvent.click(screen.getByRole('button', { name: 'Retry' }))
      expect(checkForUpdates).toHaveBeenCalledTimes(1)
    })

    it('draws a progress bar only while a download is in flight', () => {
      mockStore.appUpdateStatus = { kind: 'downloading', version: '0.7.0', percent: 42 }
      const { container } = render(<UpdatesSettings />)

      expect(screen.getByText('Downloading 0.7.0')).toBeInTheDocument()
      const bar = container.querySelector('[style*="width: 42%"]')
      expect(bar).not.toBeNull()
    })

    it('explains a dev build rather than looking stuck', () => {
      mockStore.appUpdateStatus = { kind: 'unsupported' }
      render(<UpdatesSettings />)
      expect(screen.getByText('Updates are off in development')).toBeInTheDocument()
    })
  })

  describe('Check Now', () => {
    it('asks main to check', () => {
      render(<UpdatesSettings />)
      fireEvent.click(screen.getByRole('button', { name: 'Check Now' }))
      expect(checkForUpdates).toHaveBeenCalledTimes(1)
    })

    it('is disabled mid-check so it cannot be double-fired', () => {
      mockStore.appUpdateStatus = { kind: 'checking' }
      render(<UpdatesSettings />)
      expect(screen.getByRole('button', { name: 'Checking…' })).toBeDisabled()
    })

    it('is disabled where there is no updater to ask', () => {
      mockStore.appUpdateStatus = { kind: 'unsupported' }
      render(<UpdatesSettings />)
      expect(screen.getByRole('button', { name: 'Check Now' })).toBeDisabled()
    })
  })

  describe('channel', () => {
    it('defaults to stable when config has never set one', () => {
      render(<UpdatesSettings />)
      expect(screen.getByRole('radio', { name: 'Stable' })).toHaveAttribute('aria-checked', 'true')
    })

    it('persists the choice and tells the live updater about it', () => {
      render(<UpdatesSettings />)
      fireEvent.click(screen.getByRole('radio', { name: 'Beta' }))

      expect(saveConfig).toHaveBeenCalledTimes(1)
      expect(saveConfig.mock.calls[0][0].defaults.updateChannel).toBe('beta')
      expect(mockStore.setConfig).toHaveBeenCalledTimes(1)
      // Both writes matter: without the IPC the running updater keeps polling
      // the old feed until the next launch.
      expect(setUpdateChannel).toHaveBeenCalledWith('beta')
    })

    it('reflects a channel already stored in config', () => {
      mockStore.config = makeConfig({ updateChannel: 'beta' })
      render(<UpdatesSettings />)
      expect(screen.getByRole('radio', { name: 'Beta' })).toHaveAttribute('aria-checked', 'true')
    })
  })

  describe('auto-download', () => {
    it('is on unless config says otherwise', () => {
      render(<UpdatesSettings />)
      expect(screen.getByRole('switch')).toHaveAttribute('aria-checked', 'true')
    })

    it('reads an explicit false out of config', () => {
      mockStore.config = makeConfig({ updateAutoDownload: false })
      render(<UpdatesSettings />)
      expect(screen.getByRole('switch')).toHaveAttribute('aria-checked', 'false')
    })

    it('persists the toggle and mirrors it onto the live updater', () => {
      render(<UpdatesSettings />)
      fireEvent.click(screen.getByRole('switch'))

      expect(saveConfig.mock.calls[0][0].defaults.updateAutoDownload).toBe(false)
      expect(setUpdateAutoDownload).toHaveBeenCalledWith(false)
    })
  })
})

describe('what the restart will cost', () => {
  it('names the sessions it will end', () => {
    mockStore.appUpdateStatus = { kind: 'ready', version: '0.7.0-beta.13' }
    mockStore.terminals = new Map([
      ['a', { status: 'idle' }],
      ['b', { status: 'idle' }]
    ])
    render(<UpdatesSettings />)
    expect(screen.getByText(/Your 2 sessions keep running through the update/)).toBeInTheDocument()
  })

  it('says they end where the update has to stop the server', () => {
    runtimeStatus = { ...runtimeStatus, sessionsSurviveUpdate: false }
    mockStore.appUpdateStatus = { kind: 'ready', version: '0.7.0-beta.13' }
    mockStore.terminals = new Map([
      ['a', { status: 'idle' }],
      ['b', { status: 'idle' }]
    ])
    render(<UpdatesSettings />)
    expect(screen.getByText(/Your 2 sessions end with the update/)).toBeInTheDocument()
  })

  it('names the turn only when one is running', () => {
    mockStore.appUpdateStatus = { kind: 'ready', version: '0.7.0-beta.13' }
    mockStore.terminals = new Map([['a', { status: 'running' }]])
    render(<UpdatesSettings />)
    expect(screen.getByText(/The turn in flight continues/)).toBeInTheDocument()
  })

  it('says nothing about sessions when there are none', () => {
    mockStore.appUpdateStatus = { kind: 'ready', version: '0.7.0-beta.13' }
    render(<UpdatesSettings />)
    expect(screen.queryByText(/restart on the new version/)).not.toBeInTheDocument()
    // The status detail is still there; it was replaced, not removed.
    expect(screen.getByText(/restart to apply/)).toBeInTheDocument()
  })

  it('stays quiet while a download is only downloading', () => {
    // Nothing ends until the button that ends it appears.
    mockStore.appUpdateStatus = { kind: 'downloading', version: '0.7.0-beta.13', percent: 40 }
    mockStore.terminals = new Map([['a', { status: 'running' }]])
    render(<UpdatesSettings />)
    expect(screen.queryByText(/restart on the new version/)).not.toBeInTheDocument()
  })
})

describe('sessions that have already ended', () => {
  it('are not counted, because the update does not end them again', () => {
    mockStore.appUpdateStatus = { kind: 'ready', version: '0.7.0-beta.13' }
    mockStore.terminals = new Map([
      ['a', { status: 'idle', ended: { reason: 'app-closed', at: 1, replayed: true } }],
      ['b', { status: 'idle' }]
    ])
    render(<UpdatesSettings />)
    expect(screen.getByText(/Your session keeps running through the update/)).toBeInTheDocument()
  })

  it('leave nothing to say when they are all there is', () => {
    mockStore.appUpdateStatus = { kind: 'ready', version: '0.7.0-beta.13' }
    mockStore.terminals = new Map([
      ['a', { status: 'idle', ended: { reason: 'app-closed', at: 1, replayed: true } }]
    ])
    render(<UpdatesSettings />)
    expect(screen.queryByText(/restart on the new version/)).not.toBeInTheDocument()
  })
})

describe('which build is serving', () => {
  beforeEach(() => {
    mockStore.config = makeConfig()
    upgradeServer.mockClear()
    runtimeStatus = {
      serverVersion: '0.6.0-beta.4',
      appVersion: '0.6.0-beta.4',
      serverPid: 4242,
      adopted: true,
      canUpgrade: false,
      sessionsSurviveUpdate: true,
      sessions: null,
      lastUpgrade: null
    }
  })

  it('names the server beside the app, even when they agree', () => {
    render(<UpdatesSettings />)
    // Always shown: a row that appears only when something is wrong is one
    // nobody knows exists, and this is the question the panel gets opened for.
    expect(screen.getByText('Terminal server')).toBeInTheDocument()
    expect(screen.getByText('Current')).toBeInTheDocument()
  })

  it('offers to move a server left behind by an older build', () => {
    runtimeStatus = { ...runtimeStatus, serverVersion: '0.5.0', canUpgrade: true, sessions: 2 }
    render(<UpdatesSettings />)

    expect(screen.getByText(/0\.5\.0 is still holding your terminals/)).toBeInTheDocument()
    // The reassurance is the number: "moving" reads as destructive without it.
    expect(screen.getByText(/2 terminals across without stopping them/)).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Move to this build' }))
    expect(upgradeServer).toHaveBeenCalled()
  })

  it('says why it cannot move one rather than showing a dead button', () => {
    runtimeStatus = { ...runtimeStatus, serverVersion: '0.5.0', canUpgrade: false }
    render(<UpdatesSettings />)

    expect(screen.getByText(/needs a restart of Vorn/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Move to this build' })).not.toBeInTheDocument()
    expect(screen.getByText('Restart to move')).toBeInTheDocument()
  })
})
