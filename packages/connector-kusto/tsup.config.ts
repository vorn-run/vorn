import { defineConfig } from 'tsup'

export default defineConfig({
  entry: ['src/index.ts'],
  format: ['esm'],
  target: 'node20',
  dts: true,
  clean: true,
  banner: { js: '#!/usr/bin/env node' },
  // Installed alongside the connector rather than inlined: `@azure/identity`
  // reads ambient credential state and must be a single instance, and the SDK
  // and MCP runtime would otherwise be duplicated in every connector package.
  external: ['@azure/identity', '@modelcontextprotocol/sdk', '@vornrun/connector-sdk', 'zod']
})
