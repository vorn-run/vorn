import type { AiAgentType } from './types'

export interface AgentModelChoice {
  id: string
  label: string
  description?: string
}

export interface AgentModelRequest {
  agentType: AiAgentType
  projectPath: string
  remoteHostId?: string
  refresh?: boolean
}

export interface AgentModelCatalog {
  choices: AgentModelChoice[]
  status: 'ready' | 'stale' | 'unavailable'
  /** Where a ready or stale list came from, for the menu's footer. */
  source?: 'agent' | 'built-in'
  fetchedAt?: number
  error?: string
}

const MODEL_AGENTS: readonly string[] = ['claude', 'copilot', 'opencode', 'codex']

export function supportsModelSelection(agent: string): boolean {
  return MODEL_AGENTS.includes(agent)
}

/** Copilot's CLI takes `--model` but cannot list models, so the menu offers these. */
export const CURATED_MODELS: Partial<Record<AiAgentType, AgentModelChoice[]>> = {
  copilot: [
    { id: 'auto', label: 'Auto', description: 'Let Copilot pick' },
    { id: 'gpt-5.4', label: 'GPT-5.4' },
    { id: 'gpt-5-mini', label: 'GPT-5 mini' },
    { id: 'claude-sonnet-4.5', label: 'Claude Sonnet 4.5' },
    { id: 'claude-opus-4.5', label: 'Claude Opus 4.5' }
  ]
}

/** A model id goes on a command line as one argument, so it must read as one. */
export function validateModelId(value: string): string {
  const model = value.trim()
  const unprintable = [...model].some((c) => c.charCodeAt(0) < 32 || c.charCodeAt(0) === 127)
  if (!model || model.startsWith('-') || /\s/.test(model) || unprintable) {
    throw new Error('Enter a model id without spaces, control characters, or a leading dash.')
  }
  return model
}
