import type { AiAgentType } from '../../shared/types'

/**
 * Last-used launch settings, shared by the prompt launcher and the shell
 * card's input bar. Both write the same key so the agent you picked in one
 * place is the agent the other offers.
 */

const STORAGE_KEY = 'vorn:lastLaunchSettings'

export interface SavedLaunchSettings {
  project?: string
  agent?: AiAgentType
}

export function loadLaunchSettings(): SavedLaunchSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    return raw ? JSON.parse(raw) : {}
  } catch {
    return {}
  }
}

export function persistLaunchSettings(settings: SavedLaunchSettings): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(settings))
  } catch {
    // Storage unavailable (private mode, quota). The preference is a
    // convenience, so losing it is not worth surfacing.
  }
}

export function getPreferredAgent(fallback: AiAgentType = 'claude'): AiAgentType {
  return loadLaunchSettings().agent ?? fallback
}

export function setPreferredAgent(agent: AiAgentType): void {
  persistLaunchSettings({ ...loadLaunchSettings(), agent })
}
