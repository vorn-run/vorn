// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { RemoteHost } from '../src/shared/types'

const host: RemoteHost = {
  id: 'h1',
  label: 'Build box',
  hostname: 'box.example',
  user: 'me',
  port: 22,
  authMethod: 'password'
}

const mockStore = {
  config: { remoteHosts: [host] },
  addRemoteHost: vi.fn(),
  removeRemoteHost: vi.fn(),
  updateRemoteHost: vi.fn()
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) =>
    selector ? selector(mockStore) : mockStore
}))

Object.defineProperty(window, 'api', {
  value: {
    listSSHKeys: vi.fn(async () => []),
    isSafeStorageAvailable: vi.fn(async () => true)
  },
  writable: true
})

const { SSHSettings } = await import('../src/renderer/components/settings/SSHSettings')

beforeEach(() => {
  vi.clearAllMocks()
})

describe('a remote host’s password', () => {
  it('is sent once with the save for vornd to keep, and the field empties', () => {
    render(<SSHSettings />)
    fireEvent.click(screen.getByText('Build box'))
    const input = screen.getByPlaceholderText('Enter password')
    fireEvent.change(input, { target: { value: 'hunter2' } })
    fireEvent.blur(input)
    expect(mockStore.updateRemoteHost).toHaveBeenCalledWith('h1', { ...host, password: 'hunter2' })
    expect(input).toHaveValue('')
  })

  it('sends nothing when no password was typed', () => {
    render(<SSHSettings />)
    fireEvent.click(screen.getByText('Build box'))
    fireEvent.blur(screen.getByPlaceholderText('Enter password'))
    expect(mockStore.updateRemoteHost).not.toHaveBeenCalled()
  })
})
