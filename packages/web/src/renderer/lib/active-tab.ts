import { paneOwnerId } from './pane-id'

/**
 * What the active tab should become, or `undefined` to leave it as it is.
 *
 * The strip has to keep the active tab pointing at a tab that exists, and doing
 * that naively throws away the restored one twice over. On a cold launch the
 * list is empty for a moment, so the tab is cleared before its session arrives;
 * then it is replaced by whichever session happened to land first.
 *
 * Three answers rather than two, because "leave it alone" is a real outcome and
 * the one both of those bugs needed:
 *
 *   undefined  leave it — the sessions are still coming, or this tab names one
 *              of them that is not on the board yet
 *   null       clear it — there are no tabs at all
 *   string     move it — the tab it named is not among the ones that exist
 */
export function chooseActiveTab(
  activeTabId: string | null,
  allTabIds: readonly string[],
  knownSessionIds: ReadonlySet<string> | null
): string | null | undefined {
  // Null is "the server has not been asked", which an empty board cannot be
  // told apart from "there is nothing to show".
  if (knownSessionIds === null) return undefined
  // A tab naming a session the server has but the board is not showing is
  // waiting for the banner to bring it back, not pointing at nothing.
  if (activeTabId && knownSessionIds.has(paneOwnerId(activeTabId))) return undefined
  if (allTabIds.length === 0) return activeTabId === null ? undefined : null
  if (!activeTabId || !allTabIds.includes(activeTabId)) return allTabIds[0]
  return undefined
}
