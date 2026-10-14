import { defineConfig } from 'vitest/config'
import path from 'path'

export default defineConfig({
  define: {
    'process.env.NODE_ENV': JSON.stringify('test')
  },
  test: {
    globals: true,
    include: ['tests/**/*.test.ts'],
    globalSetup: ['tests/setup/global-sandbox.ts'],
    setupFiles: ['tests/setup/sandbox.ts'],
    coverage: {
      provider: 'v8',
      reporter: ['text', 'text-summary', 'lcov'],
      include: [
        'packages/connector-sdk/src/**/*.ts',
        'packages/mcp/src/**/*.ts',
        'packages/shared/src/**/*.ts'
      ],
      exclude: ['**/*.d.ts']
    }
  },
  resolve: {
    alias: {
      // Connector packages depend on the published SDK. Tests must exercise
      // the SDK source, not whatever `dist` a previous build left behind.
      '@vornrun/connector-sdk': path.resolve(__dirname, 'packages/connector-sdk/src/index.ts'),
      // Pinned to this checkout; in a worktree the bare specifier would resolve through the main clone.
      '@vornrun/shared/': `${path.resolve(__dirname, 'packages/shared/src')}/`
    }
  }
})
