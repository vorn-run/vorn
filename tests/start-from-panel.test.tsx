// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { StartFromPanel } from '../src/renderer/components/workflow-editor/panels/StartFromPanel'
import { TEMPLATE_SEED } from '../packages/server/src/connectors/template-seed'
import type { SourceConnection } from '../packages/shared/src/types'

function connection(overrides: Partial<SourceConnection> = {}): SourceConnection {
  return {
    id: 'conn-1',
    name: 'reporting API',
    connectorId: 'http',
    filters: {},
    syncIntervalMinutes: 5,
    statusMapping: {},
    createdAt: '2026-09-01T00:00:00Z',
    ...overrides
  }
}

function panel(connections: SourceConnection[] = [], onPickTemplate = vi.fn()) {
  const onPickBlank = vi.fn()
  render(
    <StartFromPanel
      templates={TEMPLATE_SEED}
      connections={connections}
      onPickBlank={onPickBlank}
      onPickTemplate={onPickTemplate}
      onClose={vi.fn()}
    />
  )
  return { onPickBlank, onPickTemplate }
}

describe('StartFromPanel', () => {
  it('offers a blank canvas before any template', () => {
    panel()
    const options = screen.getAllByRole('button').map((b) => b.textContent)
    expect(options[1]).toContain('Blank canvas')
    expect(options.slice(2).join(' ')).toContain('Webhook to report')
  })

  it('names each template by the steps it draws', () => {
    panel()
    expect(screen.getByText('Webhook · Condition · HTTP request')).toBeInTheDocument()
  })

  it('says what a template still needs here', () => {
    panel()
    expect(screen.getByText(/Needs an HTTP profile like "reporting API"/)).toBeInTheDocument()
  })

  it('says a template is connected once this machine can answer it', () => {
    panel([connection()])
    expect(screen.queryByText(/Needs an HTTP profile/)).not.toBeInTheDocument()
    expect(screen.getByText('Connected')).toBeInTheDocument()
  })

  it('hands back the template that was picked', () => {
    const onPickTemplate = vi.fn()
    panel([], onPickTemplate)
    fireEvent.click(screen.getByText('Morning digest'))
    expect(onPickTemplate).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'morning-digest' })
    )
  })

  it('takes the blank canvas as an answer too', () => {
    const { onPickBlank } = panel()
    fireEvent.click(screen.getByText('Blank canvas'))
    expect(onPickBlank).toHaveBeenCalledTimes(1)
  })

  it('offers what a connection already knows how to build, and names it', () => {
    const onPickSuggestion = vi.fn()
    const suggestion = {
      key: 'c1:issueCreated',
      connectionId: 'c1',
      connectionName: 'workspace-eng',
      event: 'issueCreated',
      name: 'New issues to tasks'
    }
    render(
      <StartFromPanel
        templates={[]}
        connections={[]}
        suggestions={[suggestion]}
        onPickBlank={vi.fn()}
        onPickTemplate={vi.fn()}
        onPickSuggestion={onPickSuggestion}
        onClose={vi.fn()}
      />
    )

    expect(screen.getByText('From your connections')).toBeInTheDocument()
    expect(screen.getByText('workspace-eng')).toBeInTheDocument()
    fireEvent.click(screen.getByText('New issues to tasks'))
    expect(onPickSuggestion).toHaveBeenCalledWith(suggestion)
  })
})
