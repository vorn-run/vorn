import { useCallback, useEffect, useRef, useState } from 'react'
import type { AiAgentType } from '../../shared/types'
import type { AgentModelCatalog } from '@vornrun/shared/agent-models'

interface Request {
  agentType: string
  projectPath?: string
  remoteHostId?: string
}

const unavailable = (error: string): AgentModelCatalog => ({
  choices: [],
  status: 'unavailable',
  error
})

/** The models an agent can run in a project, asked once while wanted; `refresh` asks again. */
export function useAgentModelCatalog(request: Request, wanted: boolean) {
  const { agentType, projectPath = '', remoteHostId } = request
  const key = JSON.stringify([agentType, projectPath, remoteHostId ?? ''])
  const [fetched, setFetched] = useState<{ key: string; catalog: AgentModelCatalog } | null>(null)
  const [refreshing, setRefreshing] = useState(false)
  const turn = useRef(0)
  const have = fetched?.key === key

  const load = useCallback(
    async (refresh: boolean) => {
      const mine = ++turn.current
      const list = window.api?.listAgentModels
      let catalog: AgentModelCatalog
      try {
        catalog = list
          ? await list({ agentType: agentType as AiAgentType, projectPath, remoteHostId, refresh })
          : unavailable('This server cannot list models.')
      } catch {
        catalog = unavailable('Could not list models.')
      }
      await Promise.resolve()
      if (mine === turn.current) setFetched({ key, catalog })
    },
    [key, agentType, projectPath, remoteHostId]
  )

  useEffect(() => {
    if (wanted && !have) void load(false)
  }, [wanted, have, load])

  const refresh = () => {
    setRefreshing(true)
    void load(true).finally(() => setRefreshing(false))
  }

  return {
    catalog: have ? fetched.catalog : null,
    loading: (wanted && !have) || refreshing,
    refresh
  }
}
