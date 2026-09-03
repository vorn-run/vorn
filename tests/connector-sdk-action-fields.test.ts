import { describe, expect, it } from 'vitest'
import { connectorManifest, defineConnector } from '../packages/connector-sdk/src/index'
import type { ActionDefinition } from '../packages/connector-sdk/src/types'

const manifestFor = (action: Partial<ActionDefinition>) =>
  connectorManifest(
    defineConnector({
      id: 'acme',
      name: 'Acme',
      options: { channels: () => [] },
      actions: [{ type: 'post', label: 'Post', run: () => ({}), ...action } as ActionDefinition]
    })
  ).actions[0]

describe('the arguments an action declares', () => {
  it('still default to a required-less string, as they always did', () => {
    const inputs = manifestFor({ inputs: [{ key: 'text', label: 'Text' }] }).inputs
    expect(inputs).toEqual([{ key: 'text', label: 'Text', type: 'string', required: false }])
  })

  it('carry the kinds a field has to be drawn as', () => {
    for (const type of ['string', 'number', 'boolean', 'select', 'json'] as const) {
      const inputs = manifestFor({ inputs: [{ key: 'v', label: 'V', type }] }).inputs
      expect(inputs[0].type).toBe(type)
    }
  })

  it('carry fixed choices, so a select can be drawn without a connection', () => {
    const options = [{ value: 'high' }, { value: 'low', label: 'Low priority' }]
    const inputs = manifestFor({
      inputs: [{ key: 'level', label: 'Level', type: 'select', options }]
    }).inputs
    expect(inputs[0].options).toEqual(options)
    expect(inputs[0].loadOptions).toBeUndefined()
  })

  it('name an options set when the choices need a live connection to know', () => {
    const inputs = manifestFor({
      inputs: [{ key: 'channel', label: 'Channel', type: 'select', loadOptions: 'channels' }]
    }).inputs
    expect(inputs[0].loadOptions).toBe('channels')
  })
})

describe('the fields an action returns', () => {
  it('reach the manifest, so the variable picker can offer them', () => {
    const outputs = [{ key: 'id', type: 'string' as const, description: 'The message id' }]
    expect(manifestFor({ outputs }).outputs).toEqual(outputs)
  })

  it('stay absent when the connector declared none, which is not "returns nothing"', () => {
    expect(manifestFor({}).outputs).toBeUndefined()
  })
})
