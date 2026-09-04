// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { SourceConnection } from '../src/shared/types'

vi.mock('../src/renderer/components/Tooltip', () => ({
  Tooltip: ({ children }: React.PropsWithChildren) => <>{children}</>
}))

const preflightConnection = vi.fn()
;(window as unknown as { api: Record<string, unknown> }).api = {
  ...(window as unknown as { api?: Record<string, unknown> }).api,
  preflightConnection
}

import { ConnectionRow } from '../src/renderer/components/settings/ConnectionRow'

afterEach(() => {
  cleanup()
  preflightConnection.mockReset()
})

const conn = (connectorId: string): SourceConnection => ({
  id: 'c1',
  connectorId,
  name: 'Acme API',
  filters: {},
  syncIntervalMinutes: 5,
  statusMapping: {},
  createdAt: '2026-08-30T00:00:00.000Z'
})

function renderRow(connectorId = 'http') {
  return render(
    <ConnectionRow
      conn={conn(connectorId)}
      seededWorkflows={[]}
      missingEvents={[]}
      activity={{ busy: {}, failed: {}, run: async () => {} }}
      backfillResult={{}}
      onRun={vi.fn()}
      onBackfill={vi.fn()}
      onDelete={vi.fn()}
      onResetWorkflow={vi.fn()}
      onOpenWorkflow={vi.fn()}
      onRefresh={vi.fn()}
    />
  )
}

describe('the http profile Test button', () => {
  it('runs the preflight and reports the status it got back', async () => {
    preflightConnection.mockResolvedValue({ ok: true, message: 'HTTP 200' })
    const { container } = renderRow()
    fireEvent.click(container.querySelector('svg.lucide-activity')!.closest('button')!)
    await waitFor(() => expect(screen.getByText('HTTP 200')).toBeInTheDocument())
    expect(preflightConnection).toHaveBeenCalledWith('c1')
  })

  it('reports a failed test in the error tone', async () => {
    preflightConnection.mockResolvedValue({ ok: false, message: 'HTTP 401' })
    const { container } = renderRow()
    fireEvent.click(container.querySelector('svg.lucide-activity')!.closest('button')!)
    await waitFor(() => expect(screen.getByText('HTTP 401')).toBeInTheDocument())
    expect(screen.getByText('HTTP 401').className).toContain('text-red-400')
  })

  it('surfaces a thrown preflight as a failure message', async () => {
    preflightConnection.mockRejectedValue(new Error('bridge down'))
    const { container } = renderRow()
    fireEvent.click(container.querySelector('svg.lucide-activity')!.closest('button')!)
    await waitFor(() => expect(screen.getByText('bridge down')).toBeInTheDocument())
  })

  it('does not exist on non-http connections', () => {
    const { container } = renderRow('github')
    expect(container.querySelector('svg.lucide-activity')).toBeNull()
  })
})
