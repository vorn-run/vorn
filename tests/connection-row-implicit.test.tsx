// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, cleanup, fireEvent, screen } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { SourceConnection } from '../src/shared/types'

/**
 * A connection the app made for a connector that asks for nothing.
 *
 * Deleting it alone would achieve nothing — the next pack change makes it again
 * — so the row says what to do instead of offering an action that undoes itself.
 */

vi.mock('../src/renderer/components/Tooltip', () => ({
  Tooltip: ({ label, children }: React.PropsWithChildren<{ label: string }>) => (
    <span data-tooltip={label}>{children}</span>
  )
}))
;(window as unknown as { api: Record<string, unknown> }).api = {
  ...(window as unknown as { api?: Record<string, unknown> }).api,
  preflightConnection: vi.fn()
}

import { ConnectionRow } from '../src/renderer/components/settings/ConnectionRow'

afterEach(cleanup)

function renderRow(filters: Record<string, unknown>) {
  const onDelete = vi.fn()
  const conn: SourceConnection = {
    id: 'c1',
    connectorId: 'mcp',
    name: 'Echo Bench',
    filters,
    syncIntervalMinutes: 0,
    statusMapping: {},
    createdAt: '2026-09-03T00:00:00.000Z'
  }
  render(
    <ConnectionRow
      conn={conn}
      seededWorkflows={[]}
      missingEvents={[]}
      runningId={null}
      backfillingId={null}
      backfillResult={{}}
      onRun={vi.fn()}
      onBackfill={vi.fn()}
      onDelete={onDelete}
      onResetWorkflow={vi.fn()}
      onOpenWorkflow={vi.fn()}
      onRefresh={vi.fn()}
    />
  )
  const labelOf = (button: HTMLElement) =>
    button.closest('[data-tooltip]')?.getAttribute('data-tooltip') ?? ''
  const remove = screen.getAllByRole('button').find((button) => labelOf(button).includes('Remove'))
  return { onDelete, remove: remove as HTMLElement, labelOf }
}

describe('removing a connection that came with its connector', () => {
  it('is refused, and says what to remove instead', () => {
    const { onDelete, remove, labelOf } = renderRow({
      sdkConnectorId: 'echo-bench',
      implicit: true
    })
    expect(remove).toBeDisabled()
    expect(labelOf(remove)).toContain('Remove the pack instead')
    fireEvent.click(remove)
    expect(onDelete).not.toHaveBeenCalled()
  })

  it('is offered as usual for a connection somebody made', () => {
    const { onDelete, remove } = renderRow({ sdkConnectorId: 'echo-bench' })
    expect(remove).not.toBeDisabled()
    fireEvent.click(remove)
    expect(onDelete).toHaveBeenCalledWith('c1')
  })
})
