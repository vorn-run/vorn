import { useMemo, type ReactNode } from 'react'
import { useAppStore } from '../../stores'
import type { ExtensionFooterItem, ExtensionFooterReading } from '../../../shared/types'

/** Stable empty, so a card with no extensions does not invalidate this memo. */
const NO_READINGS: ExtensionFooterReading[] = []

/**
 * What an extension's footers currently say about this session.
 *
 * One band per footer, under the card's own status bar and wearing its
 * metrics, because it is the same kind of thing: a line of standing facts about
 * the session, read at a glance and never operated. Nothing here is a control —
 * a band that could be clicked would be a second toolbar in the place a person
 * looks to check something, and an extension's actions belong in its pane.
 *
 * The colour rule is the card's. A reading that is fine recedes; only something
 * wrong takes colour, so a row of green ticks cannot drown the one red one.
 */
export function ExtensionFooterBand({
  terminalId,
  dimmed
}: {
  terminalId: string
  dimmed?: boolean
}): ReactNode {
  const readings = useAppStore((s) => s.extensionFooters.get(terminalId) ?? NO_READINGS)

  // Sorted here rather than at the source: readings arrive one footer at a time
  // from processes that start in whatever order the host got to them, and a band
  // that reorders itself as its neighbours report is unreadable.
  const ordered = useMemo(
    () =>
      [...readings].sort(
        (a, b) =>
          a.extensionName.localeCompare(b.extensionName) || a.footerId.localeCompare(b.footerId)
      ),
    [readings]
  )

  if (ordered.length === 0) return null

  return (
    <>
      {ordered.map((reading) => (
        <div
          key={`${reading.extensionId}:${reading.footerId}`}
          data-testid={`extension-footer-${terminalId}-${reading.extensionId}-${reading.footerId}`}
          className={`shrink-0 flex items-center gap-3 px-2 h-[22px] border-t border-white/[0.04]
                      text-[11px] overflow-hidden
                      transition-opacity duration-200 ease-out
                      ${dimmed ? 'opacity-60 group-hover/card:opacity-100' : 'opacity-100'}`}
          style={{ background: 'var(--color-surface-raised)' }}
        >
          <span className="text-[10px] uppercase tracking-[0.04em] text-ink-ghost shrink-0">
            {reading.title}
          </span>
          {reading.error ? (
            <span className="text-danger truncate" title={reading.error}>
              {reading.error}
            </span>
          ) : (
            reading.items.map((item, i) => <FooterItem key={i} item={item} />)
          )}
          <span className="flex-1" />
          <ComputedAt at={reading.computedAt} />
        </div>
      ))}
    </>
  )
}

function FooterItem({ item }: { item: ExtensionFooterItem }): ReactNode {
  const value = <span className={`tabular-nums ${TONE[item.tone ?? 'default']}`}>{item.value}</span>
  const body = (
    <>
      <span className="text-ink-faint">{item.label}</span>
      {value}
    </>
  )

  // Opened outside rather than framed: a footer names something that lives on
  // the web -- a run, an issue -- and the host has already held it to http.
  if (item.href) {
    return (
      <button
        type="button"
        onPointerDown={(e) => e.stopPropagation()}
        onClick={(e) => {
          e.stopPropagation()
          window.api.openExternal?.(item.href as string)
        }}
        title={item.href}
        className="flex items-center gap-1.5 shrink-0 hover:underline underline-offset-2 decoration-white/25"
      >
        {body}
      </button>
    )
  }
  return <span className="flex items-center gap-1.5 shrink-0">{body}</span>
}

const TONE: Record<NonNullable<ExtensionFooterItem['tone']>, string> = {
  default: 'text-ink-secondary',
  // Fine is the ordinary case and recedes, as a command that succeeded does.
  ok: 'text-ink-faint',
  danger: 'text-danger'
}

/**
 * When this was computed.
 *
 * The clock time rather than an age. A band is redrawn when its reading moves
 * and not otherwise, so "12s ago" would freeze at whatever it said when the
 * value last changed and quietly claim a stalled footer was current — the one
 * thing this line exists to disprove.
 */
function ComputedAt({ at }: { at: string }): ReactNode {
  const when = new Date(at)
  if (Number.isNaN(when.getTime())) return null
  return (
    <span className="text-[10px] text-ink-ghost tabular-nums shrink-0">
      {when.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })}
    </span>
  )
}
