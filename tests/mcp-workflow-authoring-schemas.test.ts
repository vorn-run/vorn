import { describe, expect, it } from 'vitest'
import { triggerConfigSchema, nodeSchema } from '../packages/mcp/src/tools/workflows'

describe('MCP workflow authoring schemas', () => {
  it('accepts a webhook trigger', () => {
    const parsed = triggerConfigSchema.parse({
      triggerType: 'webhook',
      method: 'POST',
      token: 'abc123'
    })
    expect(parsed).toMatchObject({ triggerType: 'webhook', method: 'POST' })
  })

  it('rejects a webhook trigger with an unsupported method', () => {
    expect(() =>
      triggerConfigSchema.parse({ triggerType: 'webhook', method: 'DELETE', token: 'abc123' })
    ).toThrow()
  })

  it('accepts a connector poll trigger', () => {
    const parsed = triggerConfigSchema.parse({
      triggerType: 'connectorPoll',
      connectionId: 'conn-1',
      event: 'issueCreated',
      cron: '*/5 * * * *'
    })
    expect(parsed).toMatchObject({ triggerType: 'connectorPoll', event: 'issueCreated' })
  })

  it('accepts an httpRequest node', () => {
    const parsed = nodeSchema.parse({
      id: 'n1',
      type: 'httpRequest',
      label: 'Call API',
      slug: 'call-api',
      config: {
        nodeType: 'httpRequest',
        method: 'GET',
        url: 'https://x.test',
        headers: {},
        body: ''
      },
      position: { x: 0, y: 0 }
    })
    expect(parsed.type).toBe('httpRequest')
  })
})
