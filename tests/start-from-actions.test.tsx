// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { StartFromPanel } from '../src/renderer/components/workflow-editor/panels/StartFromPanel'
import { TEMPLATE_SEED } from '../packages/server/src/connectors/template-seed'
import { RequirementRow } from '../src/renderer/components/workflow-editor/panels/RequirementRow'
import type { ConnectorListing } from '../src/renderer/lib/connector-browse'
import type { RequirementAction } from '../src/renderer/lib/template-requirements'

/** A published connector this machine has not installed. */
const LISTING: ConnectorListing = {
  key: 'catalog:slack',
  id: 'slack',
  name: 'Slack',
  capabilities: ['actions'],
  category: 'Chat',
  source: 'catalog',
  keywords: [],
  connectedCount: 0
}

/** The template that wants an HTTP profile nobody here has. */
const NEEDY = TEMPLATE_SEED.filter((t) => t.id === 'webhook-to-report')

function panel(listings: ConnectorListing[] = []) {
  const onFixRequirement = vi.fn<(action: RequirementAction) => void>()
  const onPickTemplate = vi.fn()
  render(
    <StartFromPanel
      templates={NEEDY}
      connections={[]}
      listings={listings}
      onPickBlank={vi.fn()}
      onPickTemplate={onPickTemplate}
      onFixRequirement={onFixRequirement}
      onClose={vi.fn()}
    />
  )
  return { onFixRequirement, onPickTemplate }
}

describe('a requirement that carries its own fix', () => {
  it('offers to make the profile a template asks for', () => {
    panel()
    expect(screen.getByRole('button', { name: 'Create profile' })).toBeInTheDocument()
  })

  it('asks for the profile by the name the template used', () => {
    const { onFixRequirement } = panel()
    fireEvent.click(screen.getByRole('button', { name: 'Create profile' }))
    expect(onFixRequirement).toHaveBeenCalledWith({
      kind: 'createProfile',
      name: 'reporting API'
    })
  })

  it('does not start the template just because its requirement was answered', () => {
    const { onPickTemplate } = panel()
    fireEvent.click(screen.getByRole('button', { name: 'Create profile' }))
    expect(onPickTemplate).not.toHaveBeenCalled()
  })

  it('still starts the template when the row itself is picked', () => {
    const { onPickTemplate, onFixRequirement } = panel()
    fireEvent.click(screen.getByText('Webhook to report'))
    expect(onPickTemplate).toHaveBeenCalledTimes(1)
    expect(onFixRequirement).not.toHaveBeenCalled()
  })

  // The row was one button before; splitting it must leave both halves real
  // buttons rather than a div someone can only reach with a mouse.
  it('keeps the pick and the fix separately reachable', () => {
    panel()
    const pick = screen.getByText('Webhook to report').closest('button')
    const fix = screen.getByRole('button', { name: 'Create profile' })
    expect(pick).not.toBeNull()
    expect(fix.tagName).toBe('BUTTON')
    expect(pick!.contains(fix)).toBe(false)
  })

  it('names the connector it would install', () => {
    render(
      <RequirementRow
        requirement={{
          requirement: { kind: 'connection', nodeId: 'n1', connectorId: 'slack', name: 'eng' }
        }}
        listings={[LISTING]}
        onFix={vi.fn()}
      />
    )
    expect(screen.getByRole('button', { name: 'Install Slack' })).toBeInTheDocument()
  })

  it('offers the connection instead once the pack is on disk', () => {
    const onFix = vi.fn()
    const installed = { ...LISTING, pack: { id: 'slack', name: 'Slack', version: '1.2.0' } }
    render(
      <RequirementRow
        requirement={{
          requirement: { kind: 'connection', nodeId: 'n1', connectorId: 'slack', name: 'eng' }
        }}
        listings={[installed as ConnectorListing]}
        onFix={onFix}
      />
    )
    fireEvent.click(screen.getByRole('button', { name: 'Add connection' }))
    expect(onFix).toHaveBeenCalledWith(expect.objectContaining({ kind: 'addConnection' }))
  })

  it('says nothing to do when no catalog here knows the connector', () => {
    const { onFixRequirement } = panel([])
    // The profile row is the only actionable one; nothing else offers a button.
    expect(screen.queryAllByRole('button', { name: /Install/ })).toHaveLength(0)
    expect(onFixRequirement).not.toHaveBeenCalled()
  })
})
