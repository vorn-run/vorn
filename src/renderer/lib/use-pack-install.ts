import { useCallback, useEffect, useMemo, useState } from 'react'
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
  /** Inspect what a catalog row would install; install it, or ask when it differs. */
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

/**
 * Whether the checked pack is the one the row already described.
 *
 * The sheet exists to show what a listing could not: a file nobody has seen, a
 * pack that replaces an installed version, or one whose id, version or sign-in
 * differs from what was advertised. When all of that agrees, the row said it
 * already and asking again only adds a click. A catalog with no version cannot
 * agree, so it is asked about.
 */
export function matchesListing(preview: ConnectorPackSummary, listing: ConnectorListing): boolean {
  return (
    preview.id === listing.id &&
    preview.version === listing.catalogItem?.version &&
    (preview.auth?.rung ?? 'none') === (listing.authRung ?? 'none') &&
    preview.installedVersion === undefined
  )
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
          forget(id)
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
      // Only when something was actually kept: a refusal changed nothing to read.
      if (installed) await onInstalled?.()
    },
    [forget, onInstalled]
  )

  const inspect = useCallback(
    async (listing: ConnectorListing, source?: ConnectorPackSource) => {
      // Whatever the last attempt said is about that attempt, not this one.
      setError(null)
      setPending(null)
      forget(listing.id)
      const result = await window.api
        .inspectConnectorPack(source ?? sourceFor(listing))
        .catch((e: unknown) => ({ ok: false as const, error: describeFailure(e) }))
      if (!result.ok) {
        // Keyed by the row that asked, which is the row that shows the refusal.
        setProgress((current) => ({
          ...current,
          [listing.id]: { id: listing.id, phase: 'failed', error: result.error }
        }))
        return
      }
      const pack: PendingPack = {
        source: { kind: 'staged', token: result.preview.token },
        preview: result.preview,
        rowId: listing.id,
        rowKey: listing.key
      }
      // Pressing install on a row that already said all this is the answer.
      if (matchesListing(result.preview, listing)) {
        await keep(pack)
        return
      }
      setPending(pack)
    },
    [forget, keep]
  )

  const inspectFile = useCallback(async (filePath: string) => {
    setError(null)
    setPending(null)
    const result = await window.api
      .inspectConnectorPack({ kind: 'file', path: filePath })
      .catch((e: unknown) => ({ ok: false as const, error: describeFailure(e) }))
    if (!result.ok) {
      setError(result.error)
      return
    }
    // A dropped file has no row; the pack names itself once it is read.
    setPending({
      source: { kind: 'staged', token: result.preview.token },
      preview: result.preview,
      rowId: result.preview.id
    })
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
