import { useCallback, useState } from 'react'

// What a row's own actions report while they run: a phrase while busy, and a refusal that lands where the press was.
export interface RowActivity {
  /** Present tense, keyed by row, e.g. `Removing…`. */
  busy: Record<string, string>
  /** Why the last attempt on that row failed, until the next one starts. */
  failed: Record<string, string>
  run: (key: string, phrase: string, work: () => Promise<unknown>) => Promise<void>
}

/** One spelling of a row's key, so a component and its caller cannot disagree. */
export function rowKey(action: string, id: string): string {
  return `${action}:${id}`
}

/** A call that answered with a refusal rather than throwing one. */
function refusalIn(result: unknown): string | undefined {
  if (typeof result !== 'object' || result === null || !('ok' in result)) return undefined
  if ((result as { ok: unknown }).ok !== false) return undefined
  const error = (result as { error?: unknown }).error
  return typeof error === 'string' && error !== '' ? error : 'It did not say why.'
}

export function useRowAction(): RowActivity {
  const [busy, setBusy] = useState<Record<string, string>>({})
  const [failed, setFailed] = useState<Record<string, string>>({})

  const forget = useCallback((key: string) => {
    setFailed((current) => {
      if (!(key in current)) return current
      const { [key]: _cleared, ...rest } = current
      return rest
    })
  }, [])

  const run = useCallback(
    async (key: string, phrase: string, work: () => Promise<unknown>) => {
      setBusy((current) => ({ ...current, [key]: phrase }))
      // Whatever the last attempt said was about that attempt, not this one.
      forget(key)
      try {
        const refusal = refusalIn(await work())
        if (refusal) setFailed((current) => ({ ...current, [key]: refusal }))
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        setFailed((current) => ({ ...current, [key]: message }))
      } finally {
        setBusy((current) => {
          const { [key]: _done, ...rest } = current
          return rest
        })
      }
    },
    [forget]
  )

  return { busy, failed, run }
}
