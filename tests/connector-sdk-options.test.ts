import { describe, expect, it } from 'vitest'
import { connectorManifest, defineConnector, runOptions } from '../packages/connector-sdk/src/index'
import type { ActionDefinition, Connector } from '../packages/connector-sdk/src/types'
import { greeted } from './helpers/connector-server'

const withOptions = (extra: Partial<ActionDefinition> = {}): Connector =>
  defineConnector({
    id: 'acme',
    name: 'Acme',
    options: {
      channels: () => [
        { value: 'C1', label: 'general' },
        { value: 'C2', label: 'random' }
      ],
      // A bare string is the common case: the value is also what to show.
      levels: () => ['high', 'low']
    },
    actions: [
      {
        type: 'post',
        label: 'Post',
        inputs: [
          { key: 'channel', label: 'Channel', type: 'select', loadOptions: 'channels' },
          {
            key: 'level',
            label: 'Level',
            type: 'select',
            required: true,
            options: [{ value: 'high' }, { value: 'low', label: 'Low priority' }]
          }
        ],
        run: () => ({ ok: true }),
        ...extra
      } as ActionDefinition
    ]
  })

describe('choices a connection has to be asked for', () => {
  it('lists a set, taking a bare string as a choice that shows itself', async () => {
    expect(await runOptions(withOptions(), 'channels')).toEqual([
      { value: 'C1', label: 'general' },
      { value: 'C2', label: 'random' }
    ])
    expect(await runOptions(withOptions(), 'levels')).toEqual([{ value: 'high' }, { value: 'low' }])
  })

  it('says so when nothing serves the set that was asked for', async () => {
    await expect(runOptions(withOptions(), 'nope')).rejects.toThrow(/serves no options set "nope"/)
  })

  it('refuses an input pointing at a set the connector does not serve', () => {
    expect(() =>
      defineConnector({
        id: 'acme',
        name: 'Acme',
        options: { channels: () => [] },
        actions: [
          {
            type: 'post',
            label: 'Post',
            inputs: [{ key: 'c', label: 'C', type: 'select', loadOptions: 'chanels' }],
            run: () => ({})
          }
        ]
      })
    ).toThrow(/loads options from "chanels", which the connector does not serve/)
  })

  it('serves the set over the protocol, and refuses a set it does not have', async () => {
    const server = await greeted(withOptions())
    expect(await server.call('connector/options', { name: 'channels' })).toEqual({
      options: [
        { value: 'C1', label: 'general' },
        { value: 'C2', label: 'random' }
      ]
    })
    expect(await server.fail('connector/options', { name: 'nope' })).toEqual({
      code: -32602,
      message: 'acme serves no options set "nope"'
    })

    const plain = await greeted(
      defineConnector({
        id: 'plain',
        name: 'Plain',
        actions: [{ type: 'go', label: 'Go', run: () => ({}) }]
      })
    )
    expect(await plain.fail('connector/options', { name: 'channels' })).toMatchObject({
      code: -32602
    })
  })
})

describe('what the manifest says about an argument', () => {
  it('carries fixed choices, and names the set a dynamic field loads from', () => {
    const [channel, level] = connectorManifest(withOptions()).actions[0].inputs
    expect(level.options).toEqual([{ value: 'high' }, { value: 'low', label: 'Low priority' }])
    expect(channel).toMatchObject({ type: 'select', loadOptions: 'channels' })
    expect(channel).not.toHaveProperty('options')
  })

  // A step may compute the value, so the choices suggest rather than refuse.
  it('accepts a value the list does not hold, and lets the connector judge it', async () => {
    const server = await greeted(withOptions())
    expect(
      await server.call('action/run', {
        action: 'post',
        args: { level: 'computed-elsewhere', channel: 'C1' }
      })
    ).toEqual({ ok: true })
  })
})
