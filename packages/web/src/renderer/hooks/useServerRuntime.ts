import { useEffect, useState } from 'react'
import type { ServerRuntimeStatus } from '../../shared/types'

/** Followed, not fetched once: the automatic move happens long before a panel opens. */
export function useServerRuntime(): ServerRuntimeStatus | null {
  // Read during the first render so nothing renders twice; guarded for a renderer reloaded against an older `window.api`.
  const [runtime, setRuntime] = useState<ServerRuntimeStatus | null>(() =>
    typeof window.api?.getServerRuntimeStatus === 'function'
      ? window.api.getServerRuntimeStatus()
      : null
  )
  useEffect(() => window.api?.onServerRuntimeStatus?.(setRuntime), [])
  return runtime
}
