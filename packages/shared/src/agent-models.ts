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
  fetchedAt?: number
  error?: string
}

const MODEL_AGENTS: readonly string[] = ['claude', 'copilot', 'opencode', 'codex']

export function supportsModelSelection(agent: string): boolean {
  return MODEL_AGENTS.includes(agent)
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
