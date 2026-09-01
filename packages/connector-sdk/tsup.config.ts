import { defineConfig } from 'tsup'

export default defineConfig({
  entry: ['src/index.ts', 'src/cli.ts'],
  format: ['esm'],
  target: 'node20',
  dts: true,
  clean: true,
  // Connector authors install these themselves; bundling them would ship two
  // copies of the MCP runtime into every connector package.
  external: ['@modelcontextprotocol/sdk', 'zod', 'esbuild', 'tar']
})
