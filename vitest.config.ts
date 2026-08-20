import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
import path from 'path'

export default defineConfig({
  plugins: [react()],
  define: {
    'process.env.NODE_ENV': JSON.stringify('test')
  },
  test: {
    globals: true,
    include: ['tests/**/*.test.ts', 'tests/**/*.test.tsx'],
    coverage: {
      provider: 'v8',
      reporter: ['text', 'text-summary', 'lcov'],
      include: [
        'packages/connector-sdk/src/**/*.ts',
        'packages/server/src/**/*.ts',
        'packages/shared/src/**/*.ts',
        'src/renderer/lib/**/*.ts',
        'src/renderer/components/**/*.tsx'
      ],
      exclude: [
        // Process entry point: starts the stdio server, nothing to assert.
        'packages/server/src/index.ts',
        'packages/server/src/register-methods.ts',
        'packages/server/src/logger.ts',
        'packages/server/src/hook-server.ts',
        'packages/server/src/hook-installer.ts',
        'packages/server/src/copilot-hook-installer.ts',
        'packages/server/src/task-images.ts',
        'src/renderer/lib/terminal-registry.ts',
        'src/renderer/lib/workflow-execution.ts',
        'src/renderer/lib/workflow-triggers.ts',
        'src/renderer/lib/terminal-close.ts',
        // Heavy settings page dominated by IPC-backed form state —
        // matches the established exclusion pattern for this category.
        'src/renderer/components/settings/ConnectorSettings.tsx',
        '**/*.d.ts'
      ]
    }
  },
  resolve: {
    alias: {
      '@shared': path.resolve(__dirname, 'src/shared'),
      '@main': path.resolve(__dirname, 'src/main'),
      '@renderer': path.resolve(__dirname, 'src/renderer'),
      // Connector packages depend on the published SDK. Tests must exercise
      // the SDK source, not whatever `dist` a previous build left behind.
      '@vornrun/connector-sdk': path.resolve(__dirname, 'packages/connector-sdk/src/index.ts')
    }
  }
})
