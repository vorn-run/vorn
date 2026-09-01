// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { PackInstallConfirm } from '../src/renderer/components/settings/PackInstallConfirm'
import type { ConnectorPackSummary } from '../src/shared/types'

const PREVIEW: ConnectorPackSummary = {
  id: 'packdemo',
  name: 'Pack Demo',
  version: '1.1.0',
  description: 'Answers back whatever it is asked.',
  triggers: [
    {
      type: 'tick',
      label: 'Tick',
      filters: {
        pollTool: 'poll_tick',
        itemsPath: 'items',
        idField: 'externalId',
        timestampField: 'updatedAt',
        titleField: 'title',
        urlField: 'url',
        cursorArg: 'cursor',
        cursorPath: 'nextCursor'
      }
    }
  ],
  actions: [{ type: 'echo', label: 'Echo' }],
  env: [
    { name: 'API_TOKEN', required: true, secret: true },
    { name: 'REGION', required: false, secret: false }
  ]
}

describe('the sheet shown before a pack is kept', () => {
  it('says what the connector is and what it can do', () => {
    const { getByText } = render(
      <PackInstallConfirm preview={PREVIEW} onConfirm={() => {}} onCancel={() => {}} />
    )

    expect(getByText('Pack Demo')).toBeInTheDocument()
    expect(getByText('v1.1.0')).toBeInTheDocument()
    expect(getByText('Tick')).toBeInTheDocument()
    expect(getByText('Echo')).toBeInTheDocument()
  })

  it('lists only the settings a connector cannot run without', () => {
    const { getByText, queryByText } = render(
      <PackInstallConfirm preview={PREVIEW} onConfirm={() => {}} onCancel={() => {}} />
    )

    expect(getByText('API_TOKEN')).toBeInTheDocument()
    expect(queryByText('REGION')).not.toBeInTheDocument()
  })

  it('offers to install when nothing is on disk yet', () => {
    const onConfirm = vi.fn()
    const { getByText } = render(
      <PackInstallConfirm preview={PREVIEW} onConfirm={onConfirm} onCancel={() => {}} />
    )

    fireEvent.click(getByText('Install'))
    expect(onConfirm).toHaveBeenCalled()
  })

  it('offers to update, naming the version kept for a rollback', () => {
    const { getByText } = render(
      <PackInstallConfirm
        preview={{ ...PREVIEW, installedVersion: '1.0.0' }}
        onConfirm={() => {}}
        onCancel={() => {}}
      />
    )

    expect(getByText('Update')).toBeInTheDocument()
    expect(getByText(/Replaces v1\.0\.0/)).toBeInTheDocument()
  })

  it('lets the decision be refused', () => {
    const onCancel = vi.fn()
    const { getByText } = render(
      <PackInstallConfirm preview={PREVIEW} onConfirm={() => {}} onCancel={onCancel} />
    )

    fireEvent.click(getByText('Cancel'))
    expect(onCancel).toHaveBeenCalled()
  })

  it('cannot be double-submitted while the install runs', () => {
    const onConfirm = vi.fn()
    const { getByText } = render(
      <PackInstallConfirm preview={PREVIEW} busy onConfirm={onConfirm} onCancel={() => {}} />
    )

    fireEvent.click(getByText('Installing…'))
    expect(onConfirm).not.toHaveBeenCalled()
  })

  it('stays silent about capabilities a connector does not have', () => {
    const { queryByText } = render(
      <PackInstallConfirm
        preview={{ ...PREVIEW, triggers: [], actions: [], env: [] }}
        onConfirm={() => {}}
        onCancel={() => {}}
      />
    )

    expect(queryByText('Triggers')).not.toBeInTheDocument()
    expect(queryByText('Actions')).not.toBeInTheDocument()
    expect(queryByText('Needs')).not.toBeInTheDocument()
  })
})
