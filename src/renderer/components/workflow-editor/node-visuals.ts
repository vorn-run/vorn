import { Zap, Play, Terminal, GitFork, Hand, ListPlus, Repeat } from 'lucide-react'
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
  },
  loop: { icon: Repeat, label: 'Loop', color: 'text-gray-300', bg: 'bg-white/[0.06]' }
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

/** How far into a log to look for its first non-blank line. Generous enough
 *  for any realistic banner of leading blank lines, bounded so a huge log
 *  can't make the scan proportional to its size. */
const PREVIEW_SCAN_LIMIT = 4096
/** A single preview line is truncated by CSS; this only stops a log with no
 *  newlines at all from handing a megabyte-long "line" to React. */
const MAX_PREVIEW_LINE = 300

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
 *
 * Scans only the head of the text: a running agent step streams its log and
 * re-renders on every chunk, so splitting a multi-megabyte string here would
 * re-allocate the whole thing many times a second for one visible line.
 */
export function stepOutputPreview(state: NodeExecutionState): string | undefined {
  const text = state.logs || state.error
  if (!text) return undefined

  let start = 0
  while (start < text.length && start < PREVIEW_SCAN_LIMIT) {
    const newline = text.indexOf('\n', start)
    // Bound the slice, not just the loop: one "line" can itself be the whole
    // log when the output has no newlines at all.
    const end = Math.min(newline === -1 ? text.length : newline, start + MAX_PREVIEW_LINE)
    const line = text.slice(start, end).trim()
    if (line.length > 0) return line
    start = (newline === -1 ? text.length : newline) + 1
  }
  return undefined
}

/**
 * How much of a step's log the inline panel shows, and from which end.
 *
 * The end. An agent's answer — its verdict, its summary, the reason it failed —
 * is the last thing it writes, and the previous head-first slice meant a long
 * step showed its preamble and hid its conclusion. The container already
 * scrolls, so the only job of this cap is to keep a very large log out of the
 * DOM; "View full output" remains the untruncated path.
 */
const INLINE_LOG_CHARS = 8000

export function inlineLogTail(logs: string): string {
  if (logs.length <= INLINE_LOG_CHARS) return logs
  const elided = logs.length - INLINE_LOG_CHARS
  return `… ${elided.toLocaleString()} earlier characters hidden — use View full output\n\n${logs.slice(-INLINE_LOG_CHARS)}`
}

/** One line of a step's timeline. `engine` is what vorn did; `agent` is what the agent said. */
export interface StepTimelineEntry {
  kind: 'engine' | 'agent'
  text: string
}

/**
 * The engine notes the moment the agent first writes, so that line is the seam
 * between "getting the agent running" and "the agent talking". Splitting there
 * lets one ordered view read the way the step actually happened — setup, then
 * the agent, then how it ended — instead of asking the reader to correlate two
 * panels by timestamp.
 *
 * `logs` and `diagnostics` stay separate in the model on purpose: a typed step
 * parses `logs` for its declared JSON payload, and folding engine prose into it
 * would corrupt that parse.
 */
const FIRST_OUTPUT_NOTE = /^\[\+[\d.]+s\] First output from the agent\b/

export function stepTimeline(
  logs: string | undefined,
  diagnostics: string | undefined
): StepTimelineEntry[] {
  const notes = diagnostics ? diagnostics.split('\n').filter(Boolean) : []
  const agentText = logs?.trim() ? inlineLogTail(logs) : ''

  if (!agentText) {
    return notes.map((text) => ({ kind: 'engine' as const, text }))
  }

  // Everything up to and including the first-output note describes the launch;
  // whatever follows describes how the step ended.
  const seam = notes.findIndex((n) => FIRST_OUTPUT_NOTE.test(n))
  const before = seam === -1 ? notes : notes.slice(0, seam + 1)
  const after = seam === -1 ? [] : notes.slice(seam + 1)

  return [
    ...before.map((text) => ({ kind: 'engine' as const, text })),
    { kind: 'agent' as const, text: agentText },
    ...after.map((text) => ({ kind: 'engine' as const, text }))
  ]
}
