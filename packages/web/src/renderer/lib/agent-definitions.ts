import { AiAgentType } from '../../shared/types'

export interface AgentDefinition {
  type: AiAgentType
  displayName: string
  color: string
  bgColor: string
  description: string
  installCommand: string
  installUrl: string
}

export const AGENT_DEFINITIONS: Record<AiAgentType, AgentDefinition> = {
  claude: {
    type: 'claude',
    displayName: 'Claude Code',
    color: '#D97757',
    bgColor: '#D9775720',
    description: 'Anthropic Claude Code CLI',
    installCommand: 'npm install -g @anthropic-ai/claude-code',
    installUrl: 'https://docs.anthropic.com/en/docs/claude-code'
  },
  copilot: {
    type: 'copilot',
    displayName: 'GitHub Copilot',
    color: '#6CC644',
    bgColor: '#6CC64420',
    description: 'GitHub Copilot CLI',
    installCommand: 'brew install copilot-cli',
    installUrl:
      'https://docs.github.com/en/copilot/using-github-copilot/using-github-copilot-agent-mode'
  },
  codex: {
    type: 'codex',
    displayName: 'Codex CLI',
    color: '#7A9DFF',
    bgColor: '#7A9DFF20',
    description: 'OpenAI Codex CLI',
    installCommand: 'npm install -g @openai/codex',
    installUrl: 'https://github.com/openai/codex'
  },
  opencode: {
    type: 'opencode',
    displayName: 'OpenCode',
    color: '#FFFFFF',
    bgColor: '#FFFFFF15',
    description: 'OpenCode CLI',
    installCommand: 'npm install -g opencode-ai',
    installUrl: 'https://opencode.ai'
  },
  gemini: {
    type: 'gemini',
    displayName: 'Gemini CLI',
    color: '#3186FF',
    bgColor: '#3186FF20',
    description: 'Google Gemini CLI',
    installCommand: 'npm install -g @google/gemini-cli',
    installUrl: 'https://github.com/google-gemini/gemini-cli'
  }
}

export const AGENT_LIST = Object.values(AGENT_DEFINITIONS)
