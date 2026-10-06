// An MCP server on stdio with a few tools, for tests that run a real MCP connection.
//
//   echo        returns its arguments as text, typed by the input schema
//   structured  declares an output schema and returns structured content
//   fails       answers with isError and a text block
//   secret      returns the FIXTURE_SECRET and FIXTURE_PLAIN environment variables
//   slow        waits `ms` milliseconds, then answers
import process from 'node:process'
import { setTimeout as sleep } from 'node:timers/promises'
import { Server } from '@modelcontextprotocol/sdk/server/index.js'
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
import { CallToolRequestSchema, ListToolsRequestSchema } from '@modelcontextprotocol/sdk/types.js'

const TOOLS = [
  {
    name: 'echo',
    title: 'Echo',
    description: 'Says back what it was given.',
    inputSchema: {
      type: 'object',
      properties: {
        text: { type: 'string', description: 'Anything' },
        count: { type: 'integer', default: 1 },
        ratio: { type: ['number', 'null'] },
        loud: { type: 'boolean' },
        tags: { type: 'array', items: { type: 'string' } },
        mode: { type: 'string', enum: ['a', 'b', 3] }
      },
      required: ['text']
    }
  },
  {
    name: 'structured',
    description: 'Answers with structured content.',
    inputSchema: { type: 'object', properties: {} },
    outputSchema: {
      type: 'object',
      properties: { total: { type: 'number' }, items: { type: 'array' } },
      required: ['total']
    }
  },
  {
    name: 'fails',
    description: '',
    inputSchema: { type: 'object' }
  },
  {
    name: 'secret',
    inputSchema: { type: 'object', properties: {} }
  },
  {
    name: 'slow',
    inputSchema: { type: 'object', properties: { ms: { type: 'number' } } }
  }
]

const server = new Server(
  { name: 'vorn-fixture', version: '1.0.0' },
  { capabilities: { tools: {} } }
)

server.setRequestHandler(ListToolsRequestSchema, async () => ({ tools: TOOLS }))

server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const args = request.params.arguments ?? {}
  switch (request.params.name) {
    case 'echo':
      return { content: [{ type: 'text', text: JSON.stringify(args) }] }
    case 'structured':
      return {
        content: [{ type: 'text', text: '{"total":2}' }],
        structuredContent: { total: 2, items: ['x', 'y'] }
      }
    case 'fails':
      return { content: [{ type: 'text', text: 'it went wrong' }], isError: true }
    case 'secret':
      return {
        content: [
          {
            type: 'text',
            text: JSON.stringify({
              secret: process.env.FIXTURE_SECRET ?? null,
              plain: process.env.FIXTURE_PLAIN ?? null
            })
          }
        ]
      }
    case 'slow':
      await sleep(Number(args.ms ?? 0))
      return { content: [{ type: 'text', text: 'done' }] }
    default:
      throw new Error(`no tool ${request.params.name}`)
  }
})

await server.connect(new StdioServerTransport())
