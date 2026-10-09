import { readFileSync } from 'node:fs'
import { defineConfig } from 'tsup'

const { version } = JSON.parse(readFileSync('./package.json', 'utf-8'))

export default defineConfig({
  entry: ['src/index.ts'],
  format: ['esm'],
  target: 'node20',
  clean: true,
  define: {
    __MCP_VERSION__: JSON.stringify(version)
  },
  // Bundle workspace packages so the npm package is self-contained
  noExternal: [/@vorn\//],
  // CJS modules must remain external
  external: ['ws']
})
