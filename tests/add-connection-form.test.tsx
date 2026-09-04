// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { ConnectorManifest } from '../src/shared/types'

const mockState = {
  config: { projects: [{ name: 'Novum', path: '/Users/someone/dev/novum' }] }
}

vi.mock('../src/renderer/stores', () => {
  const useAppStore = (selector?: (s: unknown) => unknown) =>
    selector ? selector(mockState) : mockState
  useAppStore.getState = () => mockState
  return { useAppStore }
})

const createConnection = vi.fn()
const encryptString = vi.fn()
const detectRepo = vi.fn()

beforeEach(() => {
  createConnection.mockReset().mockResolvedValue({ id: 'conn-9' })
  encryptString.mockReset().mockResolvedValue('cipher')
  detectRepo.mockReset().mockResolvedValue(null)
  ;(window as unknown as { api: unknown }).api = {
    createConnection,
    encryptString,
    detectRepo,
    listConnections: vi.fn().mockResolvedValue([]),
    listConnectorPacks: vi.fn().mockResolvedValue([]),
    onConfigChanged: () => () => {}
  }
})

const { AddConnectionForm } = await import('../src/renderer/components/settings/AddConnectionForm')

/** A connector wanting one plain field and one secret. */
const manifest = {
  auth: [
    { key: 'profileName', label: 'Name', type: 'text', required: true },
    { key: 'token', label: 'Token', type: 'password' }
  ]
} as unknown as ConnectorManifest

const CONNECTOR = {
  id: 'acme',
  name: 'Acme',
  icon: 'plug',
  capabilities: ['actions'],
  manifest
}

function form(onDone = vi.fn()) {
  render(<AddConnectionForm connector={CONNECTOR} onDone={onDone} onCancel={vi.fn()} />)
  return { onDone }
}

describe('connecting a connector', () => {
  it('names the connector it is connecting', () => {
    form()
    expect(screen.getByText('Connect Acme')).toBeInTheDocument()
  })

  it('will not save until a required field is answered', () => {
    form()
    const save = screen.getByRole('button', { name: /Connect|Save/ })
    expect(save).toBeDisabled()
  })

  it('saves what was typed, with the secret encrypted first', async () => {
    const { onDone } = form()
    fireEvent.change(screen.getByLabelText(/Name/), { target: { value: 'work' } })
    fireEvent.change(screen.getByLabelText(/Token/), { target: { value: 'hunter2' } })
    fireEvent.click(screen.getByRole('button', { name: /Connect|Save/ }))

    await waitFor(() => expect(onDone).toHaveBeenCalled())
    expect(encryptString).toHaveBeenCalledWith('hunter2')
    const params = createConnection.mock.calls[0][0]
    expect(params.connectorId).toBe('acme')
    expect(params.name).toBe('work')
    expect(params.filters.token).toBe('cipher')
    // The plain field travels as it was typed; only the password is wrapped.
    expect(params.filters.profileName).toBe('work')
  })

  it('says so when the keychain will not encrypt', async () => {
    encryptString.mockRejectedValue(new Error('keychain locked'))
    const { onDone } = form()
    fireEvent.change(screen.getByLabelText(/Name/), { target: { value: 'work' } })
    fireEvent.change(screen.getByLabelText(/Token/), { target: { value: 'hunter2' } })
    fireEvent.click(screen.getByRole('button', { name: /Connect|Save/ }))

    await waitFor(() => expect(screen.getByText(/keychain/i)).toBeInTheDocument())
    expect(createConnection).not.toHaveBeenCalled()
    expect(onDone).not.toHaveBeenCalled()
  })

  // Asked for in the manifest, so any connector can have it -- including one
  // that arrives as a pack, which is how it has to work now that the connector
  // this was built for is not shipped.
  it('names a repo-scoped connection after the repository it found', async () => {
    detectRepo.mockResolvedValue({ owner: 'vorn-run', repo: 'vorn' })
    const onDone = vi.fn()
    render(
      <AddConnectionForm
        connector={{ ...CONNECTOR, manifest: { ...manifest, detectRepo: true } }}
        onDone={onDone}
        onCancel={vi.fn()}
      />
    )

    // Detection is what makes the connection saveable at all.
    await waitFor(() => expect(screen.getByRole('button', { name: /Connect/ })).toBeEnabled())
    fireEvent.click(screen.getByRole('button', { name: /Connect/ }))

    await waitFor(() => expect(onDone).toHaveBeenCalled())
    const params = createConnection.mock.calls[0][0]
    expect(params.name).toBe('vorn-run/vorn')
    expect(params.filters.owner).toBe('vorn-run')
    expect(params.filters.repo).toBe('vorn')
  })

  it('stamps the extra filters a listed server arrives with', async () => {
    const onDone = vi.fn()
    render(
      <AddConnectionForm
        connector={CONNECTOR}
        initialAuth={{ profileName: 'preset' }}
        extraFilters={{ command: 'npx' }}
        onDone={onDone}
        onCancel={vi.fn()}
      />
    )

    fireEvent.click(screen.getByRole('button', { name: /Connect/ }))

    await waitFor(() => expect(onDone).toHaveBeenCalled())
    expect(createConnection.mock.calls[0][0].filters.command).toBe('npx')
  })

  it('reports a save the server refused', async () => {
    createConnection.mockRejectedValue(new Error('that name is taken'))
    const { onDone } = form()
    fireEvent.change(screen.getByLabelText(/Name/), { target: { value: 'work' } })
    fireEvent.click(screen.getByRole('button', { name: /Connect|Save/ }))

    await waitFor(() => expect(screen.getByText(/that name is taken/)).toBeInTheDocument())
    expect(onDone).not.toHaveBeenCalled()
  })

  it('does not go looking for a repo unless the connector asked', async () => {
    // The test used to be on the connector's id, so every other connector was
    // repo-free by accident. One that declares nothing must still be.
    form()
    await waitFor(() => expect(screen.getByLabelText(/Name/)).toBeInTheDocument())
    expect(detectRepo).not.toHaveBeenCalled()
  })
})
