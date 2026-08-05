import { itemExternalId, itemTimestamp } from './normalize'
import type {
  ConnectorItem,
  DedupeStrategy,
  FetchContext,
  PollContext,
  PollOutcome,
  TriggerDefinition
} from './types'

/**
 * Cursor state the SDK owns on the author's behalf. It travels through Vorn as
 * an opaque string, so the shape is versioned and never inspected by the host.
 */
type DedupeCursor =
  | { v: 1; s: 'timestamp'; t: string; ids: string[] }
  | { v: 1; s: 'lastItem'; id: string }

type CursorFor<S extends DedupeStrategy> = Extract<DedupeCursor, { s: S }>

/**
 * How many ids are remembered at the newest timestamp. Items sharing one
 * instant are the only ones the timestamp window cannot separate, so the list
 * is normally tiny; the cap stops a source that stamps thousands of rows
 * identically from growing the cursor without bound. Which ids are dropped is
 * arbitrary — exceeding the cap can redeliver an item, which Vorn's inbox
 * de-duplicates on `externalId` anyway.
 */
const MAX_BOUNDARY_IDS = 500

function decodeCursor<S extends DedupeStrategy>(
  cursor: string | undefined,
  strategy: S
): CursorFor<S> | undefined {
  if (!cursor) return undefined
  let parsed: unknown
  try {
    parsed = JSON.parse(cursor)
  } catch (error) {
    throw new Error(`Cursor is not valid SDK cursor JSON: ${cursor}`, { cause: error })
  }
  const state = parsed as DedupeCursor
  if (!state || typeof state !== 'object' || state.v !== 1 || state.s !== strategy) {
    throw new Error(`Cursor does not belong to the "${strategy}" strategy: ${cursor}`)
  }
  return state as CursorFor<S>
}

/** An item with its dedupe keys resolved once, rather than per comparison. */
interface Keyed {
  item: ConnectorItem
  at: string
  id: string
}

/**
 * Turn one page into the shape the strategies share. Delivering `items` in
 * oldest-first order means Vorn creates runs in the order things actually
 * happened, and truncating to `limit` always leaves the *newest* items for the
 * next poll rather than stranding the oldest behind an advanced cursor.
 */
function page(
  chronological: Keyed[],
  context: PollContext,
  hadCursor: boolean,
  nextCursor: (delivered: Keyed[]) => DedupeCursor
): PollOutcome {
  const delivered =
    context.limit === undefined ? chronological : chronological.slice(0, context.limit)
  if (delivered.length === 0) {
    return { items: [], ...(context.cursor !== undefined && { nextCursor: context.cursor }) }
  }
  return {
    items: delivered.map((entry) => entry.item),
    nextCursor: JSON.stringify(nextCursor(delivered)),
    // Only drain a backlog we know we truncated, and only once a cursor
    // exists — a first poll should not pull the source's entire history.
    hasMore: chronological.length > delivered.length && hadCursor
  }
}

function timestampPoll(
  fetched: ConnectorItem[],
  state: CursorFor<'timestamp'> | undefined,
  context: PollContext,
  polledAt: string
): PollOutcome {
  const boundary = state?.t ?? context.since
  const seen = new Set(state?.ids ?? [])

  const fresh: Keyed[] = []
  for (const item of fetched) {
    const at = itemTimestamp(item, polledAt)
    // Items sharing the boundary instant are new only if this cursor has not
    // already delivered them. Without this, `>` drops them forever and `>=`
    // redelivers them on every single poll.
    const isNew =
      boundary === undefined ||
      at > boundary ||
      (at === boundary && !seen.has(itemExternalId(item)))
    if (isNew) fresh.push({ item, at, id: itemExternalId(item) })
  }
  fresh.sort((left, right) =>
    left.at === right.at ? (left.id < right.id ? -1 : 1) : left.at < right.at ? -1 : 1
  )

  return page(fresh, context, state !== undefined, (delivered) => {
    const newest = delivered[delivered.length - 1]!.at
    const atNewest: string[] = []
    for (let i = delivered.length - 1; i >= 0 && delivered[i]!.at === newest; i -= 1) {
      atNewest.push(delivered[i]!.id)
    }
    // Carry the previous boundary ids forward only while the boundary itself
    // has not moved; once it does they can never match again.
    const ids = (newest === boundary ? [...seen, ...atNewest] : atNewest).slice(-MAX_BOUNDARY_IDS)
    return { v: 1, s: 'timestamp', t: newest, ids }
  })
}

function lastItemPoll(
  fetched: ConnectorItem[],
  state: CursorFor<'lastItem'> | undefined,
  context: PollContext,
  polledAt: string
): PollOutcome {
  // `fetch` is documented to return newest-first for this strategy.
  const keyed = fetched.map((item) => ({
    item,
    at: itemTimestamp(item, polledAt),
    id: itemExternalId(item)
  }))
  const stopAt = state ? keyed.findIndex((entry) => entry.id === state.id) : -1
  const chronological = (stopAt === -1 ? keyed : keyed.slice(0, stopAt)).reverse()

  return page(chronological, context, state !== undefined, (delivered) => ({
    v: 1,
    s: 'lastItem',
    id: delivered[delivered.length - 1]!.id
  }))
}

/**
 * Run a declarative trigger: call the author's `fetch`, then apply the chosen
 * dedupe strategy.
 *
 * This exists because cursor bookkeeping is where hand-written pull connectors
 * go wrong — duplicate deliveries, items lost at the timestamp boundary, and
 * cursors that never advance. Solving it once here means every connector, and
 * every connector an agent generates, inherits the fix.
 */
export async function pollWithDedupe(
  trigger: TriggerDefinition,
  context: PollContext
): Promise<PollOutcome> {
  const strategy = trigger.dedupe
  const fetchItems = trigger.fetch
  if (!strategy || !fetchItems) {
    throw new Error(`Trigger ${trigger.type} is not a declarative trigger`)
  }
  const polledAt = context.now()

  if (strategy === 'lastItem') {
    const state = decodeCursor(context.cursor, 'lastItem')
    const fetched = await runFetch(trigger.type, fetchItems, {
      config: context.config,
      ...(state && { lastItemId: state.id }),
      ...(context.limit !== undefined && { limit: context.limit }),
      now: context.now
    })
    return lastItemPoll(fetched, state, context, polledAt)
  }

  const state = decodeCursor(context.cursor, 'timestamp')
  // The cursor is the authority on what has been delivered; the host's `since`
  // only seeds the very first poll.
  const since = state?.t ?? context.since
  const fetched = await runFetch(trigger.type, fetchItems, {
    config: context.config,
    ...(since !== undefined && { since }),
    ...(context.limit !== undefined && { limit: context.limit }),
    now: context.now
  })
  return timestampPoll(fetched, state, context, polledAt)
}

async function runFetch(
  type: string,
  fetchItems: NonNullable<TriggerDefinition['fetch']>,
  context: FetchContext
): Promise<ConnectorItem[]> {
  const fetched = await fetchItems(context)
  if (!Array.isArray(fetched)) {
    throw new Error(`Trigger ${type} fetch() did not return an array`)
  }
  return fetched
}
