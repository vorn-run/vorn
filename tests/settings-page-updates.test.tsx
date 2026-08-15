// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { SettingsCategory } from '../src/renderer/stores/types'

const mockStore = {
  setSettingsOpen: vi.fn(),
  settingsCategory: 'appearance' as SettingsCategory,
  setSettingsCategory: vi.fn()
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) =>
    selector ? selector(mockStore) : mockStore
}))

// The Updates entry is gated on this, and it is the whole point of the file.
const platform = { isElectron: true, isMac: false, isWeb: false }
vi.mock('../src/renderer/lib/platform', () => ({
  get isElectron() {
    return platform.isElectron
  },
  get isMac() {
    return platform.isMac
  },
  get isWeb() {
    return platform.isWeb
  },
  MOD: 'Ctrl',
  TRAFFIC_LIGHT_PAD_PX: 78
}))

// Every panel is stubbed: this file is about which one is chosen, not what
// any of them render. Spelled out rather than built in a loop — vi.mock paths
// must be statically analysable to hoist correctly.
vi.mock('../src/renderer/components/settings/AppearanceSettings', () => ({
  AppearanceSettings: () => <div data-testid="appearance-panel" />
}))
vi.mock('../src/renderer/components/settings/GeneralSettings', () => ({
  GeneralSettings: () => <div data-testid="general-panel" />
}))
vi.mock('../src/renderer/components/settings/UpdatesSettings', () => ({
  UpdatesSettings: () => <div data-testid="updates-panel" />
}))
vi.mock('../src/renderer/components/settings/NotificationSettings', () => ({
  NotificationSettings: () => <div data-testid="notifications-panel" />
}))
vi.mock('../src/renderer/components/settings/WorktreeSettings', () => ({
  WorktreeSettings: () => <div data-testid="worktrees-panel" />
}))
vi.mock('../src/renderer/components/settings/AgentSettings', () => ({
  AgentSettings: () => <div data-testid="agents-panel" />
}))
vi.mock('../src/renderer/components/settings/SSHSettings', () => ({
  SSHSettings: () => <div data-testid="ssh-panel" />
}))
vi.mock('../src/renderer/components/settings/McpSettings', () => ({
  McpSettings: () => <div data-testid="mcp-panel" />
}))
vi.mock('../src/renderer/components/settings/ConnectorSettings', () => ({
  ConnectorSettings: () => <div data-testid="connectors-panel" />
}))
vi.mock('../src/renderer/components/settings/NetworkSettings', () => ({
  NetworkSettings: () => <div data-testid="network-panel" />
}))
vi.mock('../src/renderer/components/settings/AboutSettings', () => ({
  AboutSettings: () => <div data-testid="about-panel" />
}))

vi.mock('framer-motion', () => ({
  motion: {
    div: ({ children, ...props }: React.ComponentProps<'div'>) => <div {...props}>{children}</div>
  }
}))

// Panels reach for window.api at render time (AboutSettings reads the version
// directly), and vi.mock paths built in a loop are not reliably hoisted.
Object.defineProperty(window, 'api', {
  value: { getAppVersion: () => '0.6.0-beta.4' },
  writable: true
})

const { SettingsPage } = await import('../src/renderer/components/SettingsPage')

beforeEach(() => {
  vi.clearAllMocks()
  platform.isElectron = true
  mockStore.settingsCategory = 'appearance'
})

describe('SettingsPage', () => {
  it('offers Updates in the desktop app', () => {
    render(<SettingsPage />)
    expect(screen.getByRole('button', { name: 'Updates' })).toBeInTheDocument()
  })

  it('hides Updates on the web, where there is no updater to drive', () => {
    // Without this the web build shows a panel of dead controls.
    platform.isElectron = false
    render(<SettingsPage />)

    expect(screen.queryByRole('button', { name: 'Updates' })).not.toBeInTheDocument()
    // The gate is specific to Updates — its neighbours must survive it.
    expect(screen.getByRole('button', { name: 'General' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Appearance' })).toBeInTheDocument()
  })

  it('renders the Updates panel when that category is selected', () => {
    mockStore.settingsCategory = 'updates'
    render(<SettingsPage />)

    expect(screen.getByTestId('updates-panel')).toBeInTheDocument()
    expect(screen.queryByTestId('general-panel')).not.toBeInTheDocument()
  })

  it('selects a category when its nav item is clicked', () => {
    render(<SettingsPage />)
    screen.getByRole('button', { name: 'Updates' }).click()
    expect(mockStore.setSettingsCategory).toHaveBeenCalledWith('updates')
  })

  it('still renders one panel per category for the rest of the sections', () => {
    mockStore.settingsCategory = 'about'
    render(<SettingsPage />)
    expect(screen.getByTestId('about-panel')).toBeInTheDocument()
  })
})
