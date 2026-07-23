import { execFileSync } from 'node:child_process'
import {
  AiAgentType,
  AgentCommandConfig,
  CreateTerminalPayload,
  supportsExactSessionResume,
  supportsSessionIdPinning,
  getSessionIdPinningFlag
} from '@vornrun/shared/types'
import { DEFAULT_AGENT_COMMANDS } from '@vornrun/shared/agent-defaults'
import { shellEscape } from './process-utils'

function commandExists(cmd: string, env: Record<string, string>): boolean {
  try {
    const bin = process.platform === 'win32' ? 'where' : 'which'
    execFileSync(bin, [cmd], { stdio: 'pipe', timeout: 3000, env })
    return true
  } catch {
    return false
  }
}

function resolveAgentCommand(
  config: AgentCommandConfig,
  env: Record<string, string>
): { command: string; args: string[] } {
  if (commandExists(config.command, env)) {
    return { command: config.command, args: config.args }
  }
  if (config.fallbackCommand && commandExists(config.fallbackCommand, env)) {
    return { command: config.fallbackCommand, args: config.fallbackArgs ?? [] }
  }
  return { command: config.command, args: config.args }
}

/**
 * Resolve extra args with priority: per-step override > headlessArgs > base args.
 */
function resolveHeadlessArgs(
  payload: CreateTerminalPayload,
  cmdConfig: AgentCommandConfig,
  baseArgs: string[]
): string[] {
  if (payload.args !== undefined) return payload.args
  if (cmdConfig.headlessArgs !== undefined) return cmdConfig.headlessArgs
  return baseArgs
}

/**
 * Builds the interactive launch command (for PTY/terminal sessions).
 * This starts the agent's TUI/interactive mode.
 */
export function buildAgentLaunchLine(
  payload: CreateTerminalPayload,
  agentCommands: Record<AiAgentType, AgentCommandConfig>,
  env: Record<string, string>
): string {
  if (payload.agentType === 'shell') {
    throw new Error('buildAgentLaunchLine called for shell session — use createShellPty instead')
  }
  const cmdConfig = agentCommands[payload.agentType] || DEFAULT_AGENT_COMMANDS[payload.agentType]
  const cmd = resolveAgentCommand(cmdConfig, env)
  // Per-step args override settings-level args; escape each for shell safety
  const effectiveArgs = payload.args !== undefined ? payload.args : cmd.args
  let launchLine = [cmd.command, ...effectiveArgs.map((a) => shellEscape(a))].join(' ')

  if (payload.resumeSessionId && supportsExactSessionResume(payload.agentType)) {
    const escapedResumeId = shellEscape(payload.resumeSessionId)
    switch (payload.agentType) {
      case 'claude':
        launchLine += ` --resume ${escapedResumeId}`
        break
      case 'copilot':
        launchLine += ` --resume ${escapedResumeId}`
        break
      case 'codex':
        launchLine = `${cmd.command} resume ${escapedResumeId}`
        break
      case 'opencode':
        launchLine += ` --session ${escapedResumeId}`
        break
    }
  }

  // Pin the pre-generated ID on fresh launch so we know what to --resume later
  // without reading the agent's private session store.
  if (
    !payload.resumeSessionId &&
    payload.sessionId &&
    supportsSessionIdPinning(payload.agentType)
  ) {
    launchLine += ` ${getSessionIdPinningFlag(payload.agentType)} ${shellEscape(payload.sessionId)}`
  }

  if (payload.initialPrompt) {
    const escaped = shellEscape(payload.initialPrompt)
    switch (payload.agentType) {
      case 'copilot':
        launchLine += ` -i ${escaped}`
        break
      case 'gemini':
        launchLine += ` -i ${escaped}`
        break
      case 'opencode':
        launchLine += ` --prompt ${escaped}`
        break
      default:
        launchLine += ` ${escaped}`
        break
    }
  }

  return launchLine
}

/**
 * Builds the non-interactive launch command (for headless/background execution).
 * Uses each agent's native non-interactive mode:
 *   claude  -> claude -p 'prompt'
 *   copilot -> copilot -p 'prompt'
 *   codex   -> codex exec 'prompt'
 *   opencode -> opencode run 'prompt'
 *   gemini  -> gemini -p 'prompt'
 */
