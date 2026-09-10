import type { AiAgentType } from '@vornrun/shared/types'
import { supportsModelSelection, validateModelId } from '@vornrun/shared/agent-models'
import { tokenize } from './launch-tokens'
import { existsSync } from 'node:fs'

/** Every agent takes `--model`; the others also take `-m`, and codex `-c model=`. */
function takesShortFlag(agent: AiAgentType): boolean {
  return agent !== 'claude'
}

function isCodexModelConfig(value: string | undefined): boolean {
  return /^model\s*=/.test(value ?? '')
}

/** How many arguments the model selector starting at `args[i]` spans, or 0 when it is not one. */
function modelSelectorLength(agent: AiAgentType, args: string[], i: number): number {
  const arg = args[i]!
  const next = args[i + 1]
  if (arg === '--model' || (takesShortFlag(agent) && arg === '-m')) {
    if (!next || next.startsWith('-')) {
      throw new Error('Fix the incomplete model flag in agent arguments.')
    }
    return 2
  }
  if (arg.startsWith('--model=')) return 1
  if (takesShortFlag(agent) && arg.startsWith('-m') && arg.length > 2) return 1
  if (agent === 'codex') {
    if ((arg === '-c' || arg === '--config') && isCodexModelConfig(next)) return 2
    if (/^(?:-c=?|--config=)model\s*=/.test(arg)) return 1
  }
  return 0
}

/** The configured arguments with `model` as their one model selector; nothing past `--` is touched. */
export function applyModelArguments(agent: AiAgentType, args: string[], model: string): string[] {
  if (!supportsModelSelection(agent)) {
    throw new Error(`Model selection is not supported for ${agent}.`)
  }
  const id = validateModelId(model)
  const kept: string[] = []
  let i = 0
  while (i < args.length && args[i] !== '--') {
    const span = modelSelectorLength(agent, args, i)
    if (span === 0) kept.push(args[i]!)
    i += span || 1
  }
  return [...kept, '--model', id, ...args.slice(i)]
}

/** One executable the shell can be handed quoted, or a wrapper it must read as written. */
export function commandShape(command: string, onThisMachine = true): 'executable' | 'wrapper' {
  if (/[;&|<>`\n\r]/.test(command) || command.includes('$(')) return 'wrapper'
  const tokens = tokenize(command)
  if (!tokens) return 'wrapper'
  if (tokens.length === 1) return 'executable'
  // A path with spaces tokenizes into several words yet names one file.
  return onThisMachine && existsSync(command) ? 'executable' : 'wrapper'
}

/** A model rides the arguments, so the command must be one executable. */
export function assertModelCommand(command: string, onThisMachine = true): void {
  if (commandShape(command, onThisMachine) === 'wrapper') {
    throw new Error(
      'A model needs a command that is one executable. Move the wrapper and its flags into agent arguments, or use the configured default.'
    )
  }
}
