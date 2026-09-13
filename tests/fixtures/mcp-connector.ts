// A connector built and served with the SDK as it ships today, which speaks only MCP.
import { defineConnector, serveConnector } from '../../packages/connector-sdk/src/index'

const connector = defineConnector({
  id: 'mcp-fixture',
  name: 'MCP fixture',
  version: '1.0.0',
  triggers: [
    {
      type: 'tick',
      label: 'Tick',
      poll: (context) => ({
        items: [
          {
            externalId: 'tick-1',
            title: 'Tick',
            updatedAt: '2026-09-13T00:00:00.000Z',
            data: { limit: context.limit ?? null }
          }
        ]
      })
    }
  ],
  actions: [
    {
      type: 'echo',
      label: 'Echo',
      inputs: [
        { key: 'text', label: 'Text', required: true },
        { key: 'count', label: 'Count', type: 'number' }
      ],
      run: (args) => ({ text: args.text, count: args.count })
    }
  ],
  preflight: () => ({ ok: true, message: 'ready' })
})

void serveConnector(connector, { config: {} })
