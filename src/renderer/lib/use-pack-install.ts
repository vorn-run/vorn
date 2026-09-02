import { useCallback, useEffect, useState } from 'react'
import type {
  ConnectorInstallProgress,
  ConnectorPackSource,
  ConnectorPackSummary
} from '../../shared/types'
import type { ConnectorListing } from './connector-browse'

/**
 * Installing a pack: inspect first, show what it is, keep it only on confirm.
 *
 * Settings owned all of this inline, which was fine while the directory was the
 * only place a connector could be installed from. A template that names a
 * connector it needs wants the same three steps, and two copies of a
 * verify-then-commit flow is one too many.
 */
export interface PackInstall {
  /** Live install and rejection state, keyed by connector id. */
  progress: Record<string, ConnectorInstallProgress>
  /** The pack awaiting a yes, with everything the sheet shows. */
  pending: { source: ConnectorPackSource; preview: ConnectorPackSummary } | null
  /** A refusal with no row to land on, such as a dropped file's. */
  error: string | null
  installing: boolean
  /** Inspect what a catalog row would install, then ask. */
  inspect: (listing: ConnectorListing, source?: ConnectorPackSource) => Promise<void>
  /** Inspect a pack already on this disk, then ask. */
  inspectFile: (filePath: string) => Promise<void>
  /** Install the files the sheet described. Resolves once they are on disk. */
  confirm: () => Promise<void>
  cancel: () => void
  clearError: () => void
  /** Say something where a refusal appears, such as what a removal cost. */
  report: (message: string) => void
}

/** Where a listing's pack comes from when the caller does not name a source. */
function sourceFor(listing: ConnectorListing): ConnectorPackSource {
  if (listing.catalogItem?.packUrl) {
    return {
      kind: 'url',
      url: listing.catalogItem.packUrl,
      ...(listing.catalogItem.sha256 && { sha256: listing.catalogItem.sha256 })
    } as ConnectorPackSource
  }
  return { kind: 'npm', packageName: listing.catalogItem?.packageName ?? listing.id } as const
}

export function usePackInstall(onInstalled?: () => void | Promise<void>): PackInstall {
  // Rejections live only here: nothing was written to disk, so they clear on reload.
  const [progress, setProgress] = useState<Record<string, ConnectorInstallProgress>>({})
  const [error, setError] = useState<string | null>(null)
  const [pending, setPending] = useState<PackInstall['pending']>(null)
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

  const inspect = useCallback(
    async (listing: ConnectorListing, source?: ConnectorPackSource) => {
      // Whatever the last attempt said is about that attempt, not this one.
      setError(null)
      setPending(null)
      forget(listing.id)
      const result = await window.api.inspectConnectorPack(source ?? sourceFor(listing))
      if (!result.ok) {
        // Keyed by the row that asked, which is the row that shows the refusal.
        setProgress((current) => ({
          ...current,
          [listing.id]: { id: listing.id, phase: 'failed', error: result.error }
        }))
        return
      }
      setPending({
        source: { kind: 'staged', token: result.preview.token },
        preview: result.preview
      })
    },
    [forget]
  )

  const inspectFile = useCallback(async (filePath: string) => {
    setError(null)
    setPending(null)
    const result = await window.api.inspectConnectorPack({ kind: 'file', path: filePath })
    if (!result.ok) {
      setError(result.error)
      return
    }
    setPending({
      source: { kind: 'staged', token: result.preview.token },
      preview: result.preview
    })
  }, [])

  const confirm = useCallback(async () => {
    if (!pending) return
    const id = pending.preview.id
    setInstalling(true)
    try {
      const result = await window.api.installConnectorPack(pending.source)
      if (result.ok) {
        forget(id)
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
    await onInstalled?.()
  }, [pending, forget, onInstalled])

  const cancel = useCallback(() => setPending(null), [])
  const clearError = useCallback(() => setError(null), [])
  const report = useCallback((message: string) => setError(message), [])

  return {
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
  }
}
