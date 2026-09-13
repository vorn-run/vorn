// A connector built and served with the SDK, exactly as a connector's bin runs it.
import { defineConnector, serveConnector } from '../../packages/connector-sdk/src/index'

const connector = defineConnector({
  id: 'sdk-fixture',
  name: 'SDK fixture',
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
      run: (args) => {
        console.log(`echoing ${String(args.text)}`)
        return { text: args.text, count: args.count }
      }
    },
    {
      type: 'wait',
      label: 'Wait',
      inputs: [{ key: 'ms', label: 'Milliseconds', type: 'number', required: true }],
      run: async (args) => {
        await new Promise((resolve) => setTimeout(resolve, Number(args.ms)))
        return { waited: args.ms }
      }
    }
  ],
  preflight: () => ({ ok: true, message: 'ready' })
})

void serveConnector(connector, { config: {} })
console.log('booting')
