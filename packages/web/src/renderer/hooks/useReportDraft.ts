import { useEffect, useRef } from 'react'

/**
 * Tell the host about a draft each time it changes.
 *
 * The host hands a fresh callback each render; reporting only when the draft
 * changes, through whichever callback came last, keeps its state update from
 * rendering the editor again, forever.
 */
export function useReportDraft<T>(
  report: ((draft: T) => void) | undefined,
  draft: T,
  changed: readonly unknown[]
): void {
  const latest = useRef(report)
  useEffect(() => {
    latest.current = report
  })
  useEffect(() => {
    latest.current?.(draft)
    // The draft is rebuilt each render; what it holds is what `changed` lists.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, changed)
}
