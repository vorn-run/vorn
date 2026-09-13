import type { ExternalItem, VornConnector } from '@vornrun/shared/types'

const MAX_BACKFILL_PAGES = 1_000

export interface Page<T> {
  items: T[]
  nextCursor?: string
  hasMore?: boolean
}

/** Follow `hasMore` from the first page to the last, refusing a cursor that stops moving. */
export async function forEachPage<T>(
  who: string,
  page: (cursor: string | undefined) => Promise<Page<T>>,
  visit: (item: T) => void
): Promise<void> {
  let cursor: string | undefined
  for (let n = 0; n < MAX_BACKFILL_PAGES; n++) {
    const result = await page(cursor)
    for (const item of result.items) visit(item)
    if (!result.hasMore) return
    if (!result.nextCursor || result.nextCursor === cursor) {
      throw new Error(`${who} did not advance its backfill cursor`)
    }
    cursor = result.nextCursor
  }
  throw new Error(`${who} exceeded ${MAX_BACKFILL_PAGES} backfill pages`)
}

/** Drain every reconciliation page a connector exposes; one without `listItemsPage` runs once through `listItems`. */
export async function forEachConnectorItem(
  connector: VornConnector,
  filters: Record<string, unknown>,
  visit: (item: ExternalItem) => void
): Promise<void> {
  if (!connector.listItems && !connector.listItemsPage) {
    throw new Error(`Connector ${connector.id} does not support listItems()`)
  }
  await forEachPage(
    `Connector ${connector.id}`,
    async (cursor) =>
      connector.listItemsPage
        ? connector.listItemsPage(filters, cursor)
        : { items: await connector.listItems!(filters), hasMore: false },
    visit
  )
}
