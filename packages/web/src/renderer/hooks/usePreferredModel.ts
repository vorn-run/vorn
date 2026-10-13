import { useSyncExternalStore } from 'react'
import type { AiAgentType } from '../../shared/types'
import { getPreferredModel, setPreferredModel } from '../lib/launch-prefs'

function subscribe(onChange: () => void): () => void {
  window.addEventListener('vorn:launch-preferences', onChange)
  window.addEventListener('storage', onChange)
  return () => {
    window.removeEventListener('vorn:launch-preferences', onChange)
    window.removeEventListener('storage', onChange)
  }
}

export function usePreferredModel(agent: AiAgentType, host?: string) {
  const model = useSyncExternalStore(subscribe, () => getPreferredModel(agent, host))
  return { model, setModel: (value: string | undefined) => setPreferredModel(agent, value, host) }
}
