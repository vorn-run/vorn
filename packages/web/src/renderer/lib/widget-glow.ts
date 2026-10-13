import type { AiAgentType, AgentType } from '../../shared/types'

const GLOW_COLORS: Record<AiAgentType, string> = {
  claude: 'rgba(217, 119, 87, 0.45)',
  copilot: 'rgba(255, 255, 255, 0.3)',
  codex: 'rgba(122, 157, 255, 0.45)',
  opencode: 'rgba(255, 255, 255, 0.3)',
  gemini: 'rgba(49, 134, 255, 0.45)'
}
const SHELL_GLOW = 'rgba(156, 163, 175, 0.35)'

/** Shells use a neutral grey so they don't borrow AI-agent brand colors. */
export function glowColorForAgent(agentType: AgentType): string {
  return agentType === 'shell' ? SHELL_GLOW : GLOW_COLORS[agentType]
}