export function buildHeadlessLaunchLine(
  payload: CreateTerminalPayload,
  agentCommands: Record<AiAgentType, AgentCommandConfig>,
  env: Record<string, string>
): string {
  if (payload.agentType === 'shell') {
    throw new Error('buildHeadlessLaunchLine called for shell session')
  }
  const cmdConfig = agentCommands[payload.agentType] || DEFAULT_AGENT_COMMANDS[payload.agentType]
  const cmd = resolveAgentCommand(cmdConfig, env)
  const baseCmd = cmd.command
  const extraArgs = resolveHeadlessArgs(payload, cmdConfig, cmd.args)
  const argsStr = extraArgs.length > 0 ? extraArgs.join(' ') + ' ' : ''

  const emptyStr = process.platform === 'win32' ? '""' : "''"
  const prompt = payload.initialPrompt ? shellEscape(payload.initialPrompt) : emptyStr

  switch (payload.agentType) {
    case 'claude':
      return `${baseCmd} ${argsStr}-p ${prompt}`

    case 'copilot':
      return `${baseCmd} ${argsStr}-p ${prompt}`

    case 'codex':
      return `${baseCmd} ${argsStr}exec ${prompt}`

    case 'opencode':
      return `${baseCmd} ${argsStr}run ${prompt}`

    case 'gemini':
      return `${baseCmd} ${argsStr}-p ${prompt}`

    default:
      return `${baseCmd} ${argsStr}-p ${prompt}`
  }
}

export interface HeadlessSpawnArgs {
  command: string
  args: string[]
  /**
   * When set, the prompt should be written to the child's stdin rather than
   * passed as a command-line argument. Used for agents that read their prompt
   * from stdin (claude) so a multi-line prompt survives intact on Windows,
   * where `spawn(..., { shell: true })` word-splits and line-breaks unquoted
   * argv on the cmd.exe command line. See buildHeadlessSpawnArgs.
   */
  stdin?: string
}

/**
 * Returns { command, args } for direct spawn (no shell wrapper).
 * Avoids TTY/stdin issues that occur when spawning through sh -c in Node.js.
 *
 * On Windows the headless spawn uses `shell: true` (required to run the
 * `.cmd`/`.ps1` shims that npm-installed agents ship as). Under `shell: true`,
 * Node concatenates argv into a single cmd.exe command line with no quoting,
 * so a workflow prompt — which is multi-word and multi-line — gets word-split
 * (claude's `-p` then sees only the first token, e.g. `#`) and truncated at the
 * first newline. To avoid this entirely, agents that can read their prompt from
 * stdin return it via `stdin` instead of on the command line.
 */
export function buildHeadlessSpawnArgs(
  payload: CreateTerminalPayload,
  agentCommands: Record<AiAgentType, AgentCommandConfig>,
  env: Record<string, string>
): HeadlessSpawnArgs {
  if (payload.agentType === 'shell') {
    throw new Error('buildHeadlessSpawnArgs called for shell session')
  }
  const cmdConfig = agentCommands[payload.agentType] || DEFAULT_AGENT_COMMANDS[payload.agentType]
  const cmd = resolveAgentCommand(cmdConfig, env)
  const prompt = payload.initialPrompt || ''
  const extraArgs = [...resolveHeadlessArgs(payload, cmdConfig, cmd.args)]

  if (
    payload.resumeSessionId &&
    supportsExactSessionResume(payload.agentType) &&
    (payload.agentType === 'claude' || payload.agentType === 'copilot')
  ) {
    extraArgs.push('--resume', payload.resumeSessionId)
  } else if (payload.sessionId && supportsSessionIdPinning(payload.agentType)) {
    extraArgs.push(getSessionIdPinningFlag(payload.agentType), payload.sessionId)
  }

  switch (payload.agentType) {
    case 'claude':
      // `claude -p` (print mode) reads the prompt from stdin when no positional
      // prompt is given. Deliver it there so the shell never sees it.
      return prompt
        ? { command: cmd.command, args: [...extraArgs, '-p'], stdin: prompt }
        : { command: cmd.command, args: [...extraArgs, '-p', ''] }
    case 'copilot':
      return { command: cmd.command, args: [...extraArgs, '-p', prompt] }
    case 'codex':
      return { command: cmd.command, args: [...extraArgs, 'exec', prompt] }
    case 'opencode':
      return { command: cmd.command, args: [...extraArgs, 'run', prompt] }
    case 'gemini':
      return { command: cmd.command, args: [...extraArgs, '-p', prompt] }
    default:
      return { command: cmd.command, args: [...extraArgs, '-p', prompt] }
  }
}
