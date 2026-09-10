import {
  AiAgentType,
  AgentType,
  AgentCommandConfig,
  CreateTerminalPayload,
  supportsExactSessionResume,
  supportsSessionIdPinning,
  getSessionIdPinningFlag
} from '@vornrun/shared/types'
import { DEFAULT_AGENT_COMMANDS } from '@vornrun/shared/agent-defaults'
import { shellEscape } from './process-utils'
import { stripSessionSelectors } from './launch-tokens'
import { applyModelArguments, assertModelCommand, commandShape } from './model-arguments'
import { findOnPath } from './resolve-executable'

/** The configured command, and where on `env.PATH` it lives; the name when it is not there. */
function resolveAgentCommand(
  config: AgentCommandConfig,
  env: Record<string, string>
): { command: string; args: string[]; path: string } {
  const pathEnv = env.PATH ?? env.Path
  const primary = findOnPath(config.command, pathEnv)
  if (primary) return { command: config.command, args: config.args, path: primary }
  if (config.fallbackCommand) {
    const fallback = findOnPath(config.fallbackCommand, pathEnv)
    if (fallback)
      return { command: config.fallbackCommand, args: config.fallbackArgs ?? [], path: fallback }
  }
  return { command: config.command, args: config.args, path: config.command }
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

/** The arguments with the chosen model as their one model selector, or as they are. */
function withModel(payload: CreateTerminalPayload, command: string, args: string[]): string[] {
  if (payload.model === undefined) return args
  assertModelCommand(command, !payload.remoteHostId)
  return applyModelArguments(payload.agentType, args, payload.model)
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
  if ((payload.agentType as AgentType) === 'shell') {
    throw new Error('buildAgentLaunchLine called for shell session — use createShellPty instead')
  }
  const cmdConfig = agentCommands[payload.agentType] || DEFAULT_AGENT_COMMANDS[payload.agentType]
  const cmd = resolveAgentCommand(cmdConfig, env)
  // Per-step args override settings-level args; escape each for shell safety
  const escape = (value: string) => shellEscape(value, payload.remoteHostId ? 'posix' : 'auto')
  const effectiveArgs = withModel(payload, cmd.command, payload.args ?? cmd.args)
  const commandLine =
    commandShape(cmd.command, !payload.remoteHostId) === 'executable'
      ? escape(cmd.command)
      : cmd.command
  let launchLine = [commandLine, ...effectiveArgs.map(escape)].join(' ')

  // Where the configured command ends and its arguments begin. Known exactly
  // rather than guessed, because this line was just composed here -- which is
  // what lets `npx -y @anthropic-ai/claude-code` be a command and an argument
  // that merely ends in `/claude` be left alone.
  const argsFrom = commandLine.length

  const exactResume = Boolean(
    payload.resumeSessionId && supportsExactSessionResume(payload.agentType)
  )
  const pinning = Boolean(
    !payload.resumeSessionId && payload.sessionId && supportsSessionIdPinning(payload.agentType)
  )

  // A configured command may already carry a selector -- somebody who always
  // resumes the same session, or who pinned an id by hand. Two of them compete
  // and one wins silently, which is the wrong session with nothing to say a
  // choice was made. Removed only where it can be proven; a line this cannot
  // read is handed back untouched and ours is appended beside theirs.
  if (exactResume || pinning) {
    launchLine = stripSessionSelectors(launchLine, payload.agentType, argsFrom)
  }

  if (exactResume && payload.resumeSessionId) {
    const escapedResumeId = escape(payload.resumeSessionId)
    switch (payload.agentType) {
      case 'claude':
        launchLine += ` --resume ${escapedResumeId}`
        break
      case 'copilot':
        launchLine += ` --resume ${escapedResumeId}`
        break
      case 'codex': {
        // Spliced in as the first argument rather than replacing the line.
        // Rebuilding it as `${cmd.command} resume ${id}` threw away every
        // argument the person had configured -- a model, a sandbox setting, an
        // approval policy -- and only ever on the resume path, so a session came
        // back configured differently from the one it continues.
        const rest = launchLine.slice(argsFrom)
        launchLine = `${launchLine.slice(0, argsFrom)} resume ${escapedResumeId}${rest}`
        break
      }
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
    launchLine += ` ${getSessionIdPinningFlag(payload.agentType)} ${escape(payload.sessionId)}`
  }

  if (payload.initialPrompt) {
    const escaped = escape(payload.initialPrompt)
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

export interface HeadlessSpawnArgs {
  command: string
  args: string[]
  /**
   * When set, the prompt should be written to the child's stdin rather than
   * passed as a command-line argument. Every supported agent reads its prompt
   * this way, so a multi-line prompt survives intact on Windows — there,
   * `spawn(..., { shell: true })` word-splits unquoted argv on the cmd.exe
   * command line, and a literal newline cannot be carried by it at all,
   * quoted or not. See buildHeadlessSpawnArgs.
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
  if ((payload.agentType as AgentType) === 'shell') {
    throw new Error('buildHeadlessSpawnArgs called for shell session')
  }
  const cmdConfig = agentCommands[payload.agentType] || DEFAULT_AGENT_COMMANDS[payload.agentType]
  const cmd = resolveAgentCommand(cmdConfig, env)
  const prompt = payload.initialPrompt || ''
  const extraArgs = [
    ...withModel(payload, cmd.command, resolveHeadlessArgs(payload, cmdConfig, cmd.args))
  ]

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
        ? { command: cmd.path, args: [...extraArgs, '-p'], stdin: prompt }
        : { command: cmd.path, args: [...extraArgs, '-p', ''] }
    case 'copilot':
      // `copilot` reads the prompt from stdin when `-p` is absent ("Run in an
      // interactive terminal or provide a prompt with -p or via standard in").
      // Deliver it there so the workflow prompt — always multi-line — never
      // reaches the cmd.exe command line, which cannot carry a literal newline.
      //
      // Passing it as `-p` on Windows left copilot with a truncated value or
      // none at all, and with no prompt it blocks on stdin producing no output
      // whatsoever: the step never finished and its run never closed.
      return prompt
        ? { command: cmd.path, args: [...extraArgs], stdin: prompt }
        : { command: cmd.path, args: [...extraArgs, '-p', ''] }
    case 'codex':
      // `codex exec` reads its instructions from stdin when no PROMPT argument
      // is given. Passing the prompt positionally instead would also work on
      // POSIX, but on Windows it goes through the cmd.exe command line, which
      // truncates it at the first newline. Note the prompt must NOT also be
      // passed as an argument — codex then treats stdin as a separate
      // `<stdin>` block rather than as the instructions.
      if (payload.resumeSessionId) {
        return {
          command: cmd.path,
          args: [...extraArgs, 'exec', 'resume', payload.resumeSessionId, '-'],
          stdin: prompt
        }
      }
      return prompt
        ? { command: cmd.path, args: [...extraArgs, 'exec'], stdin: prompt }
        : { command: cmd.path, args: [...extraArgs, 'exec', ''] }
    case 'opencode':
      // `opencode run` with no positional message reads the message from stdin.
      return prompt
        ? { command: cmd.path, args: [...extraArgs, 'run'], stdin: prompt }
        : { command: cmd.path, args: [...extraArgs, 'run', ''] }
    case 'gemini':
      // gemini reads stdin whenever stdin isn't a TTY and uses it as the input
      // when no `-p` is given (`input = input ? stdin + input : stdin`). Piped
      // stdio also puts it in headless mode on its own, so `-p` isn't needed to
      // stop it going interactive.
      //
      // Caveat: under gemini's own sandbox (GEMINI_SANDBOX), it reads stdin and
      // re-injects it as `--prompt` on the relaunch command line — which would
      // reintroduce the newline truncation. Vorn doesn't enable that sandbox.
      return prompt
        ? { command: cmd.path, args: [...extraArgs], stdin: prompt }
        : { command: cmd.path, args: [...extraArgs, '-p', ''] }
    default:
      return { command: cmd.path, args: [...extraArgs, '-p', prompt] }
  }
}
