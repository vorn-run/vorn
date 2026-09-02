// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { ConnectorKey } from '../src/shared/types'
import { KeysSettings } from '../src/renderer/components/settings/KeysSettings'
import { __resetConnectionsCacheForTests } from '../src/renderer/lib/use-connections'

const PROFILE: ConnectorKey = {
  connectionId: 'conn-1',
  name: 'reporting API',
  connectorId: 'http',
  usageCount: 2,
  fields: [{ key: 'secret', label: 'secret', readable: true, hint: 'sk_live_••••4242' }]
}

const PACKAGED: ConnectorKey = {
  connectionId: 'conn-2',
  name: 'Slack',
  connectorId: 'slack',
  usageCount: 0,
  fields: [{ key: 'secretEnv', label: 'secretEnv', readable: true, envNames: ['SLACK_BOT_TOKEN'] }]
}

const api = {
  listConnectorKeys: vi.fn(),
  isSafeStorageAvailable: vi.fn(),
  preflightConnection: vi.fn(),
  encryptString: vi.fn(),
  rotateConnectionSecret: vi.fn(),
  onConfigChanged: vi.fn(() => () => {}),
  listConnections: vi.fn(),
  listConnectorPacks: vi.fn()
}

beforeEach(() => {
  vi.clearAllMocks()
  __resetConnectionsCacheForTests()
  api.listConnectorKeys.mockResolvedValue([PROFILE, PACKAGED])
  api.isSafeStorageAvailable.mockResolvedValue(true)
  api.preflightConnection.mockResolvedValue({ ok: true, message: 'HTTP 200' })
  api.encryptString.mockResolvedValue('sealed')
  api.rotateConnectionSecret.mockResolvedValue({ ok: true })
  api.onConfigChanged.mockReturnValue(() => {})
  api.listConnections.mockResolvedValue([])
  api.listConnectorPacks.mockResolvedValue([])
  ;(window as unknown as { api: unknown }).api = api
})

describe('the keys this machine holds', () => {
  it('says what each key is and what rotating it would touch', async () => {
    render(<KeysSettings />)

    expect(await screen.findByText('reporting API')).toBeInTheDocument()
    expect(screen.getByText(/sk_live_••••4242 · used by 2 steps/)).toBeInTheDocument()
    // A blob is named by the variables it carries, never by their values.
    expect(screen.getByText(/SLACK_BOT_TOKEN · unused/)).toBeInTheDocument()
  })

  it('offers nothing to do when no connection holds a secret', async () => {
    api.listConnectorKeys.mockResolvedValue([])
    render(<KeysSettings />)

    expect(await screen.findByText('No keys yet')).toBeInTheDocument()
  })

  it('says a locked key is still there, rather than showing it as empty', async () => {
    api.listConnectorKeys.mockResolvedValue([
      { ...PROFILE, fields: [{ key: 'secret', label: 'secret', readable: false }] }
    ])
    render(<KeysSettings />)

    expect(await screen.findByText(/Locked — this machine cannot read it/)).toBeInTheDocument()
  })

  it('warns when the keychain cannot seal a replacement', async () => {
    api.isSafeStorageAvailable.mockResolvedValue(false)
    render(<KeysSettings />)

    expect(await screen.findByText(/Keychain encryption is not available/)).toBeInTheDocument()
  })

  it('offers no rotation it could not carry out, rather than failing at Save', async () => {
    api.isSafeStorageAvailable.mockResolvedValue(false)
    render(<KeysSettings />)

    await screen.findByText(/Keychain encryption is not available/)
    for (const button of screen.getAllByRole('button', { name: /Rotate/ })) {
      expect(button).toBeDisabled()
    }
  })

  it('draws a packaged connector with the mark it ships', async () => {
    api.listConnections.mockResolvedValue([
      {
        id: 'conn-2',
        connectorId: 'mcp',
        name: 'Slack',
        filters: {
          sdkConnectorId: 'slack',
          sdkIcon: '{"viewBox":"0 0 24 24","paths":["M2 2h9v9z"]}'
        },
        syncIntervalMinutes: 5,
        statusMapping: {},
        createdAt: '2026-09-02T00:00:00Z'
      }
    ])
    const { container } = render(<KeysSettings />)

    await screen.findByText('Slack')
    await waitFor(() => expect(container.querySelector('path[d="M2 2h9v9z"]')).not.toBeNull())
  })
})

describe('what a key can be asked', () => {
  it('offers Test where a preflight exists', async () => {
    render(<KeysSettings />)
    await screen.findByText('reporting API')

    // One profile, one packaged connection: only the profile answers today.
    expect(screen.getAllByRole('button', { name: 'Test' })).toHaveLength(1)
  })
})

