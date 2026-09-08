import { existsSync } from 'node:fs'
import { isAbsolute, join } from 'node:path'
import type {
  ExtensionActivation,
  ExtensionAgent,
  ExtensionContributionSummary,
  InstalledConnectorPack,
  TerminalSession
} from '@vornrun/shared/types'
import { remoteHostOf } from '../git-utils'

/**
 * Where an extension shows, decided per session.
 *
 * Every declared field has to hold and each is satisfied by any one of its
 * values, so an extension is absent where it has nothing to say rather than
 * drawing an empty band. A predicate this build cannot answer — an unreachable
 * git remote, a path it cannot stat — widens rather than hides, matching what
 * the manifest reader already does with one it cannot parse.
 */

/** What a session is, as a predicate reads it. */
export interface ActivationSubject {
  worktreePath: string
  agent: ExtensionAgent
  platform: NodeJS.Platform
  /** Read lazily: an extension naming no host must not cost a git call. */
  remoteHost: () => string | null
}

/** Which of an extension's contributions show for one session. */
export interface Activation {
  /** False when the extension itself is not active, whatever its contributions say. */
  active: boolean
  panes: string[]
  footers: string[]
  linkHandlers: string[]
}

const INACTIVE: Activation = { active: false, panes: [], footers: [], linkHandlers: [] }

/** A session as the rules read it, with the git remote deferred until asked for. */
export function subjectOf(session: TerminalSession): ActivationSubject {
  const worktreePath = session.worktreePath ?? session.projectPath
  let looked = false
  let host: string | null = null
  return {
    worktreePath,
    agent: session.agentType as ExtensionAgent,
    platform: process.platform,
    remoteHost: () => {
      if (!looked) {
        looked = true
        host = remoteHostOf(worktreePath)
      }
      return host
    }
  }
}

/** A listed path exists under the worktree; `..` and absolutes were refused when the manifest was read. */
function containsAny(worktreePath: string, paths: string[]): boolean {
  return paths.some((entry) => {
    if (entry === '' || isAbsolute(entry) || entry.split('/').includes('..')) return false
    return existsSync(join(worktreePath, entry))
  })
}

export function matches(
  predicate: ExtensionActivation | undefined,
  subject: ActivationSubject
): boolean {
  if (!predicate) return true
  if (
    predicate.workspaceContains?.length &&
    !containsAny(subject.worktreePath, predicate.workspaceContains)
  ) {
    return false
  }
  if (predicate.agent?.length && !predicate.agent.includes(subject.agent)) return false
  if (predicate.platform?.length) {
    const platform = subject.platform as (typeof predicate.platform)[number]
    if (!predicate.platform.includes(platform)) return false
  }
  if (predicate.remoteHost?.length) {
    const host = subject.remoteHost()
    // An unreadable remote widens: refusing here would hide the extension on a
    // repository it was written for because a git call happened to fail.
    if (host !== null && !predicate.remoteHost.some((named) => named.toLowerCase() === host)) {
      return false
    }
  }
  return true
}

const shown = (
  contributions: ExtensionContributionSummary[] | undefined,
  subject: ActivationSubject
): string[] =>
  (contributions ?? []).filter((one) => matches(one.when, subject)).map((one) => one.id)

/** What this extension contributes to this session, which for a connector is nothing. */
export function activationFor(
  pack: InstalledConnectorPack,
  subject: ActivationSubject
): Activation {
  if (pack.kind !== 'extension') return INACTIVE
  if (!matches(pack.activates, subject)) return INACTIVE
  return {
    active: true,
    panes: shown(pack.contributes?.panes, subject),
    footers: shown(pack.contributes?.footers, subject),
    linkHandlers: shown(pack.contributes?.linkHandlers, subject)
  }
}
