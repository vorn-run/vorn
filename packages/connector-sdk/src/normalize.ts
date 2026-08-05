import type { ConnectorItem, NormalizedItem } from './types'

/**
 * Reserved keys a connector item always defines. `data` may not overwrite
 * them, because Vorn reads them by name to build its task/trigger context.
 */
const RESERVED_KEYS = [
  'externalId',
  'title',
  'url',
  'description',
  'status',
  'labels',
  'assignee',
  'updatedAt'
] as const

/**
 * Keys that would mutate `Object.prototype` (or look like they do) once a
 * consumer spreads or merges the normalized item. Dropped rather than thrown
 * on, so a single odd column in a source system cannot stall a whole poll.
 */
const UNSAFE_KEYS = ['__proto__', 'constructor', 'prototype'] as const

function isoTimestamp(value: string | Date | undefined, fallback: string): string {
  if (value === undefined) return fallback
  const date = value instanceof Date ? value : new Date(value)
  if (Number.isNaN(date.getTime())) {
    throw new Error(`Invalid updatedAt: ${String(value)}`)
  }
  return date.toISOString()
}

/**
 * Turn an author-supplied item into the flat JSON shape Vorn consumes.
 *
 * Normalizing here rather than in each connector is what lets one host-side
 * poll configuration (`idField: externalId`, `timestampField: updatedAt`, …)
 * work for every SDK connector.
 */
export function normalizeItem(item: ConnectorItem, polledAt: string): NormalizedItem {
  const externalId = String(item.externalId ?? '').trim()
  if (!externalId) {
    throw new Error('Connector item is missing externalId')
  }
  if (!item.title || !item.title.trim()) {
    throw new Error(`Connector item ${externalId} is missing title`)
  }

  const extra: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(item.data ?? {})) {
    if ((RESERVED_KEYS as readonly string[]).includes(key)) continue
    if ((UNSAFE_KEYS as readonly string[]).includes(key)) continue
    extra[key] = value
  }

  return {
    ...extra,
    externalId,
    title: item.title,
    url: item.url ?? '',
    description: item.description ?? '',
    status: item.status ?? 'open',
    labels: item.labels ?? [],
    ...(item.assignee !== undefined && { assignee: item.assignee }),
    updatedAt: isoTimestamp(item.updatedAt, polledAt)
  }
}

/**
 * Normalize a page and reject duplicate ids within it. Two items sharing an
 * id in one page means one of them would be silently dropped by Vorn's
 * dedupe, which looks like data loss long after the fact.
 */
export function normalizeItems(items: ConnectorItem[], polledAt: string): NormalizedItem[] {
  const seen = new Set<string>()
  return items.map((item) => {
    const normalized = normalizeItem(item, polledAt)
    if (seen.has(normalized.externalId)) {
      throw new Error(`Duplicate externalId "${normalized.externalId}" in one poll page`)
    }
    seen.add(normalized.externalId)
    return normalized
  })
}
