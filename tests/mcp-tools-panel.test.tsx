// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { McpToolsPanel } from '../src/renderer/components/settings/McpToolsPanel'
import type { SourceConnection } from '../src/shared/types'

const refreshMcpTools = vi.fn()
const executeConnectorAction = vi.fn()

beforeEach(() => {
  refreshMcpTools.mockReset().mockResolvedValue({ ok: true })
  executeConnectorAction.mockReset().mockResolvedValue({ success: true, output: { rows: 1 } })
  ;(window as unknown as { api: unknown }).api = { refreshMcpTools, executeConnectorAction }
})

const TOOLS = [
  { name: 'search', description: 'Search everything' },
  {
    name: 'run_query',
    inputSchema: {
      type: 'object',
      properties: {
        query: { type: 'string' },
        limit: { type: 'number', default: 50 },
        dryRun: { type: 'boolean' },
        tags: { type: 'array' },
        opts: { type: 'object' },
        odd: { type: 'nonsense' }
      }
    }
  }
]

const connection = (discoveredTools: unknown = TOOLS): SourceConnection =>
  ({
    id: 'c1',
    connectorId: 'mcp',
    name: 'A server',
    filters: { discoveredTools },
    syncIntervalMinutes: 5,
    statusMapping: {}
  }) as unknown as SourceConnection

function setup(conn = connection()) {
  const onRefresh = vi.fn()
  return { ...render(<McpToolsPanel connection={conn} onRefresh={onRefresh} />), onRefresh }
}

describe('the tools an MCP connection discovered', () => {
  it('counts them without being opened', () => {
    expect(setup().getByText('2 tools')).toBeInTheDocument()
  })

  it('does not say "1 tools"', () => {
    expect(setup(connection([TOOLS[0]])).getByText('1 tool')).toBeInTheDocument()
  })

  it('says so when discovery has not produced anything', () => {
    // The field is absent until the first tools/list completes.
    expect(setup(connection(null)).getByText('No tools discovered yet')).toBeInTheDocument()
  })

  it('is not fooled by a list that arrived as a JSON string', () => {
    // `'[{...}]'.length` is a number, so a string here would have been counted
    // as that many tools.
    expect(
      setup(connection('[{"name":"a"}]')).getByText('No tools discovered yet')
    ).toBeInTheDocument()
  })

  it('lists them, with what each one is for, once opened', () => {
    const { getByText } = setup()
    fireEvent.click(getByText('2 tools'))
    expect(getByText('search')).toBeInTheDocument()
    expect(getByText('Search everything')).toBeInTheDocument()
  })

  it('explains an empty list rather than showing a blank panel', () => {
    const { getByText } = setup(connection([]))
    fireEvent.click(getByText('No tools discovered yet'))
    expect(getByText(/Click refresh to run tools\/list/)).toBeInTheDocument()
  })

  it('re-runs discovery and pulls the result into the row', async () => {
    const { container, onRefresh } = setup()
    fireEvent.click(container.querySelectorAll('button')[1])

    await waitFor(() => expect(refreshMcpTools).toHaveBeenCalledWith('c1'))
    // Without this the row keeps rendering the tools it was mounted with.
    await waitFor(() => expect(onRefresh).toHaveBeenCalled())
  })

  it('reports a refusal from the server', async () => {
    refreshMcpTools.mockResolvedValue({ ok: false, error: 'server closed the connection' })
    const { container, findByText } = setup()
    fireEvent.click(container.querySelectorAll('button')[1])
    expect(await findByText('server closed the connection')).toBeInTheDocument()
  })

  it('reports a refresh that threw rather than leaving the spinner on', async () => {
    refreshMcpTools.mockRejectedValue(new Error('socket hang up'))
    const { container, findByText } = setup()
    fireEvent.click(container.querySelectorAll('button')[1])
    expect(await findByText('socket hang up')).toBeInTheDocument()
  })
})

describe('invoking a tool', () => {
  const open = () => {
    const utils = setup()
    fireEvent.click(utils.getByText('2 tools'))
    fireEvent.click(utils.getAllByText('Invoke')[1])
    return utils
  }

  it('starts from a skeleton built out of the tool schema, not an empty object', () => {
    // Typing the argument names by hand from a schema nobody has read is how a
    // first invocation fails on a typo.
    const { getByLabelText } = open()
    const args = JSON.parse((getByLabelText('Arguments (JSON)') as HTMLTextAreaElement).value)
    expect(args).toEqual({
      query: '',
      limit: 50,
      dryRun: false,
      tags: [],
      opts: {},
      odd: null
    })
  })

  it('offers nothing to fill in for a tool that takes nothing', () => {
    const utils = setup()
    fireEvent.click(utils.getByText('2 tools'))
    fireEvent.click(utils.getAllByText('Invoke')[0])
    expect((utils.getByLabelText('Arguments (JSON)') as HTMLTextAreaElement).value).toBe('{}')
  })

  it('runs the tool with what was typed and shows what came back', async () => {
    const { getByText, getByLabelText, findByText } = open()
    fireEvent.change(getByLabelText('Arguments (JSON)'), {
      target: { value: '{"query":"Alerts"}' }
    })
    fireEvent.click(getByText('Run'))

    await waitFor(() =>
      expect(executeConnectorAction).toHaveBeenCalledWith({
        connectionId: 'c1',
        action: 'run_query',
        args: { query: 'Alerts' }
      })
    )
    expect(await findByText('Success')).toBeInTheDocument()
  })

  it('names the syntax error rather than sending broken JSON', async () => {
    const { getByText, getByLabelText, findByText } = open()
    fireEvent.change(getByLabelText('Arguments (JSON)'), { target: { value: '{not json' } })
    fireEvent.click(getByText('Run'))

    expect(await findByText(/Invalid JSON/)).toBeInTheDocument()
    expect(executeConnectorAction).not.toHaveBeenCalled()
  })

  it('treats an empty box as no arguments rather than as a syntax error', async () => {
    const { getByText, getByLabelText } = open()
    fireEvent.change(getByLabelText('Arguments (JSON)'), { target: { value: '  ' } })
    fireEvent.click(getByText('Run'))
    await waitFor(() =>
      expect(executeConnectorAction).toHaveBeenCalledWith(expect.objectContaining({ args: {} }))
    )
  })

  it('shows what the tool failed with', async () => {
    executeConnectorAction.mockResolvedValue({ success: false, error: 'no such table' })
    const { getByText, findByText } = open()
    fireEvent.click(getByText('Run'))
    expect(await findByText('no such table')).toBeInTheDocument()
  })

  it('shows the schema it built the skeleton from, for when the skeleton is wrong', () => {
    const { getByText } = open()
    expect(getByText('inputSchema')).toBeInTheDocument()
  })

  it('closes', () => {
    const { getByText, queryByLabelText } = open()
    fireEvent.click(getByText('Close'))
    expect(queryByLabelText('Arguments (JSON)')).not.toBeInTheDocument()
  })
})
