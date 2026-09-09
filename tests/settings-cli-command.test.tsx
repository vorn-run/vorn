// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { AppConfig } from '../src/shared/types'

const config = (): AppConfig =>
  ({
    version: 1,
    defaults: { shell: '/bin/zsh', fontSize: 13, theme: 'dark' },
    workflows: []
  }) as unknown as AppConfig

const mockStore = { config: config(), setConfig: vi.fn() }
vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) =>
    selector ? selector(mockStore) : mockStore
}))

vi.mock('../src/renderer/lib/platform', () => ({
  isElectron: true,
  isMac: true,
  isWeb: false,
  MOD: 'Cmd'
}))
vi.mock('../src/renderer/hooks/useAgentInstallStatus', () => ({
  useAgentInstallStatus: () => ({ status: {} })
}))
vi.mock('../src/renderer/components/settings/ShellPicker', () => ({
  ShellPicker: () => <div />
}))

import { GeneralSettings } from '../src/renderer/components/settings/GeneralSettings'

const cliCommandStatus = vi.fn()
const installCliCommand = vi.fn()

beforeEach(() => {
  mockStore.config = config()
  cliCommandStatus.mockReset()
  installCliCommand.mockReset()
  ;(window as unknown as { api: unknown }).api = {
    saveConfig: vi.fn(),
    cliCommandStatus,
    installCliCommand
  }
})

describe('the command line tool row', () => {
  it('offers to install the command, and says where it went', async () => {
    cliCommandStatus.mockResolvedValue({
      available: true,
      installed: false,
      path: '/usr/local/bin/vorn',
      onPath: true
    })
    installCliCommand.mockResolvedValue({ ok: true, path: '/usr/local/bin/vorn' })

    render(<GeneralSettings />)
    const button = await screen.findByRole('button', { name: 'Install' })
    expect(screen.getByText('/usr/local/bin/vorn')).toBeInTheDocument()

    fireEvent.click(button)
    await waitFor(() => expect(installCliCommand).toHaveBeenCalled())
    expect(await screen.findByRole('button', { name: 'Reinstall' })).toBeInTheDocument()
  })

  it('shows why it could not, instead of claiming it did', async () => {
    cliCommandStatus.mockResolvedValue({
      available: true,
      installed: false,
      path: '/usr/local/bin/vorn',
      onPath: true
    })
    installCliCommand.mockResolvedValue({ ok: false, error: 'EACCES: permission denied' })

    render(<GeneralSettings />)
    fireEvent.click(await screen.findByRole('button', { name: 'Install' }))

    expect(await screen.findByText('EACCES: permission denied')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Install' })).toBeInTheDocument()
  })

  it('says when the command would land somewhere the shell does not look', async () => {
    cliCommandStatus.mockResolvedValue({
      available: true,
      installed: false,
      path: '/Users/j/.local/bin/vorn',
      onPath: false
    })

    render(<GeneralSettings />)
    expect(await screen.findByText(/add its directory to your PATH/)).toBeInTheDocument()
  })

  it('stays out of the way while running from source', async () => {
    cliCommandStatus.mockResolvedValue({
      available: false,
      installed: false,
      path: '',
      onPath: false
    })

    render(<GeneralSettings />)
    expect(await screen.findByRole('button', { name: 'Install' })).toBeDisabled()
  })
})
