import { useState, useEffect, useCallback } from 'react'
import { AiAgentType } from '../../shared/types'

export type AgentInstallStatus = Record<AiAgentType, boolean>

const DEFAULT_STATUS: AgentInstallStatus = {
  claude: true,
  copilot: true,
  codex: true,
  opencode: true,
  gemini: true
}

let globalStatus: AgentInstallStatus | null = null
const listeners: Set<() => void> = new Set()

function notifyListeners(): void {
  listeners.forEach((fn) => fn())
}

export function useAgentInstallStatus(): {
  status: AgentInstallStatus
  loading: boolean
  refresh: () => Promise<void>
} {
  const [status, setStatus] = useState<AgentInstallStatus>(globalStatus || DEFAULT_STATUS)
  const [loading, setLoading] = useState(globalStatus === null)

  const refresh = useCallback(async () => {
    setLoading(true)
    try {
      const result = await window.api.detectInstalledAgents()
      globalStatus = result
      setStatus(result)
      notifyListeners()
    } catch (err) {
      console.error('[useAgentInstallStatus] detection failed:', err)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    // Subscribe to updates from other hook instances
    const listener = (): void => {
      if (globalStatus) setStatus(globalStatus)
    }
    listeners.add(listener)

    // Only fetch on first mount if not yet cached
    if (globalStatus === null) {
      refresh()
    }

    return () => {
      listeners.delete(listener)
    }
  }, [refresh])

  return { status, loading, refresh }
}
