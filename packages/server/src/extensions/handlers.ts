import type { ExtensionLinkMatch, TerminalSession } from '@vornrun/shared/types'
import { activationFor, subjectOf } from './activation'
import { getOrStartHost, installedExtensions } from './hosts'
import { openPane, type OpenPane } from './panes'
import log from '../logger'

/**
 * The extensions that offer themselves for a piece of clicked text.
 *
 * Patterns are compiled per call and matched against a bounded string: the
 * manifest reader caps a pattern's length, and this caps the text, so a
 * pathological pattern costs a keystroke rather than the window.
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
    name: `vorn_handler_${handlerId}`,
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
