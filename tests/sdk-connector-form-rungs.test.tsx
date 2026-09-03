// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { SdkConnectorForm } from '../src/renderer/components/settings/SdkConnectorForm'
import type { ConnectorCatalogItem, SdkConnectorManifest } from '../src/shared/types'

// What the connection form asks for, per rung.

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector: (state: unknown) => unknown) =>
    selector({ config: { projects: [{ name: 'vorn', path: '/repo' }] } })
}))

const probeSdkConnector = vi.fn()
const probeConnectorAuth = vi.fn()
const createConnection = vi.fn()
const encryptString = vi.fn()

function manifest(auth: SdkConnectorManifest['auth']): SdkConnectorManifest {
  return {
    id: 'gitlab',
    name: 'GitLab',
    version: '1.0.0',
    ...(auth && { auth }),
    triggers: [],
    actions: [{ type: 'createIssue', label: 'Create issue' }],
    env: [
      { name: 'GITLAB_HOST', required: true, secret: false },
      { name: 'GITLAB_TOKEN', required: true, secret: true }
    ]
  }
}

const CATALOG_ENTRY = {
  id: 'gitlab',
  name: 'GitLab',
  packageName: '@vornrun/connector-gitlab',
  capabilities: ['actions'],
  launch: { command: 'npx', args: ['-y', '@vornrun/connector-gitlab'] }
} as unknown as ConnectorCatalogItem

beforeEach(() => {
  probeSdkConnector.mockReset()
  probeConnectorAuth.mockReset().mockResolvedValue({ ok: true, identity: 'javier' })
  createConnection.mockReset().mockResolvedValue({ id: 'c1' })
  encryptString.mockReset().mockImplementation(async (v: string) => `enc(${v})`)
  ;(window as unknown as { api: unknown }).api = {
    probeSdkConnector,
    probeConnectorAuth,
    createConnection,
    encryptString
  }
})

const setup = () => {
  const onDone = vi.fn()
  const utils = render(
    <SdkConnectorForm onDone={onDone} onCancel={vi.fn()} catalogEntry={CATALOG_ENTRY} />
  )
  return { ...utils, onDone }
}

