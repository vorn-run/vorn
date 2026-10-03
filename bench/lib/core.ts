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

export const CORE_MODE = core.mode
export const nativeCore = core.native
