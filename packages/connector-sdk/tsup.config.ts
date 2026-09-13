import { readFileSync } from 'node:fs'
import { defineConfig } from 'tsup'

const { version } = JSON.parse(
  readFileSync(new URL('./package.json', import.meta.url), 'utf8')
) as {
  version: string
}

export default defineConfig({
  entry: ['src/index.ts', 'src/cli.ts'],
  format: ['esm'],
  target: 'node20',
  dts: true,
  clean: true,
  // Connector authors install these themselves; a pack bundles what it needs.
  external: ['esbuild', 'tar'],
  // What a connector tells Vorn it was built with, in its hello.
  define: { __VORN_SDK_VERSION__: JSON.stringify(version) }
})
