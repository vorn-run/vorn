import { describe, expect, it } from 'vitest'
import { connectorManifest, defineConnector, runAction } from '../packages/connector-sdk/src/index'
import type { ActionInputField, Connector } from '../packages/connector-sdk/src/types'

/** Hands back exactly what `runAction` passed the action, after coercion. */
const echo = (inputs: ActionInputField[]): Connector =>
  defineConnector({
    id: 'acme',
    name: 'Acme',
    actions: [{ type: 'take', label: 'Take', inputs, run: (args) => ({ ...args }) }]
  })

const take = (inputs: ActionInputField[], args: Record<string, unknown>) =>
  runAction(echo(inputs), 'take', args)

describe('the kinds of value an argument arrives as', () => {
  it('parses a number and a boolean out of the text a template rendered', async () => {
    const output = await take(
      [
        { key: 'count', label: 'Count', type: 'number' },
        { key: 'draft', label: 'Draft', type: 'boolean' }
      ],
      { count: '7', draft: 'false' }
    )
    expect(output).toEqual({ count: 7, draft: false })
  })

  it('parses a json argument, so a step can send a structured value', async () => {
    const output = await take([{ key: 'body', label: 'Body', type: 'json' }], {
      body: '{"a":[1,2]}'
    })
    expect(output).toEqual({ body: { a: [1, 2] } })
  })

  it('says what it wanted when the json will not parse, quoting only a little', async () => {
    await expect(
      take([{ key: 'body', label: 'Body', type: 'json' }], { body: `{"a":${'9'.repeat(200)}` })
    ).rejects.toThrow(/Expected JSON, got "\{"a":9+…"/)
  })

  it('leaves a select as the string it already is', async () => {
    const output = await take(
      [{ key: 'level', label: 'Level', type: 'select', options: [{ value: 'high' }] }],
      { level: 'high' }
    )
    expect(output).toEqual({ level: 'high' })
  })

  it('still requires what is required, and drops what was left blank', async () => {
    const inputs: ActionInputField[] = [
      { key: 'id', label: 'Id', required: true },
      { key: 'note', label: 'Note' }
    ]
    await expect(take(inputs, { id: '' })).rejects.toThrow(/requires "id"/)
    expect(await take(inputs, { id: '1', note: '' })).toEqual({ id: '1' })
  })
})

describe('a note left for whoever builds the next connector', () => {
  it('reaches the manifest from a config field and from an argument', () => {
    const manifest = connectorManifest(
      defineConnector({
        id: 'acme',
        name: 'Acme',
        config: [
          {
            key: 'token',
            label: 'Token',
            builderHint: 'Create one under Settings → Developer, scope read:items'
          }
        ],
        actions: [
          {
            type: 'take',
            label: 'Take',
            inputs: [{ key: 'id', label: 'Id', builderHint: 'The numeric id, not the slug' }],
            run: () => ({})
          }
        ],
        triggers: [
          {
            type: 'made',
            label: 'Made',
            dedupe: 'timestamp',
            fetch: () => []
          }
        ]
      })
    )

    expect(manifest.actions[0].inputs[0].builderHint).toBe('The numeric id, not the slug')
    expect(manifest.triggers[0].setup.env[0].builderHint).toContain('Settings → Developer')
  })
})
