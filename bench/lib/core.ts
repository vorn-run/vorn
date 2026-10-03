/**
 * Which core this suite process runs, and a hard stop when `VORN_CORE=native`
 * was asked for but the binary did not load: the server falls back to JS
 * silently, which in a benchmark would report JS numbers under a native label.
 */
import { activeCore } from '../../packages/server/src/native-core'

const core = activeCore()
if (process.env.VORN_CORE === 'native' && !core.native) {
  throw new Error(`VORN_CORE=native but the core did not load: ${core.fallback}`)
}
// A core built without libghostty-vt has the Analyzer but no Screen, and would
// run the xterm screen model under a native label.
if (core.native && (!core.native.Screen || !core.native.Analyzer)) {
  throw new Error('VORN_CORE=native but the core was built without Screen or Analyzer')
}

export const CORE_MODE = core.mode
export const nativeCore = core.native
