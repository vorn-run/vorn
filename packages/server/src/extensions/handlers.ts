import type { ExtensionLinkMatch, TerminalSession } from '@vornrun/shared/types'
import { activationFor, subjectOf } from './activation'
import { handlerToolName } from '../connectors/sdk-tools'
import { getOrStartHost, installedExtensions } from './hosts'
import { openPane, type OpenPane } from './panes'
import log from '../logger'

/**
 * The extensions that offer themselves for a piece of clicked text.
 *
 * What a pattern may be is decided before it gets here: the manifest reader caps
 * its length and refuses one that repeats a group which already repeats, which
 * is the shape that costs exponential time. This caps the text as well, so what
 * is matched is bounded on both sides.
 */

/** Longer than any link worth clicking, short enough that a bad pattern cannot chew on it. */
const MAX_CLICKED_TEXT = 2048

export function matchLinks(session: TerminalSession, text: string): ExtensionLinkMatch[] {
  const clicked = text.slice(0, MAX_CLICKED_TEXT)
  if (clicked === '') return []
  const subject = subjectOf(session)
  const matches: ExtensionLinkMatch[] = []

  for (const pack of installedExtensions()) {
    const activation = activationFor(pack, subject)
    if (!activation.active) continue
    for (const handler of pack.contributes?.linkHandlers ?? []) {
      if (!activation.linkHandlers.includes(handler.id)) continue
      let pattern: RegExp
      try {
        pattern = new RegExp(handler.pattern)
      } catch {
        // A pattern that no longer compiles offers nothing rather than refusing the click.
        continue
      }
      if (!pattern.test(clicked)) continue
      matches.push({
        extensionId: pack.id,
        extensionName: pack.name,
        handlerId: handler.id,
        title: handler.title
      })
    }
  }
  return matches
}

export async function runHandler(
  extensionId: string,
  handlerId: string,
  session: TerminalSession,
  url: string
): Promise<{ openedPane?: OpenPane }> {
  const client = await getOrStartHost(extensionId, session.projectPath)
  const answered = await client.callTool({
    name: handlerToolName(handlerId),
    arguments: {
      sessionId: session.id,
      worktreePath: session.worktreePath ?? session.projectPath,
      agent: session.agentType,
      url: url.slice(0, MAX_CLICKED_TEXT)
    }
  })
  if (answered.isError) {
    throw new Error(String(answered.content ?? `${handlerId} failed`))
  }
  const asked = (answered.structuredContent as { openPane?: unknown })?.openPane
  if (typeof asked !== 'string' || asked === '') return {}
  log.info(`[extensions] ${extensionId} ${handlerId} opened ${asked}`)
  return { openedPane: await openPane(extensionId, asked, session) }
}
