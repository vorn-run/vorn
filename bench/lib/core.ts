/**
 * The core every suite runs on, and a hard stop when it did not load: without
 * it the server has no screen model or analysis, and a suite would time a
 * no-op.
 */
import { activeCore } from '../../packages/server/src/native-core'

const core = activeCore()
if (!core.native) {
  throw new Error(`the vorn core did not load (run \`yarn build:core\`): ${core.error}`)
}
// A core built without libghostty-vt has the Analyzer but no screen model.
if (!core.native.TerminalPipeline || !core.native.Analyzer) {
  throw new Error('the vorn core was built without TerminalPipeline or Analyzer')
}

export const nativeCore = core.native
