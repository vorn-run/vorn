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

  it('still requires what is required, and drops what was left blank or null', async () => {
    const inputs: ActionInputField[] = [
      { key: 'id', label: 'Id', required: true },
      { key: 'note', label: 'Note' }
    ]
    await expect(take(inputs, { id: '' })).rejects.toThrow(/requires "id"/)
    await expect(take(inputs, { id: null })).rejects.toThrow(/requires "id"/)
    expect(await take(inputs, { id: '1', note: '' })).toEqual({ id: '1' })
    expect(await take(inputs, { id: '1', note: null })).toEqual({ id: '1' })
  })

  it('takes a value that already has its type as it is', async () => {
    const output = await take(
      [
        { key: 'count', label: 'Count', type: 'number' },
        { key: 'draft', label: 'Draft', type: 'boolean' },
        { key: 'body', label: 'Body', type: 'json' }
      ],
      { count: 7, draft: true, body: { a: [1, 2] } }
    )
    expect(output).toEqual({ count: 7, draft: true, body: { a: [1, 2] } })
  })

  it('reads a number or a flag handed to a text field as that text', async () => {
    const inputs: ActionInputField[] = [
      { key: 'id', label: 'Id' },
      { key: 'level', label: 'Level', type: 'select', options: [{ value: '1' }] }
    ]
    expect(await take(inputs, { id: 42, level: true })).toEqual({ id: '42', level: 'true' })
  })

  it('names the field whose value is not the type it declares', async () => {
    const inputs: ActionInputField[] = [
      { key: 'count', label: 'Count', type: 'number' },
      { key: 'draft', label: 'Draft', type: 'boolean' }
    ]
    await expect(take(inputs, { count: true })).rejects.toMatchObject({
      name: 'ActionArgumentError',
      field: 'count',
      message: 'Action take argument "count": Expected a number, got true'
    })
    await expect(take(inputs, { draft: 1 })).rejects.toMatchObject({
      field: 'draft',
      message: 'Action take argument "draft": Expected a boolean, got 1'
    })
    await expect(take(inputs, { count: 'many' })).rejects.toThrow('Expected a number, got "many"')
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
