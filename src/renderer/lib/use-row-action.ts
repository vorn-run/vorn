import { useCallback, useMemo, useRef, useState } from 'react'

// What a row's own actions report while they run: a phrase while busy, and a refusal that lands where the press was.
export type RowAction = 'backfill' | 'delete' | 'run' | 'rollback' | 'remove'

/** Present tense, so a row says what is happening rather than what was pressed. */
const PHRASES: Record<RowAction, string> = {
  backfill: 'Importing…',
  delete: 'Removing…',
  run: 'Polling…',
  rollback: 'Rolling back…',
  remove: 'Removing…'
}

/** A call that answers with a refusal rather than throwing one. */
type Refusal = { ok: boolean; error?: string }

/** What a row shows for the actions it owns: one phrase, or one reason it failed. */
export interface RowState {
  phrase?: string
  error?: string
}

export interface RowActivity {
  /** Present tense, keyed by action and row. */
  busy: Record<string, string>
  /** Why the last attempt on that row failed, until the next one starts. */
  failed: Record<string, string>
  run: (action: RowAction, id: string, work: () => Promise<void | Refusal>) => Promise<void>
  /** Whichever of a row's actions is working, else whichever of them last failed. */
  state: (id: string, actions: RowAction[]) => RowState
}

function keyFor(action: RowAction, id: string): string {
  return `${action}:${id}`
}

/** The fold a row does to show one line, kept here so every surface folds alike. */
export function rowState(
  busy: Record<string, string>,
  failed: Record<string, string>,
  id: string,
  actions: RowAction[]
): RowState {
  const phrase = actions.map((action) => busy[keyFor(action, id)]).find(Boolean)
  if (phrase) return { phrase }
  const error = actions.map((action) => failed[keyFor(action, id)]).find(Boolean)
  return error ? { error } : {}
}

export function useRowAction(): RowActivity {
  const [busy, setBusy] = useState<Record<string, string>>({})
  const [failed, setFailed] = useState<Record<string, string>>({})

  // Keys in flight, so a second press before the first render disables the button is dropped.
  const inFlight = useRef(new Set<string>())
  const run = useCallback(
    async (action: RowAction, id: string, work: () => Promise<void | Refusal>) => {
      const key = keyFor(action, id)
      if (inFlight.current.has(key)) return
      inFlight.current.add(key)
      setBusy((current) => ({ ...current, [key]: PHRASES[action] }))
      // Whatever the last attempt said was about that attempt, not this one.
      setFailed((current) => {
        if (!(key in current)) return current
        const { [key]: _cleared, ...rest } = current
        return rest
      })
      try {
        const result = await work()
        if (result && result.ok === false) {
          setFailed((current) => ({ ...current, [key]: result.error || 'It did not say why.' }))
        }
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        setFailed((current) => ({ ...current, [key]: message }))
      } finally {
        inFlight.current.delete(key)
        setBusy((current) => {
          const { [key]: _done, ...rest } = current
          return rest
        })
      }
    },
    []
  )

  const state = useCallback(
    (id: string, actions: RowAction[]) => rowState(busy, failed, id, actions),
    [busy, failed]
  )

  return useMemo(() => ({ busy, failed, run, state }), [busy, failed, run, state])
}
