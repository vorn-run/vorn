// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { AppConfig, UpdateStatus } from '../src/shared/types'

const mockStore = {
  config: null as AppConfig | null,
  setConfig: vi.fn(),
  appUpdateStatus: { kind: 'idle', lastCheckedAt: null } as UpdateStatus
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

Object.defineProperty(window, 'api', {
  value: {
    installUpdate,
    downloadUpdate,
    checkForUpdates,
    setUpdateChannel,
    setUpdateAutoDownload,
    saveConfig,
    getAppVersion: () => '0.6.0-beta.4'
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

      expect(screen.getByText("Couldn't check for updates")).toBeInTheDocument()
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
