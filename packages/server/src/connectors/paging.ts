import type { ExternalItem, VornConnector } from '@vornrun/shared/types'

const MAX_BACKFILL_PAGES = 1_000

/**
 * Drain every reconciliation page a connector exposes. Legacy connectors
 * without `listItemsPage` still run once through `listItems`.
 */
export async function forEachConnectorItem(
  connector: VornConnector,
  filters: Record<string, unknown>,
  visit: (item: ExternalItem) => void
): Promise<void> {
  if (!connector.listItems && !connector.listItemsPage) {
    throw new Error(`Connector ${connector.id} does not support listItems()`)
  }

  let cursor: string | undefined
  for (let page = 0; page < MAX_BACKFILL_PAGES; page++) {
    const result = connector.listItemsPage
      ? await connector.listItemsPage(filters, cursor)
      : { items: await connector.listItems!(filters), hasMore: false }
    result.items.forEach(visit)
    if (!result.hasMore) return
    if (!result.nextCursor || result.nextCursor === cursor) {
      throw new Error(`Connector ${connector.id} did not advance its backfill cursor`)
    }
    cursor = result.nextCursor
  }
  throw new Error(`Connector ${connector.id} exceeded ${MAX_BACKFILL_PAGES} backfill pages`)
}
