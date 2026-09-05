import { useCallback, useEffect, useMemo, useState } from 'react'
import type {
  ConnectorInstallProgress,
  ConnectorPackSource,
  ConnectorPackSummary
} from '../../shared/types'
import { matchesListing, type ConnectorListing } from './connector-browse'

/**
 * Installing a pack: inspect first, show what it is, keep it only on confirm.
 *
 * Settings owned all of this inline, which was fine while the directory was the
 * only place a connector could be installed from. A template that names a
 * connector it needs wants the same three steps, and two copies of a
 * verify-then-commit flow is one too many.
 */
/** A checked pack, held between the two steps of an install. */
export interface PendingPack {
  source: ConnectorPackSource
  preview: ConnectorPackSummary
  /** The row this began on, so both steps report their refusals to it. */
  rowId: string
  /** The listing that was pressed, so the sheet opens under it; a dropped file has none. */
  rowKey?: string
}

export interface PackInstall {
  /** Live install and rejection state, keyed by connector id. */
  progress: Record<string, ConnectorInstallProgress>
  /** The pack waiting to be asked about: a file, a replacement, or one unlike its listing. */
  pending: PendingPack | null
  /** A refusal with no row to land on, such as a dropped file's. */
  error: string | null
  installing: boolean
  /** Inspect what a catalog row would install; `direct` lets a surface that showed the pack's facts skip the sheet. */
  inspect: (listing: ConnectorListing, options?: { direct?: boolean }) => Promise<void>
  /** Inspect a pack already on this disk, then ask. */
  inspectFile: (filePath: string) => Promise<void>
  /** Install the files the sheet described. Resolves once they are on disk. */
  confirm: () => Promise<void>
  cancel: () => void
  clearError: () => void
  /** Say something where a refusal appears, such as what a removal cost. */
  report: (message: string) => void
}

/** Where a listing's pack comes from, or nothing when no release published one. */
function sourceFor(listing: ConnectorListing): ConnectorPackSource | undefined {
  const entry = listing.catalogItem
  if (!entry?.packUrl) return undefined
  return { kind: 'url', url: entry.packUrl, ...(entry.sha256 && { sha256: entry.sha256 }) }
}

/** Check a source, answering a refusal rather than throwing one. */
async function stage(source: ConnectorPackSource) {
  return window.api
    .inspectConnectorPack(source)
    .catch((e: unknown) => ({ ok: false as const, error: describeFailure(e) }))
}

/** What was checked, addressed to the row that asked for it. */
function staged(preview: ConnectorPackSummary, rowId: string, rowKey?: string): PendingPack {
  return {
    source: { kind: 'staged', token: preview.token },
    preview,
    rowId,
    ...(rowKey !== undefined && { rowKey })
  }
}

export function usePackInstall(onInstalled?: () => void | Promise<void>): PackInstall {
  // Rejections live only here: nothing was written to disk, so they clear on reload.
  const [progress, setProgress] = useState<Record<string, ConnectorInstallProgress>>({})
  const [error, setError] = useState<string | null>(null)
  const [pending, setPending] = useState<PendingPack | null>(null)
  const [installing, setInstalling] = useState(false)

  // The unsubscribe is what keeps a reopened panel from stacking a second listener.
  useEffect(() => {
    return window.api.onConnectorInstallProgress?.((update) => {
      setProgress((current) => ({ ...current, [update.id]: update }))
    })
  }, [])

  const forget = useCallback((id: string) => {
    setProgress((current) => {
      const next = { ...current }
      delete next[id]
      return next
    })
  }, [])

  // The second step, whether a sheet asked or the pack matched what was pressed.
  const keep = useCallback(
    async (pack: PendingPack) => {
      // The row that asked, so a refusal at either step lands in the same place.
      const id = pack.rowId
      let installed = false
      setInstalling(true)
      try {
        const result = await window.api.installConnectorPack(pack.source)
        if (result.ok) {
          // Still installing to the row until the reload shows the pack; the server's last word came too early.
          setProgress((current) => ({ ...current, [id]: { id, phase: 'installing' } }))
          installed = true
        } else {
          setProgress((current) => ({
            ...current,
            [id]: { id, phase: 'failed', error: result.error }
          }))
          setError(result.error)
        }
      } catch (err) {
        // A transport failure lands where a refusal lands, or the sheet never closes.
        const message = err instanceof Error ? err.message : 'The pack could not be installed'
        setProgress((current) => ({ ...current, [id]: { id, phase: 'failed', error: message } }))
        setError(message)
      } finally {
        setInstalling(false)
        setPending(null)
      }
      if (!installed) return
      try {
        await onInstalled?.()
      } catch (err) {
        // Kept on disk, but the list could not be re-read; say so rather than spin forever.
        setError(
          `Installed, but the list could not be re-read: ${err instanceof Error ? err.message : String(err)}`
        )
      } finally {
        forget(id)
      }
    },
    [forget, onInstalled]
  )

  const inspect = useCallback(
    async (listing: ConnectorListing, options?: { direct?: boolean }) => {
      // Whatever the last attempt said is about that attempt, not this one.
      setError(null)
      setPending(null)
      // Busy from the press itself, so the button cannot be pressed twice while the server is still silent.
      const source = sourceFor(listing)
      // Nothing to fetch until a release publishes one; whatever an earlier press left on the row goes too.
      if (!source) {
        forget(listing.id)
        return
      }
      setProgress((current) => ({
        ...current,
        [listing.id]: { id: listing.id, phase: 'checking' }
      }))
      const result = await stage(source)
      if (!result.ok) {
        // Keyed by the row that asked, which is the row that shows the refusal.
        setProgress((current) => ({
          ...current,
          [listing.id]: { id: listing.id, phase: 'failed', error: result.error }
        }))
        return
      }
      const pack = staged(result.preview, listing.id, listing.key)
      // Only a surface that showed the pack's facts asks for this; elsewhere the sheet is the only disclosure.
      if (options?.direct && matchesListing(result.preview, listing)) {
        await keep(pack)
        return
      }
      // The sheet takes over; the row is not busy again until a decision is made.
      forget(listing.id)
      setPending(pack)
    },
    [forget, keep]
  )

  const inspectFile = useCallback(async (filePath: string) => {
    setError(null)
    setPending(null)
    const result = await stage({ kind: 'file', path: filePath })
    if (!result.ok) {
      setError(result.error)
      return
    }
    // A dropped file has no row; the pack names itself once it is read.
    setPending(staged(result.preview, result.preview.id))
  }, [])

  const confirm = useCallback(async () => {
    if (pending) await keep(pending)
  }, [pending, keep])

  const cancel = useCallback(() => setPending(null), [])
  const clearError = useCallback(() => setError(null), [])
  const report = useCallback((message: string) => setError(message), [])

  // One object per change of what it holds, so a consumer can depend on it
  // without re-running an effect on every render of its owner.
  return useMemo(
    () => ({
      progress,
      pending,
      error,
      installing,
      inspect,
      inspectFile,
      confirm,
      cancel,
      clearError,
      report
    }),
    [
      progress,
      pending,
      error,
      installing,
      inspect,
      inspectFile,
      confirm,
      cancel,
      clearError,
      report
    ]
  )
}

/** A rejected call, worded for the row or sheet that asked. */
function describeFailure(error: unknown): string {
  return error instanceof Error ? error.message : 'The pack could not be checked'
}
