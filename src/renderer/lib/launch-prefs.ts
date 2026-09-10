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
  models?: Record<string, Partial<Record<AiAgentType, string>>>
}

export function loadLaunchSettings(): SavedLaunchSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return {}
    const parsed: unknown = JSON.parse(raw)
    // Valid JSON is not necessarily the shape we wrote — a corrupted or
    // older value could be a string or number, which would otherwise be
    // spread back into storage as indexed characters.
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) return {}
    return parsed as SavedLaunchSettings
  } catch {
    return {}
  }
}

export function persistLaunchSettings(settings: SavedLaunchSettings): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...loadLaunchSettings(), ...settings }))
    window.dispatchEvent?.(new Event('vorn:launch-preferences'))
  } catch {
    // Storage unavailable (private mode, quota). The preference is a
    // convenience, so losing it is not worth surfacing.
  }
}

export function getPreferredModel(agent: AiAgentType, host?: string): string | undefined {
  const value = loadLaunchSettings().models?.[host ?? 'local']?.[agent]
  return typeof value === 'string' ? value : undefined
}

export function setPreferredModel(
  agent: AiAgentType,
  model: string | undefined,
  host?: string
): void {
  const saved = loadLaunchSettings()
  const key = host ?? 'local'
  persistLaunchSettings({
    models: { ...saved.models, [key]: { ...saved.models?.[key], [agent]: model } }
  })
}

export function getPreferredAgent(fallback: AiAgentType = 'claude'): AiAgentType {
  return loadLaunchSettings().agent ?? fallback
}

export function setPreferredAgent(agent: AiAgentType): void {
  persistLaunchSettings({ ...loadLaunchSettings(), agent })
}
