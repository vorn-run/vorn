import { describe, it, expect } from 'vitest'
import {
  connectorSeededWorkflowId,
  connectorSeededWorkflowIdPrefix,
  parseConnectorWorkflowId
} from '../packages/shared/src/types'

describe('connector workflow id helpers', () => {
  it('connectorSeededWorkflowId assembles the stable three-segment id', () => {
    expect(connectorSeededWorkflowId('conn-1', 'issueCreated')).toBe(
      'vorn.connectors:conn-1:issueCreated'
    )
  })

  it('connectorSeededWorkflowIdPrefix returns the deletion-sweep prefix', () => {
    expect(connectorSeededWorkflowIdPrefix('conn-1')).toBe('vorn.connectors:conn-1:')
  })

  it('parseConnectorWorkflowId round-trips the assembled id', () => {
    const id = connectorSeededWorkflowId('abc-xyz', 'prOpened')
    expect(parseConnectorWorkflowId(id)).toEqual({
      connectionId: 'abc-xyz',
      event: 'prOpened'
    })
  })

  it('parseConnectorWorkflowId returns null for non-connector ids', () => {
    expect(parseConnectorWorkflowId('system:default-task-workflow')).toBeNull()
    expect(parseConnectorWorkflowId('manual-workflow')).toBeNull()
    expect(parseConnectorWorkflowId('')).toBeNull()
  })

  it('parseConnectorWorkflowId handles event strings that themselves contain colons', () => {
    expect(parseConnectorWorkflowId('vorn.connectors:conn-1:event:with:colons')).toEqual({
      connectionId: 'conn-1',
      event: 'event:with:colons'
    })
  })

  it('parseConnectorWorkflowId returns null when the second segment is missing', () => {
    expect(parseConnectorWorkflowId('vorn.connectors:only-one-segment')).toBeNull()
  })
})
