import type {
  ExtensionActivation,
  ExtensionAgent,
  ExtensionContributions,
  ExtensionPermission,
  ExtensionPlatform
} from '../../shared/types'
import { AGENT_DEFINITIONS } from './agent-definitions'

/**
 * An extension in the words a person reads before installing it.
 *
 * The manifest states a permission as `terminal.selection` and an activation as
 * a glob, which is right for a pack and wrong for the page that asks whether to
 * trust one. Everything that turns either into a sentence lives here, so the
 * row, the detail page and the confirm sheet cannot describe the same extension
 * three different ways.
 */

/** Which verb a permission answers to, and what it reaches. */
export const EXTENSION_PERMISSION: Record<
  ExtensionPermission,
  { bucket: 'Reads' | 'Sends' | 'Renames'; phrase: string }
> = {
  'git.read': { bucket: 'Reads', phrase: "the worktree's diff and status" },
  'terminal.read': { bucket: 'Reads', phrase: "the session's recent terminal output" },
  'terminal.selection': { bucket: 'Reads', phrase: 'the text selected in the terminal' },
  'agent.usage': { bucket: 'Reads', phrase: "the agent's context and provider allowance" },
  'terminal.send': { bucket: 'Sends', phrase: "text into the session's terminal" },
  'card.rename': { bucket: 'Renames', phrase: 'the session card' }
}

/**
 * Every permission, in the order they are worth reading.
 *
 * Rows follow this rather than the manifest's order, so two extensions asking
 * for the same things read identically and a reordered manifest cannot look
 * like a changed request.
 */
const PERMISSION_ORDER: ExtensionPermission[] = [
  'git.read',
  'terminal.read',
  'terminal.selection',
  'agent.usage',
  'terminal.send',
  'card.rename'
]

const BUCKET_ORDER: Array<'Reads' | 'Sends' | 'Renames'> = ['Reads', 'Sends', 'Renames']

/** What an extension asks for, grouped under the verb it answers to. */
export function permissionRows(
  permissions: ExtensionPermission[] | undefined
): Array<{ label: 'Reads' | 'Sends' | 'Renames'; items: string[] }> {
  const asked = new Set(permissions ?? [])
  return BUCKET_ORDER.map((label) => ({
    label,
    items: PERMISSION_ORDER.filter(
      (name) => asked.has(name) && EXTENSION_PERMISSION[name].bucket === label
    ).map((name) => EXTENSION_PERMISSION[name].phrase)
  })).filter((row) => row.items.length > 0)
}

/**
 * What it did not ask for, from the same closed list.
 *
 * A grant reads as small only against what could have been asked, and this is
 * the only honest way to say it: the complement of the set, not a reassuring
 * sentence about things no extension can reach anyway.
 */
export function notAsked(permissions: ExtensionPermission[] | undefined): string[] {
  const asked = new Set(permissions ?? [])
  return PERMISSION_ORDER.filter((name) => !asked.has(name)).map(
    (name) => EXTENSION_PERMISSION[name].phrase
  )
}

const PLATFORM_LABEL: Record<ExtensionPlatform, string> = {
  darwin: 'macOS',
  linux: 'Linux',
  win32: 'Windows'
}

// A shell is not one of the agents, so it is named here rather than in their table.
function agentLabel(agent: ExtensionAgent): string {
  if (agent === 'shell') return 'shell'
  return AGENT_DEFINITIONS[agent]?.displayName ?? agent
}

/** Well-known hosts said as their names; anything else says the host itself. */
const HOST_LABEL: Record<string, string> = {
  'github.com': 'GitHub',
  'gitlab.com': 'GitLab',
  'bitbucket.org': 'Bitbucket'
}

/** "a, b or c" — any one of them is enough, which is what the predicate means. */
function anyOf(values: string[]): string {
  if (values.length <= 1) return values[0] ?? ''
  return `${values.slice(0, -1).join(', ')} or ${values[values.length - 1]}`
}

/**
 * Where an extension shows, as clauses rather than a predicate.
 *
 * Each declared field has to hold and each is satisfied by any one of its
 * values, so every clause is an "or" inside and an "and" between. Nothing
 * declared means it shows everywhere, which is worth saying rather than leaving
 * the section empty.
 */
export function describeActivation(activates: ExtensionActivation | undefined): string[] {
  const clauses: string[] = []
  const contains = activates?.workspaceContains?.filter(Boolean) ?? []
  if (contains.length > 0) clauses.push(`projects with ${anyOf(contains)}`)

  const hosts = activates?.remoteHost?.filter(Boolean) ?? []
  if (hosts.length > 0) {
    const named = hosts.filter((host) => HOST_LABEL[host] !== undefined)
    const rest = hosts.filter((host) => HOST_LABEL[host] === undefined)
    if (named.length > 0) clauses.push(`${anyOf(named.map((host) => HOST_LABEL[host]))} remotes`)
    if (rest.length > 0) clauses.push(`remotes on ${anyOf(rest)}`)
  }

  const agents = activates?.agent ?? []
  if (agents.length > 0) clauses.push(`${anyOf(agents.map(agentLabel))} sessions`)

  const platforms = activates?.platform ?? []
  if (platforms.length > 0)
    clauses.push(anyOf(platforms.map((name) => PLATFORM_LABEL[name] ?? name)))

  return clauses.length > 0 ? clauses : ['every session']
}

/** "2 panes, 1 footer" — what it adds, counted the way the row counts triggers. */
export function describeContributions(contributes: ExtensionContributions | undefined): string {
  const parts = [
    count(contributes?.panes?.length ?? 0, 'pane'),
    count(contributes?.footers?.length ?? 0, 'footer'),
    count(contributes?.linkHandlers?.length ?? 0, 'link handler')
  ].filter((part): part is string => part !== null)
  return parts.join(', ')
}

/** How a pane is drawn, or how often a footer runs: the one fact per kind worth a line. */
export function describePaneKind(pane: { web?: string; command?: string[] }): string {
  if (pane.command && pane.command.length > 0) return `runs ${pane.command.join(' ')}`
  return 'a page it ships'
}

export function describeFooterInterval(seconds: number): string {
  if (seconds % 60 === 0 && seconds >= 60) {
    const minutes = seconds / 60
    return `every ${minutes === 1 ? 'minute' : `${minutes} minutes`}`
  }
  return `every ${seconds}s`
}

function count(n: number, noun: string): string | null {
  if (n === 0) return null
  return `${n} ${noun}${n === 1 ? '' : 's'}`
}
