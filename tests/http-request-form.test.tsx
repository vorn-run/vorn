// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, cleanup, fireEvent, screen } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { HttpRequestConfig } from '../src/shared/types'

const listConnections = vi.fn(async () => [
  { id: 'p1', name: 'Acme API', connectorId: 'http' },
  { id: 'g1', name: 'owner/repo', connectorId: 'github' }
])
;(window as unknown as { api: Record<string, unknown> }).api = {
  ...(window as unknown as { api?: Record<string, unknown> }).api,
  listConnections
}

import { HttpRequestConfigForm } from '../src/renderer/components/workflow-editor/panels/HttpRequestConfigForm'

afterEach(cleanup)

const baseConfig: HttpRequestConfig = {
  nodeType: 'httpRequest',
  method: 'GET',
  url: '',
  headers: {},
  body: ''
}

function renderForm(config: Partial<HttpRequestConfig> = {}) {
  const onChange = vi.fn()
  const utils = render(
    <HttpRequestConfigForm config={{ ...baseConfig, ...config }} onChange={onChange} />
  )
  return { ...utils, onChange }
}

describe('the HTTP request form', () => {
  it('shows method, URL, profile, headers, and body fields', () => {
    renderForm()
    expect(screen.getByText('Method')).toBeInTheDocument()
    expect(screen.getByText('URL')).toBeInTheDocument()
    expect(screen.getByText('Auth profile')).toBeInTheDocument()
    expect(screen.getByText('Headers')).toBeInTheDocument()
    expect(screen.getByText('Body')).toBeInTheDocument()
  })

  it('reports URL edits with the rest of the config intact', () => {
    const { onChange } = renderForm({ method: 'POST' })
    const url = screen.getByPlaceholderText('https://api.example.com/items')
    fireEvent.change(url, { target: { value: 'https://x.test/{{trigger.body.id}}' } })
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ method: 'POST', url: 'https://x.test/{{trigger.body.id}}' })
    )
  })

  it('hints a relative path once a profile is chosen', () => {
    renderForm({ profileConnectionId: 'p1' })
    expect(screen.getByPlaceholderText('/v1/items')).toBeInTheDocument()
  })

  it('adds a header row', () => {
    const { onChange } = renderForm()
    fireEvent.click(screen.getByText('Add header'))
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ headers: { '': '' } }))
  })

  it('removes a header row', () => {
    const { onChange } = renderForm({ headers: { 'X-One': '1' } })
    fireEvent.click(screen.getByLabelText('Remove header'))
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ headers: {} }))
  })

  it('says the profile keeps its secret on the server', () => {
    renderForm()
    expect(screen.getByText(/its secret never/)).toBeInTheDocument()
  })
})

describe('the profile dropdown and header rows', () => {
  it('lists only http connections and reports a selection', async () => {
    const { onChange } = renderForm()
    fireEvent.click(screen.getByText('None'))
    expect(await screen.findByText('Acme API')).toBeInTheDocument()
    expect(screen.queryByText('owner/repo')).not.toBeInTheDocument()
    fireEvent.mouseDown(screen.getByText('Acme API'))
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ profileConnectionId: 'p1' }))
  })

  it('edits an existing header name and value', () => {
    const { onChange } = renderForm({ headers: { 'X-One': '1' } })
    fireEvent.change(screen.getByPlaceholderText('Name'), { target: { value: 'X-Two' } })
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ headers: { 'X-Two': '1' } }))
    fireEvent.change(screen.getByPlaceholderText('Value'), { target: { value: '2' } })
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ headers: { 'X-One': '2' } }))
  })
})
