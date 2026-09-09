import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useAppStore } from '../../stores'
import { Tooltip } from '../Tooltip'
import type { ExtensionFooterItem, ExtensionFooterReading } from '../../../shared/types'

/** Stable empty, so a card with no extensions does not invalidate this memo. */
const NO_READINGS: ExtensionFooterReading[] = []

/** Full items, one chip per footer, or one chip per footer with the last few dropped. */
interface Fit {
  collapsed: boolean
  hidden: number
}

const WIDEST: Fit = { collapsed: false, hidden: 0 }

/**
 * What an extension's footers say about this session, in the card's status bar.
 *
 * In the bar rather than under it: a reading is the same kind of fact as the
 * branch or the last command, and a band of its own costs every card 22px per
 * installed extension — a price the grid pays whether or not anyone is reading.
 *
 * The colour rule is the bar's. A reading that is fine recedes; only something
 * wrong takes colour, so a row of passing checks cannot drown the one that failed.
 */
export function ExtensionStatusItems({ terminalId }: { terminalId: string }): ReactNode {
  const readings = useAppStore((s) => s.extensionFooters.get(terminalId) ?? NO_READINGS)

  // Sorted here rather than at the source: readings arrive one footer at a time
  // from processes the host started in whatever order it got to them, and chips
  // that reorder themselves as their neighbours report are unreadable.
  const ordered = useMemo(
    () =>
      [...readings].sort(
        (a, b) =>
          a.extensionName.localeCompare(b.extensionName) || a.footerId.localeCompare(b.footerId)
      ),
    [readings]
  )

  const box = useRef<HTMLDivElement>(null)
  const [fit, setFit] = useState<Fit>(WIDEST)
  const [fitted, setFitted] = useState(ordered)

  // Widest again during the render that brings new readings, rather than in an
  // effect: an effect would batch with the step-down below into no change at
  // all, and the bar would keep whatever width it first measured.
  if (fitted !== ordered) {
    setFitted(ordered)
    setFit(WIDEST)
  }

  // The bar is watched, not this box: this one narrows as it gives things up,
  // which would be a size change of its own and a measurement that never settles.
  useEffect(() => {
    const bar = box.current?.parentElement
    if (!bar || typeof ResizeObserver === 'undefined') return
    let width = bar.clientWidth
    const observer = new ResizeObserver(() => {
      if (bar.clientWidth === width) return
      width = bar.clientWidth
      setFit(WIDEST)
    })
    observer.observe(bar)
    return () => observer.disconnect()
  }, [])

  // One step narrower per pass, measured after each: every footer collapses to a
  // single chip first, and only then are they dropped, so what is given up is the
  // least that can be. It ends because each step is strictly narrower than the last.
  useLayoutEffect(() => {
    const el = box.current
    if (!el || el.scrollWidth <= el.clientWidth) return
    setFit((current) => {
      if (!current.collapsed) return { collapsed: true, hidden: 0 }
      if (current.hidden < ordered.length) return { collapsed: true, hidden: current.hidden + 1 }
      return current
    })
  }, [fit, ordered])

  if (ordered.length === 0) return null

  const shown = fit.hidden > 0 ? ordered.slice(0, ordered.length - fit.hidden) : ordered

  return (
    <div
      ref={box}
      data-testid={`extension-items-${terminalId}`}
      className="flex items-center gap-2 min-w-0 overflow-hidden"
    >
      {shown.map((reading) =>
        fit.collapsed ? (
          <CollapsedFooter key={`${reading.extensionId}:${reading.footerId}`} reading={reading} />
        ) : reading.error ? (
          <FailedFooter key={`${reading.extensionId}:${reading.footerId}`} reading={reading} />
        ) : (
          reading.items.map((item, i) => (
            <Item
              key={`${reading.extensionId}:${reading.footerId}:${i}`}
              item={item}
              reading={reading}
            />
          ))
        )
      )}
    </div>
  )
}

function Item({
  item,
  reading
}: {
  item: ExtensionFooterItem
  reading: ExtensionFooterReading
}): ReactNode {
  const body = (
    <>
      <span className="text-ink-faint">{item.label}</span>
      <span className={`tabular-nums ${TONE[item.tone ?? 'default']}`}>{item.value}</span>
    </>
  )
  const href = item.href

  // Opened outside rather than framed: an item names something that lives on
  // the web -- a run, an issue -- and the host has already held it to http.
  return (
    <Tooltip label={source(reading)}>
      {href ? (
        <button
          type="button"
          data-chip
          onPointerDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation()
            window.api.openExternal?.(href)
          }}
          className="flex items-center gap-1 shrink-0 whitespace-nowrap text-[10px]
                     hover:underline underline-offset-2 decoration-white/25"
        >
          {body}
        </button>
      ) : (
        <span data-chip className="flex items-center gap-1 shrink-0 whitespace-nowrap text-[10px]">
          {body}
        </span>
      )}
    </Tooltip>
  )
}

function FailedFooter({ reading }: { reading: ExtensionFooterReading }): ReactNode {
  return (
    <Tooltip label={`${source(reading)} — ${reading.error}`}>
      <span data-chip className="shrink-0 whitespace-nowrap text-[10px] text-danger">
        {reading.title}
      </span>
    </Tooltip>
  )
}

/** A whole footer in one chip, for a bar with no room to say it item by item. */
function CollapsedFooter({ reading }: { reading: ExtensionFooterReading }): ReactNode {
  const spelled = reading.error
    ? reading.error
    : reading.items.map((item) => `${item.label} ${item.value}`).join(', ')
  const wrong = Boolean(reading.error) || reading.items.some((item) => item.tone === 'danger')
  return (
    <Tooltip label={`${source(reading)} — ${spelled}`}>
      <span
        data-chip
        className={`shrink-0 whitespace-nowrap text-[10px] ${wrong ? 'text-danger' : 'text-ink-secondary'}`}
      >
        {reading.title} · {reading.error ? 1 : reading.items.length}
      </span>
    </Tooltip>
  )
}

/**
 * Which footer of whose extension, and when it last said so.
 *
 * The clock time rather than an age: a chip is redrawn when its reading moves
 * and not otherwise, so "12s ago" would freeze at whatever it said when the
 * value last changed and quietly claim a stalled footer was current.
 */
function source(reading: ExtensionFooterReading): string {
  const head = `${reading.title} · ${reading.extensionName}`
  const when = new Date(reading.computedAt)
  if (Number.isNaN(when.getTime())) return head
  return `${head} · ${when.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })}`
}

const TONE: Record<NonNullable<ExtensionFooterItem['tone']>, string> = {
  default: 'text-ink-secondary',
  // Fine is the ordinary case and recedes, as a command that succeeded does.
  ok: 'text-ink-faint',
  danger: 'text-danger'
}
