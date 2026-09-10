// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { ConnectionRow } from '../src/renderer/components/settings/ConnectionRow'
import type { SdkBrowserSignIn, SourceConnection } from '../src/shared/types'

const browser: SdkBrowserSignIn = {
  signInUrl: 'https://substack.com/sign-in',
  origins: ['https://substack.com', 'https://*.substack.com'],
  check: { url: 'https://substack.com/api/v1/user/profile/self', identity: ['name', 'handle'] }
}

const signInConnection = vi.fn()
const signOutConnection = vi.fn()

beforeEach(() => {
  signInConnection.mockReset().mockResolvedValue({ ok: true, identity: 'Javier Canizalez' })
  signOutConnection.mockReset().mockResolvedValue(undefined)
  ;(window as unknown as { api: unknown }).api = { signInConnection, signOutConnection }
})

function setup(conn: Partial<SourceConnection> = {}) {
  const onRefresh = vi.fn()
  const utils = render(
    <ConnectionRow
      conn={
        {
          id: 'c1',
          connectorId: 'mcp',
          name: 'Substack',
          filters: {},
          syncIntervalMinutes: 5,
          statusMapping: {},
          createdAt: '2026-09-10T20:00:00.000Z',
          ...conn
        } as SourceConnection
      }
      browserSignIn={browser}
      seededWorkflows={[]}
      missingEvents={[]}
      activity={{ busy: {}, failed: {}, run: async () => {}, state: () => ({}) }}
      backfillResult={{}}
      onRun={vi.fn()}
      onBackfill={vi.fn()}
      onDelete={vi.fn()}
      onResetWorkflow={vi.fn()}
      onOpenWorkflow={vi.fn()}
      onRefresh={onRefresh}
    />
  )
  return { ...utils, onRefresh }
}

describe('a connection that signs in through a Vorn window', () => {
  it('says who it is signed in as, and offers to sign in again or out', async () => {
    const { getByText, getByRole, onRefresh } = setup({
      signedInAs: 'Javier Canizalez (javiercanizalez)',
      signedInAt: '2026-09-10T20:05:00.000Z'
    })
    expect(getByText('Signed in as Javier Canizalez (javiercanizalez)')).toBeInTheDocument()
    expect(getByRole('button', { name: 'Sign in again' })).toBeInTheDocument()
    fireEvent.click(getByRole('button', { name: 'Sign out' }))
    await waitFor(() => expect(signOutConnection).toHaveBeenCalledWith('c1'))
    await waitFor(() => expect(onRefresh).toHaveBeenCalled())
  })

  it('says when no one is signed in, and opens the window when asked', async () => {
    const { getByText, getByRole, onRefresh } = setup()
    expect(getByText('Not signed in')).toBeInTheDocument()
    fireEvent.click(getByRole('button', { name: 'Sign in' }))
    await waitFor(() => expect(signInConnection).toHaveBeenCalledWith('c1', undefined))
    await waitFor(() => expect(onRefresh).toHaveBeenCalled())
  })

  it('takes an emailed sign-in link, for a site that signs in by email', async () => {
    const { getByRole } = setup()
    fireEvent.click(getByRole('button', { name: 'Use a sign-in link' }))
    const field = getByRole('textbox', { name: 'Sign-in link' })
    fireEvent.change(field, { target: { value: 'https://substack.com/sign-in?token=abc' } })
    fireEvent.keyDown(field, { key: 'Enter' })
    await waitFor(() =>
      expect(signInConnection).toHaveBeenCalledWith('c1', 'https://substack.com/sign-in?token=abc')
    )
  })

  it("shows the window's answer when no one signed in", async () => {
    signInConnection.mockResolvedValue({
      ok: false,
      message: 'The sign-in window closed before anyone signed in.'
    })
    const { getByRole, findByText } = setup()
    fireEvent.click(getByRole('button', { name: 'Sign in' }))
    expect(
      await findByText('The sign-in window closed before anyone signed in.')
    ).toBeInTheDocument()
  })
})
