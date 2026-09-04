import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js'
const [tool, argsJson] = process.argv.slice(2)
const W = process.env.VORN_WORKTREE
const transport = new StdioClientTransport({
  command: 'npx',
  args: ['tsx', `${W}/packages/mcp/src/index.ts`],
  cwd: W,
  env: { ...process.env, VORN_DATA_DIR: `${process.env.HOME}/.vorn-devhome/.vorn` }
})
const client = new Client({ name: 'factory-driver', version: '0.1.0' })
await client.connect(transport)
if (tool === 'tools') {
  const { tools } = await client.listTools()
  console.log(tools.map((t) => t.name).join('\n'))
} else {
  const result = await client.callTool({ name: tool, arguments: argsJson ? JSON.parse(argsJson) : {} })
  for (const c of result.content ?? []) console.log(c.type === 'text' ? c.text : JSON.stringify(c))
  if (result.isError) process.exitCode = 1
}
await client.close()
