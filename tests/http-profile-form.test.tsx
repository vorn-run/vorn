// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { HttpProfileForm } from '../src/renderer/components/settings/HttpProfileForm'

const createConnection = vi.fn()
const updateConnection = vi.fn()
const preflightConnection = vi.fn()
const encryptString = vi.fn()

beforeEach(() => {
  createConnection.mockReset().mockResolvedValue({ id: 'conn-9' })
  updateConnection.mockReset().mockResolvedValue({ id: 'conn-9' })
  preflightConnection.mockReset().mockResolvedValue({ ok: true, message: '200 from /health' })
  encryptString.mockReset().mockResolvedValue('cipher')
  ;(window as unknown as { api: unknown }).api = {
    createConnection,
    updateConnection,
    preflightConnection,
    encryptString
  }
})

/** A profile is refused at run time without a base URL, so every case fills one. */
function fillBaseUrl(value = 'https://api.example.com'): void {
  fireEvent.change(screen.getByPlaceholderText('https://api.example.com'), {
    target: { value }
  })
}

function form(onDone = vi.fn()) {
  render(<HttpProfileForm name="reporting API" onDone={onDone} onCancel={vi.fn()} />)
  fillBaseUrl()
  return { onDone }
}

describe('making a profile where it was asked for', () => {
  it('starts from the name the template used', () => {
    form()
    expect(screen.getByDisplayValue('reporting API')).toBeInTheDocument()
  })

  it('will not write a profile with no base URL to confine it to', () => {
    render(<HttpProfileForm name="reporting API" onDone={vi.fn()} onCancel={vi.fn()} />)
    expect(screen.getByRole('button', { name: 'Save and test' })).toBeDisabled()
  })

  // A profile left behind by a failed attempt would be a second connection of
  // the same name, and a requirement cannot choose between two.
  it('corrects the profile it already made instead of adding another', async () => {
    preflightConnection.mockResolvedValue({ ok: false, message: '401 Unauthorized' })
    const { onDone } = form()
    fireEvent.click(screen.getByRole('button', { name: 'Save and test' }))
    await waitFor(() => expect(screen.getByText('401 Unauthorized')).toBeInTheDocument())

    preflightConnection.mockResolvedValue({ ok: true })
    fillBaseUrl('https://api.example.com/v2')
    fireEvent.click(screen.getByRole('button', { name: 'Save and test' }))

    await waitFor(() => expect(onDone).toHaveBeenCalled())
    expect(createConnection).toHaveBeenCalledTimes(1)
    expect(updateConnection).toHaveBeenCalledTimes(1)
    expect(updateConnection.mock.calls[0][0]).toBe('conn-9')
  })

  it('hands back the profile it made, so the step can bind it', async () => {
    const { onDone } = form()
    fireEvent.click(screen.getByRole('button', { name: 'Save and test' }))

    await waitFor(() => expect(onDone).toHaveBeenCalledWith('conn-9'))
  })

  it('saves the profile, tests it, and hands back when it answers', async () => {
    const { onDone } = form()
    fireEvent.click(screen.getByRole('button', { name: 'Save and test' }))

    await waitFor(() => expect(onDone).toHaveBeenCalled())
    expect(createConnection).toHaveBeenCalledWith(
      expect.objectContaining({ connectorId: 'http', name: 'reporting API' })
    )
    expect(preflightConnection).toHaveBeenCalledWith('conn-9')
  })

  it('never stores a secret as it was typed', async () => {
    const { onDone } = form()
    fireEvent.change(screen.getByLabelText('Secret'), { target: { value: 'hunter2' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save and test' }))

    await waitFor(() => expect(onDone).toHaveBeenCalled())
    expect(encryptString).toHaveBeenCalledWith('hunter2')
    const filters = createConnection.mock.calls[0][0].filters as Record<string, unknown>
    expect(filters.secret).toBe('cipher')
  })

  // A profile that cannot answer is the one worth correcting, so the form stays.
  it('stays open and says why when the test fails', async () => {
    preflightConnection.mockResolvedValue({ ok: false, message: '401 Unauthorized' })
    const { onDone } = form()
    fireEvent.click(screen.getByRole('button', { name: 'Save and test' }))

    await waitFor(() => expect(screen.getByText('401 Unauthorized')).toBeInTheDocument())
    expect(onDone).not.toHaveBeenCalled()
  })

  it('reports a save that never got as far as a test', async () => {
    createConnection.mockRejectedValue(new Error('the keychain is locked'))
    const { onDone } = form()
    fireEvent.click(screen.getByRole('button', { name: 'Save and test' }))

    await waitFor(() => expect(screen.getByText('the keychain is locked')).toBeInTheDocument())
    expect(onDone).not.toHaveBeenCalled()
  })
})
