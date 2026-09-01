import { describe, expect, it } from 'vitest'
import type { ConnectorItemContext } from '../src/shared/types'
import {
  schedulerExecutionContext,
  webhookTriggerFromItem
} from '../src/renderer/lib/workflow-helpers'

const webhookItem: ConnectorItemContext = {
  connectionId: 'webhook',
  connectorId: 'webhook',
  externalId: 'evt-1',
  title: 'Webhook POST',
  raw: { body: { n: 1 }, headers: { 'x-event': 'deploy' }, query: { ref: 'main' }, method: 'POST' }
}

const pollItem: ConnectorItemContext = {
  connectionId: 'conn-1',
  connectorId: 'github',
  externalId: '42',
  title: 'An issue',
  raw: { number: 42 }
}

describe('schedulerExecutionContext', () => {
  it('lifts a webhook event into the trigger namespace and keeps the connector item', () => {
    const context = schedulerExecutionContext(webhookItem, undefined)
    expect(context?.trigger).toEqual({
      type: 'webhook',
      body: { n: 1 },
      headers: { 'x-event': 'deploy' },
      query: { ref: 'main' },
      method: 'POST'
    })
    expect(context?.connectorItem).toBe(webhookItem)
  })

  it('leaves connector poll items alone', () => {
    const context = schedulerExecutionContext(pollItem, undefined)
    expect(context?.trigger).toBeUndefined()
    expect(context?.connectorItem).toBe(pollItem)
  })

  it('returns undefined with nothing to carry', () => {
    expect(schedulerExecutionContext(undefined, undefined)).toBeUndefined()
  })

  it('carries manual-run inputs through', () => {
    const context = schedulerExecutionContext(undefined, { name: 'a' })
    expect(context?.inputs).toEqual({ name: 'a' })
  })
})

describe('webhookTriggerFromItem', () => {
  it('rebuilds the trigger namespace from a stored webhook item', () => {
    expect(webhookTriggerFromItem(webhookItem)).toMatchObject({
      type: 'webhook',
      body: { n: 1 },
      query: { ref: 'main' }
    })
  })

  it('returns nothing for a connector poll item', () => {
    expect(webhookTriggerFromItem(pollItem)).toBeUndefined()
  })
})