describe('testing a key', () => {
  it('reports what the connection answered', async () => {
    render(<KeysSettings />)
    fireEvent.click((await screen.findAllByRole('button', { name: 'Test' }))[0])

    expect(await screen.findByText('HTTP 200')).toBeInTheDocument()
    expect(api.preflightConnection).toHaveBeenCalledWith('conn-1')
  })

  it('says so when the connector has nothing to check', async () => {
    api.preflightConnection.mockResolvedValue({ ok: null })
    render(<KeysSettings />)
    fireEvent.click((await screen.findAllByRole('button', { name: 'Test' }))[0])

    expect(await screen.findByText('This connector has nothing to check')).toBeInTheDocument()
  })

  it('reports a rejected call rather than leaving the row silent', async () => {
    api.preflightConnection.mockRejectedValue(new Error('the server went away'))
    render(<KeysSettings />)
    fireEvent.click((await screen.findAllByRole('button', { name: 'Test' }))[0])

    expect(await screen.findByText('the server went away')).toBeInTheDocument()
  })
})

describe('rotating a key', () => {
  it('seals the new value before it leaves, and reloads what is held', async () => {
    render(<KeysSettings />)
    fireEvent.click((await screen.findAllByRole('button', { name: /Rotate/ }))[0])
    fireEvent.change(screen.getByPlaceholderText('The replacement value'), {
      target: { value: 'sk_live_new' }
    })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))

    await waitFor(() => expect(api.rotateConnectionSecret).toHaveBeenCalled())
    expect(api.encryptString).toHaveBeenCalledWith('sk_live_new')
    // Ciphertext to store, and the value itself so the key works at once.
    expect(api.rotateConnectionSecret).toHaveBeenCalledWith({
      connectionId: 'conn-1',
      field: 'secret',
      value: 'sealed',
      plaintext: 'sk_live_new'
    })
    await waitFor(() => expect(api.listConnectorKeys).toHaveBeenCalledTimes(2))
  })

  it('re-lists what is held when a key changes somewhere else', async () => {
    render(<KeysSettings />)
    await screen.findByText('reporting API')
    expect(api.onConfigChanged).toHaveBeenCalled()

    api.listConnectorKeys.mockResolvedValue([{ ...PROFILE, name: 'renamed API' }])
    // The connections cache subscribes too; which registered first is not the
    // point of this test.
    for (const [callback] of api.onConfigChanged.mock.calls) {
      ;(callback as () => void)()
    }

    expect(await screen.findByText('renamed API')).toBeInTheDocument()
  })

  it('stops listening when the page goes away', async () => {
    const unsubscribe = vi.fn()
    api.onConfigChanged.mockReturnValue(unsubscribe)
    const { unmount } = render(<KeysSettings />)
    await screen.findByText('reporting API')

    unmount()
    expect(unsubscribe).toHaveBeenCalled()
  })

  it('keeps the field open and says why when the server refuses', async () => {
    api.rotateConnectionSecret.mockResolvedValue({ ok: false, error: 'not a secret here' })
    render(<KeysSettings />)
    fireEvent.click((await screen.findAllByRole('button', { name: /Rotate/ }))[0])
    fireEvent.change(screen.getByPlaceholderText('The replacement value'), {
      target: { value: 'x' }
    })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))

    expect(await screen.findByText('not a secret here')).toBeInTheDocument()
    expect(screen.getByPlaceholderText('The replacement value')).toBeInTheDocument()
  })

  it('will not send an empty replacement', async () => {
    render(<KeysSettings />)
    fireEvent.click((await screen.findAllByRole('button', { name: /Rotate/ }))[0])

    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled()
  })

  it('asks for the whole set when the field carries a set of variables', async () => {
    render(<KeysSettings />)
    fireEvent.click((await screen.findAllByRole('button', { name: /Rotate/ }))[1])

    expect(screen.getByPlaceholderText('{"TOKEN": "…"}')).toBeInTheDocument()
  })

  it('leaves the key alone when the replacement is abandoned', async () => {
    render(<KeysSettings />)
    fireEvent.click((await screen.findAllByRole('button', { name: /Rotate/ }))[0])
    fireEvent.change(screen.getByPlaceholderText('The replacement value'), {
      target: { value: 'typed' }
    })
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))

    expect(screen.queryByPlaceholderText('The replacement value')).not.toBeInTheDocument()
    expect(api.rotateConnectionSecret).not.toHaveBeenCalled()
  })

  it('reports a rejected call rather than looking like it saved', async () => {
    api.rotateConnectionSecret.mockRejectedValue(new Error('the server went away'))
    render(<KeysSettings />)
    fireEvent.click((await screen.findAllByRole('button', { name: /Rotate/ }))[0])
    fireEvent.change(screen.getByPlaceholderText('The replacement value'), {
      target: { value: 'x' }
    })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))

    expect(await screen.findByText('the server went away')).toBeInTheDocument()
  })
})
