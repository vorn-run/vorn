import { Zap, Play, Terminal, GitFork, Hand, ListPlus } from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import type { NodeExecutionState, WorkflowNode } from '../../../shared/types'

/**
 * How a workflow node presents itself — one glyph and colour per node type,
 * shared by the config panel and the run trace so a step looks the same
 * wherever it is read.
 */
export const NODE_TYPE_VISUAL: Record<
  WorkflowNode['type'],
  { icon: LucideIcon; label: string; color: string; bg: string }
> = {
  trigger: { icon: Zap, label: 'Trigger', color: 'text-blue-400', bg: 'bg-blue-500/10' },
  launchAgent: { icon: Play, label: 'Agent', color: 'text-green-400', bg: 'bg-green-500/10' },
  script: { icon: Terminal, label: 'Script', color: 'text-amber-400', bg: 'bg-amber-500/10' },
  condition: {
    icon: GitFork,
    label: 'Condition',
    color: 'text-purple-400',
    bg: 'bg-purple-500/10'
  },
  approval: { icon: Hand, label: 'Approval', color: 'text-amber-400', bg: 'bg-amber-500/10' },
  createTaskFromItem: {
    icon: ListPlus,
    label: 'Create Task',
    color: 'text-gray-300',
    bg: 'bg-white/[0.06]'
  },
  callConnectorAction: {
    icon: Zap,
    label: 'Connector Action',
    color: 'text-gray-300',
    bg: 'bg-white/[0.06]'
  }
}

/**
 * The connection a node talks to, when it has one. Such a step is better
 * identified by its connector's brand mark than by its generic node type — a
 * "GitHub Trigger" should read as GitHub, not as a lightning bolt.
 */
export function nodeConnectionId(node: WorkflowNode | undefined): string | undefined {
  if (!node) return undefined
  const config = node.config as { connectionId?: string; triggerType?: string } | undefined
  if (node.type === 'callConnectorAction') return config?.connectionId
  if (node.type === 'trigger' && config?.triggerType === 'connectorPoll') return config.connectionId
  return undefined
}

/** Join the parts of a step descriptor, dropping the ones a node didn't set. */
function joinMeta(...parts: (string | undefined)[]): string | undefined {
  const kept = parts.filter((p): p is string => !!p && p.trim().length > 0)
  return kept.length > 0 ? kept.join(' · ') : undefined
}

function configString(node: WorkflowNode, key: string): string | undefined {
  const value = (node.config as Record<string, unknown> | undefined)?.[key]
  return typeof value === 'string' && value.trim().length > 0 ? value.trim() : undefined
}

/**
 * The one-line "what this step is configured to do" descriptor shown under a
 * step's title — the shell and project for a script, the agent and its
 * project, the connector action being called. Answers "which of my three
 * script steps is this?" without expanding the step.
 */
export function stepMeta(node: WorkflowNode | undefined, connectorId?: string): string | undefined {
  if (!node) return undefined
  const str = (key: string): string | undefined => configString(node, key)

  switch (node.type) {
    case 'trigger':
      return str('triggerType') === 'connectorPoll'
        ? joinMeta(connectorId, str('event'))
        : joinMeta(str('triggerType'), str('cron'))
    case 'script':
      return joinMeta(str('scriptType'), str('projectName'))
    case 'launchAgent':
      return joinMeta(str('projectName'), str('agentType'), str('branch'))
    case 'condition':
      return joinMeta(str('variable'), str('operator'), str('value'))
    case 'createTaskFromItem':
      return joinMeta(str('project'), str('initialStatus'))
    case 'callConnectorAction':
      return joinMeta(connectorId, str('action'))
    default:
      return undefined
  }
}

/**
 * The configured body of a step — the script it runs, the prompt it sends, the
 * approval message it waits on. Shown as a preview so a trace can be read
 * top-to-bottom without opening every step.
 */
export function stepPreview(node: WorkflowNode | undefined): string | undefined {
  if (!node) return undefined

  switch (node.type) {
    case 'trigger': {
      const event = configString(node, 'event')
      const cron = configString(node, 'cron')
      if (event) return `on: ${event}`
      return cron ? `cron: ${cron}` : undefined
    }
    case 'script':
      return configString(node, 'scriptContent')
    case 'launchAgent':
      return configString(node, 'prompt')
    case 'approval':
      return configString(node, 'message')
    case 'callConnectorAction': {
      const args = (node.config as { args?: Record<string, unknown> } | undefined)?.args
      const entries = args ? Object.entries(args) : []
      return entries.length > 0
        ? entries.map(([k, v]) => `${k}: ${String(v)}`).join('  ')
        : undefined
    }
    default:
      return undefined
  }
}

/**
 * The opening of a step's output, for the one-line preview on its card. Reads
 * like the log itself does — from the top — so expanding a step continues
 * where the preview left off rather than contradicting it.
 */
export function stepOutputPreview(state: NodeExecutionState): string | undefined {
  const text = state.logs?.trim() || state.error?.trim()
  if (!text) return undefined
  const firstLine = text.split('\n').find((l) => l.trim().length > 0)
  return firstLine?.trim() || undefined
}
