// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { AppConfig } from '../src/shared/types'

const config = (defaults: Partial<AppConfig['defaults']> = {}): AppConfig =>
  ({
    version: 1,
    defaults: { shell: '/bin/zsh', fontSize: 13, theme: 'dark', ...defaults },
    workflows: []
  }) as unknown as AppConfig

const mockStore = { config: config(), setConfig: vi.fn() }
vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) =>
    selector ? selector(mockStore) : mockStore
}))

const platform = { isElectron: true }
vi.mock('../src/renderer/lib/platform', () => ({
  get isElectron() {
    return platform.isElectron
  },
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

const saveConfig = vi.fn()
beforeEach(() => {
  mockStore.config = config()
  mockStore.setConfig.mockReset()
  saveConfig.mockReset()
  platform.isElectron = true
  ;(window as unknown as { api: unknown }).api = { saveConfig }
})

const row = (label: string) => screen.getByText(label).closest('div')!

describe('Start Vorn When I Sign In', () => {
  it('sits directly above Reopen Sessions', () => {
    render(<GeneralSettings />)
    const start = row('Start Vorn When I Sign In')
    const reopen = row('Reopen Sessions on Startup')
    expect(start.compareDocumentPosition(reopen) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })

  it('is off until asked, and saves the choice', () => {
    render(<GeneralSettings />)
    const toggle = screen.getAllByRole('switch')[0]
    expect(toggle).toHaveAttribute('aria-checked', 'false')
    fireEvent.click(toggle)
    expect(saveConfig).toHaveBeenCalledWith(
      expect.objectContaining({ defaults: expect.objectContaining({ startAtLogin: true }) })
    )
  })

  it('warns what reopen makes it mean, only while reopen is on', () => {
    const note = /agents start again at sign-in/
    const { unmount } = render(<GeneralSettings />)
    expect(screen.getByText(note)).toBeInTheDocument()
    unmount()
    mockStore.config = config({ reopenSessions: false })
    render(<GeneralSettings />)
    expect(screen.queryByText(note)).not.toBeInTheDocument()
  })

  it('is not offered on the web, which has no sign-in to open at', () => {
    platform.isElectron = false
    render(<GeneralSettings />)
    expect(screen.queryByText('Start Vorn When I Sign In')).not.toBeInTheDocument()
  })
})
