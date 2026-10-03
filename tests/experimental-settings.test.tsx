// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { AppConfig } from '../src/shared/types'

const mockStore = {
  config: null as AppConfig | null,
  setConfig: vi.fn()
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) =>
    selector ? selector(mockStore) : mockStore
}))

const saveConfig = vi.fn()
Object.defineProperty(window, 'api', { value: { saveConfig }, writable: true })

const { ExperimentalSettings } =
  await import('../src/renderer/components/settings/ExperimentalSettings')

function makeConfig(defaults: Partial<AppConfig['defaults']> = {}): AppConfig {
  return {
    version: 1,
    defaults: { shell: '/bin/zsh', fontSize: 13, theme: 'dark', ...defaults }
  } as AppConfig
}

beforeEach(() => {
  vi.clearAllMocks()
  mockStore.config = makeConfig()
})

describe('ExperimentalSettings', () => {
  it('renders nothing until config has loaded', () => {
    mockStore.config = null
    const { container } = render(<ExperimentalSettings />)
    expect(container).toBeEmptyDOMElement()
  })

  it('shows Native Git off until it has been turned on', () => {
    render(<ExperimentalSettings />)
    expect(screen.getByText('Native Git')).toBeInTheDocument()
    expect(screen.getByRole('switch')).toHaveAttribute('aria-checked', 'false')
  })

  it('saves the switch without touching the other defaults', () => {
    render(<ExperimentalSettings />)
    fireEvent.click(screen.getByRole('switch'))

    const saved = saveConfig.mock.calls[0][0] as AppConfig
    expect(saved.defaults.experimental).toEqual({ nativeGit: true })
    expect(saved.defaults.shell).toBe('/bin/zsh')
    expect(mockStore.setConfig).toHaveBeenCalledWith(saved)
  })

  it('turns it back off', () => {
    mockStore.config = makeConfig({ experimental: { nativeGit: true } })
    render(<ExperimentalSettings />)
    expect(screen.getByRole('switch')).toHaveAttribute('aria-checked', 'true')
    fireEvent.click(screen.getByRole('switch'))
    expect((saveConfig.mock.calls[0][0] as AppConfig).defaults.experimental).toEqual({
      nativeGit: false
    })
  })
})