describe('a connector that borrows a login', () => {
  beforeEach(() => {
    probeSdkConnector.mockResolvedValue({
      ok: true,
      manifest: manifest({
        rung: 'cli',
        probe: { command: 'glab', args: ['auth', 'status'] },
        borrow: { env: ['GITLAB_TOKEN'], tokenArgs: ['auth', 'token'] }
      })
    })
  })

  it('says who you already are instead of asking for a token', async () => {
    const { findByText, queryByDisplayValue, container } = setup()
    expect(await findByText(/Signed in as javier/)).toBeInTheDocument()
    // The secret field is what borrowing replaces; the form says what is handed over instead.
    expect(container.textContent).toContain('Hands over GITLAB_TOKEN from glab')
    expect(container.textContent).toContain('GITLAB_HOST')
    expect(queryByDisplayValue('token')).toBeNull()
  })

  it('still asks for a secret the borrow does not cover', async () => {
    probeSdkConnector.mockResolvedValue({
      ok: true,
      manifest: {
        ...manifest({
          rung: 'cli',
          probe: { command: 'glab', args: ['auth', 'status'] },
          borrow: { env: ['GITLAB_TOKEN'], tokenArgs: ['auth', 'token'] }
        }),
        env: [
          { name: 'GITLAB_TOKEN', required: true, secret: true },
          { name: 'WEBHOOK_SECRET', required: true, secret: true }
        ]
      }
    })
    const { findByText, container, getByText } = setup()
    await findByText(/Signed in as javier/)
    // One secret field left: the token is borrowed, the webhook secret is not.
    expect(container.querySelectorAll('input[type="password"]')).toHaveLength(1)
    expect(container.textContent).toContain('WEBHOOK_SECRET')
    expect(getByText('Connect').closest('button')).toBeDisabled()
  })

  it('names the command it asked', async () => {
    const { findByText } = setup()
    expect(await findByText(/glab auth status/)).toBeInTheDocument()
  })

  it('connects without a token, since the borrow supplies one', async () => {
    const { findByText, getByText, container } = setup()
    await findByText(/Signed in as javier/)
    const host = container.querySelector('input[type="text"]') as HTMLInputElement
    fireEvent.change(host, { target: { value: 'gitlab.com' } })

    fireEvent.click(getByText('Connect'))
    await waitFor(() => expect(createConnection).toHaveBeenCalled())
    // Nothing borrowed is written down: the token is asked for at spawn.
    expect(createConnection.mock.calls[0][0].filters.secretEnv).toBeUndefined()
  })

  it('offers a token for the machine where the tool is not the answer', async () => {
    const { findByText, getByText, container } = setup()
    await findByText(/Signed in as javier/)
    fireEvent.click(getByText('Use a token instead'))
    await waitFor(() => expect(container.textContent).toContain('GITLAB_TOKEN'))
  })

  it('names what it will hand over', async () => {
    const { findByText } = setup()
    expect(await findByText(/Hands over GITLAB_TOKEN from glab/)).toBeInTheDocument()
  })

  it('forgets a token typed before the borrow was chosen again', async () => {
    const { findByText, getByText, container } = setup()
    await findByText(/Signed in as javier/)

    fireEvent.click(getByText('Use a token instead'))
    await waitFor(() => expect(container.textContent).toContain('GITLAB_TOKEN'))
    const secret = container.querySelector('input[type="password"]') as HTMLInputElement
    fireEvent.change(secret, { target: { value: 'typed-by-hand' } })
    fireEvent.click(getByText('Borrow the signed-in tool instead'))

    const host = container.querySelector('input[type="text"]') as HTMLInputElement
    fireEvent.change(host, { target: { value: 'gitlab.com' } })
    fireEvent.click(getByText('Connect'))

    await waitFor(() => expect(createConnection).toHaveBeenCalled())
    // Hidden means gone: a token left behind would outrank the borrow it was hidden in favour of.
    expect(createConnection.mock.calls[0][0].filters.secretEnv).toBeUndefined()
    expect(encryptString).not.toHaveBeenCalled()
  })

  it('says the check cannot be made rather than checking forever', async () => {
    probeConnectorAuth.mockResolvedValue({ ok: null })
    const { findByText } = setup()
    expect(await findByText(/Nothing to check for this connector/)).toBeInTheDocument()
  })

  it('says the same when the build cannot ask at all', async () => {
    probeConnectorAuth.mockResolvedValue(undefined)
    const { findByText } = setup()
    expect(await findByText(/Nothing to check for this connector/)).toBeInTheDocument()
  })

  it('says what to run when the tool is signed out', async () => {
    probeConnectorAuth.mockResolvedValue({
      ok: false,
      message: 'Sign in by running `glab auth login` in your terminal.'
    })
    const { findByText } = setup()
    expect(await findByText(/glab auth login/)).toBeInTheDocument()
  })

  it('offers a way to get the tool when it is the tool that is missing', async () => {
    probeConnectorAuth.mockResolvedValue({
      ok: false,
      message: 'glab is not installed or not on PATH.',
      installHint: 'Install with Homebrew: `brew install glab`'
    })
    const { findByText } = setup()
    expect(await findByText(/brew install glab/)).toBeInTheDocument()
  })
})

describe('a connector that asks for nothing', () => {
  beforeEach(() => {
    probeSdkConnector.mockResolvedValue({ ok: true, manifest: manifest({ rung: 'none' }) })
  })

  it('says it is ready rather than asking anything', async () => {
    const { findByText, container } = setup()
    expect(await findByText(/asks for no sign-in/)).toBeInTheDocument()
    expect(container.textContent).not.toContain('GITLAB_TOKEN')
    expect(container.textContent).not.toContain('GITLAB_HOST')
  })

  it('does not make a second connection for one it already has', async () => {
    const { findByText, getByText, onDone } = setup()
    await findByText(/asks for no sign-in/)
    fireEvent.click(getByText('Done'))
    await waitFor(() => expect(onDone).toHaveBeenCalled())
    expect(createConnection).not.toHaveBeenCalled()
  })
})

describe('a connector that asks for a key', () => {
  it('asks for it, exactly as before', async () => {
    probeSdkConnector.mockResolvedValue({
      ok: true,
      manifest: manifest({ rung: 'key', keys: ['GITLAB_TOKEN'] })
    })
    const { findByText, container } = setup()
    await findByText('GITLAB_HOST')
    expect(container.textContent).toContain('GITLAB_TOKEN')
    expect(probeConnectorAuth).not.toHaveBeenCalled()
  })
})
